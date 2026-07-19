import { memo, useCallback, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { ChevronDown, RotateCw } from 'lucide-react'
import { Virtualizer, type VirtualizerHandle } from 'virtua'
import type { AgentPlanState, ChatMessage, ConversationContextState } from './types'
import { MessageBubble } from './MessageBubble'
import { MessageGroup } from './MessageGroup'
import { MessageNavigator } from './ChatMessageNavigator'
import { CompactionDivider } from './CompactionDivider'
import { CompactionInProgress } from './CompactionInProgress'
import { CompactionSummaryPanel } from './CompactionSummaryPanel'
import { resolveCompactionBoundaries, resolvePendingCompactionAfterIndex, type CompactionBoundaryView } from './compactionBoundary'
import { isExecutableAgentPlanText } from './agentPlan'
import { foldMessageGroups } from './messageGroups'
import {
  activeMessageNavigatorNodeId,
  buildMessageNavigatorNodes,
  visibleMessageNavigatorNodeIds,
  type MessageNavigatorNode,
} from './messageNavigator'
import { useStreamCoarse, useStreamSnapshot } from './streamingStore'
import { getActiveGroup, useGroupsVersion } from './groupStreamingStore'
import { prefersReducedMotion } from './utils'
import type { Lang } from '../settings/i18n'

export interface AssistantStreamStats {
  messageId: string
  tokensPerSec: number
  reasoningDurationMs?: number | null
  reasoningDurationMsBySegmentId?: Record<string, number>
}

interface MessageListProps {
  conversationId?: string | null
  messages: ChatMessage[]
  agentPlanState?: AgentPlanState | null
  assistantStreamStatsByMessageId?: Record<string, AssistantStreamStats>
  onUpdateMessage?: (messageId: string, content: string) => Promise<void>
  onRegenerateMessage?: (messageId: string, newContent?: string) => Promise<void>
  onForkMessage?: (messageId: string) => Promise<void>
  onDeleteMessage?: (messageId: string) => Promise<void>
  onSaveMessageToNote?: (messageId: string) => Promise<boolean>
  onExecuteAgentPlan?: (messageId: string) => Promise<void> | void
  // 失败发送后线程末尾留下的孤儿用户消息：点「重试」用它的 id 重新生成。
  onRetryLastUser?: (messageId: string) => void
  // 多模型一问多答（任务 06-30）：多答组「选中条」映射 + 点选回调。
  groupSelections?: Record<string, string>
  onSetGroupSelection?: (groupId: string, messageId: string) => void
  contextState?: ConversationContextState | null
  compactionInProgress?: boolean
  animateCompactionBoundaryId?: string | null
  lang?: Lang
}

const LIST_EDGE_PADDING_PX = 16
const BOTTOM_LEAVE_THRESHOLD_PX = 32
const BOTTOM_REENTER_THRESHOLD_PX = 16

// 列表里每一项的统一形态。整条会话全量喂给虚拟列表（消息都在内存，virtua 只渲可见项），
// 屏外的气泡连同其 KaTeX host / Markdown / 图片 DOM 真正从 DOM 卸载。
type RenderItem =
  | { kind: 'spacer'; key: 'padding-top' | 'padding-bottom'; size: number }
  | { kind: 'message'; key: string; message: ChatMessage; sentModels?: GroupModelLabel[] }
  | { kind: 'group'; key: string; groupId: string; messages: ChatMessage[] }
  | { kind: 'live-group'; key: string; groupId: string }
  | { kind: 'streaming'; key: 'streaming-assistant'; message: ChatMessage; messageStreaming: boolean; reasoningStreaming: boolean }
  | { kind: 'thinking'; key: 'thinking' }
  | { kind: 'error'; key: 'error'; text: string; retryMessageId: string | null }
  | { kind: 'compaction-divider'; key: string; boundary: CompactionBoundaryView; animate: boolean }
  | { kind: 'compaction-summary'; key: string; boundary: CompactionBoundaryView }
  | { kind: 'compaction-progress'; key: string; afterIndex: number }

type RenderSlot =
  | { kind: 'history'; item: RenderItem }
  | { kind: 'dynamic' }
  | { kind: 'error' }
  | { kind: 'bottom' }

const BOTTOM_SPACER_ITEM: RenderItem = {
  kind: 'spacer',
  key: 'padding-bottom',
  size: LIST_EDGE_PADDING_PX,
}

// R8（多模型一问多答）：多答组的「本次所发模型」列表，渲染在该组对应 user 消息顶部。
type GroupModelLabel = { providerId: string | null; model: string | null }

function MessageListBase({
  conversationId,
  messages,
  agentPlanState = null,
  assistantStreamStatsByMessageId = {},
  onUpdateMessage,
  onRegenerateMessage,
  onForkMessage,
  onDeleteMessage,
  onSaveMessageToNote,
  onExecuteAgentPlan,
  onRetryLastUser,
  groupSelections = {},
  onSetGroupSelection,
  contextState = null,
  compactionInProgress = false,
  animateCompactionBoundaryId = null,
  lang = 'zh',
}: MessageListProps) {
  // 流式预览状态直接订阅 streamingStore——只有本组件随每帧内容重渲，Chat/侧栏/输入栏不动。
  const coarse = useStreamCoarse()
  const snapshot = useStreamSnapshot()
  // 多答组实时流：订阅 group store 版本号，活跃组列内容更新时驱动重渲。
  const groupsVersion = useGroupsVersion()
  const liveGroup = conversationId ? getActiveGroup(conversationId) : undefined
  // Group column objects are mutated in place for every stream delta. Only model
  // identity changes should rebuild historical rows; content deltas stay in the
  // dedicated live-group row.
  const liveGroupModelsKey = liveGroup
    ? `${liveGroup.groupId}\0${liveGroup.columns
      .map((column) => `${column.providerId ?? ''}:${column.model ?? ''}`)
      .join('\0')}`
    : ''
  const liveGroupId = liveGroup?.groupId ?? null
  const liveGroupColumns = liveGroup?.columns
  const liveGroupModels = useMemo(() => (
    liveGroupId && liveGroupColumns
      ? {
        key: liveGroupModelsKey,
        groupId: liveGroupId,
        labels: liveGroupColumns.map((column) => ({
          providerId: column.providerId,
          model: column.model,
        })),
      }
      : null
  ), [liveGroupColumns, liveGroupId, liveGroupModelsKey])
  const streaming = coarse.streaming
  const streamFrozen = coarse.streamFrozen
  const error = coarse.streamError
  const streamingContent = snapshot.content
  const streamingReasoning = snapshot.reasoning
  const streamingReasoningDurationMs = snapshot.reasoningDurationMs
  const streamingReasoningDurationMsBySegmentId = snapshot.reasoningDurationMsBySegmentId
  const reasoningStreaming = snapshot.reasoningStreaming
  const streamingToolCalls = snapshot.toolCalls
  const streamingSegments = snapshot.segments

  const scrollRef = useRef<HTMLDivElement>(null)
  const virtualizerRef = useRef<VirtualizerHandle>(null)
  // 用户是否“贴在底部”——决定流式生成时是否跟随钉底。默认 true（初次渲染贴底）
  const stickToBottomRef = useRef(true)
  const prevMessageCountRef = useRef(0)
  // 是否贴在底部——驱动「回到底部」按钮的显隐（ref 不触发渲染，故另用 state）
  const atBottomRef = useRef(true)
  const [atBottom, setAtBottom] = useState(true)
  const [activeNavigatorNodeId, setActiveNavigatorNodeId] = useState<string | null>(null)
  const [visibleNavigatorNodeIds, setVisibleNavigatorNodeIds] = useState<string[]>([])
  const lastScrollOffsetRef = useRef(0)
  const navigatorNodesRef = useRef<MessageNavigatorNode[]>([])
  const activeNavigatorNodeIdRef = useRef<string | null>(null)
  const visibleNavigatorNodeIdsRef = useRef<string[]>([])

  const updateAtBottom = useCallback((next: boolean) => {
    if (atBottomRef.current === next) return
    atBottomRef.current = next
    setAtBottom(next)
  }, [])

  const legacyPlanMessageId = useMemo(() => {
    const legacyPlan = agentPlanState?.plan?.trim()
    if (!isExecutableAgentPlanText(legacyPlan)) return null
    const hasMessagePlan = messages.some((message) => Boolean(
      isExecutableAgentPlanText((message.agent_plan ?? message.agentPlan)?.plan),
    ))
    if (hasMessagePlan) return null
    return [...messages]
      .reverse()
      .find((message) => message.role === 'assistant' && message.content.trim() === legacyPlan)
      ?.id ?? null
  }, [agentPlanState, messages])

  const messageIndexById = useMemo(() => {
    const map = new Map<string, number>()
    messages.forEach((message, index) => map.set(message.id, index))
    return map
  }, [messages])

  const boundaries = useMemo(
    () => resolveCompactionBoundaries(messages, contextState),
    [contextState, messages],
  )

  const boundariesByAfterIndex = useMemo(() => {
    const map = new Map<number, CompactionBoundaryView[]>()
    for (const boundary of boundaries) {
      const existing = map.get(boundary.afterIndex) ?? []
      existing.push(boundary)
      map.set(boundary.afterIndex, existing)
    }
    return map
  }, [boundaries])

  const folded = useMemo(() => foldMessageGroups(messages), [messages])

  const pendingCompactionAfterIndex = useMemo(
    () => (
      compactionInProgress
        ? resolvePendingCompactionAfterIndex(messages, contextState, animateCompactionBoundaryId)
        : null
    ),
    [animateCompactionBoundaryId, compactionInProgress, contextState, messages],
  )

  const appendCompactionItems = useCallback((
    list: RenderItem[],
    afterIndex: number,
  ) => {
    const boundaries = boundariesByAfterIndex.get(afterIndex)
    if (!boundaries) return
    for (const boundary of boundaries) {
      const recordId = boundary.record.id
      list.push({
        kind: 'compaction-divider',
        key: `compaction-divider-${recordId}`,
        boundary,
        animate: animateCompactionBoundaryId === recordId,
      })
      list.push({
        kind: 'compaction-summary',
        key: `compaction-summary-${recordId}`,
        boundary,
      })
    }
  }, [animateCompactionBoundaryId, boundariesByAfterIndex])

  const appendCompactionSlot = useCallback((
    list: RenderItem[],
    afterIndex: number,
  ) => {
    const hasBoundary = boundariesByAfterIndex.has(afterIndex)
    if (
      compactionInProgress
      && pendingCompactionAfterIndex === afterIndex
      && !hasBoundary
    ) {
      list.push({
        kind: 'compaction-progress',
        key: `compaction-progress-after-${afterIndex}`,
        afterIndex,
      })
      return
    }
    appendCompactionItems(list, afterIndex)
  }, [
    appendCompactionItems,
    boundariesByAfterIndex,
    compactionInProgress,
    pendingCompactionAfterIndex,
  ])

  // 历史项只在消息/压缩边界/组模型身份变化时重建。高频流式文本不进入依赖，
  // 避免长会话每帧遍历并重新分配整个历史数组。
  const historyItems = useMemo<RenderItem[]>(() => {
    const list: RenderItem[] = [
      { kind: 'spacer', key: 'padding-top', size: LIST_EDGE_PADDING_PX },
    ]

    // 多模型一问多答（任务 06-30）：把同一 group_id 的连续 assistant 消息折成一个 group item，
    // 横向并排多列；其余消息线性 push（折叠逻辑是纯函数 foldMessageGroups，便于单测）。
    // R8：先收集 group_id → 本次所发模型列表，给该组对应 user 消息加模型标签行。
    const sentModelsByGroup = new Map<string, GroupModelLabel[]>()
    for (const item of folded) {
      if (item.type === 'group') {
        sentModelsByGroup.set(
          item.groupId,
          item.messages.map((m) => ({
            providerId: m.provider_id ?? m.providerId ?? null,
            model: m.model ?? null,
          })),
        )
      }
    }
    // 流式态下本组 assistant 尚未落库 → 从实时列补出模型列表，让 user 消息标签即时出现。
    if (
      liveGroupModels
      && liveGroupModels.labels.length > 0
      && !sentModelsByGroup.has(liveGroupModels.groupId)
    ) {
      sentModelsByGroup.set(
        liveGroupModels.groupId,
        liveGroupModels.labels,
      )
    }

    for (const item of folded) {
      if (item.type === 'group') {
        list.push({
          kind: 'group',
          key: `group-${item.groupId}`,
          groupId: item.groupId,
          messages: item.messages,
        })
        const boundaryIndices = new Set<number>()
        for (const message of item.messages) {
          const index = messageIndexById.get(message.id)
          if (index != null) boundaryIndices.add(index)
        }
        for (const index of boundaryIndices) {
          appendCompactionSlot(list, index)
        }
      } else {
        const message = item.message
        const groupId = message.role === 'user' ? (message.group_id ?? message.groupId ?? null) : null
        const sentModels = groupId ? sentModelsByGroup.get(groupId) : undefined
        list.push({ kind: 'message', key: message.id, message, sentModels })
        const index = messageIndexById.get(message.id)
        if (index != null) appendCompactionSlot(list, index)
      }
    }

    return list
  }, [folded, liveGroupModels, messageIndexById, appendCompactionSlot])

  // 高频变化只更新固定的尾部 slot，不重建历史项。
  const dynamicItem = useMemo<RenderItem | null>(() => {
    const hasLiveGroup = Boolean(liveGroup && (coarse.streaming || coarse.streamFrozen))
    const hasStreamingPreview =
      !hasLiveGroup &&
      (streaming || streamFrozen) &&
      (streamingContent || streamingReasoning || streamingToolCalls.length > 0 || streamingSegments.length > 0)
    if (hasLiveGroup && liveGroup) {
      return { kind: 'live-group', key: `live-group-${liveGroup.groupId}`, groupId: liveGroup.groupId }
    }
    if (hasStreamingPreview) {
      return {
        kind: 'streaming',
        key: 'streaming-assistant',
        messageStreaming: streaming && !streamFrozen,
        reasoningStreaming: reasoningStreaming && !streamFrozen,
        message: {
          id: 'streaming-assistant',
          role: 'assistant',
          content: streamingContent,
          reasoning: streamingReasoning || undefined,
          artifacts: [],
          tool_calls: streamingToolCalls,
          segments: streamingSegments,
          timestamp: Math.floor(Date.now() / 1000),
        },
      }
    }
    return streaming ? { kind: 'thinking', key: 'thinking' } : null
  }, [
    liveGroup,
    coarse.streaming,
    coarse.streamFrozen,
    streaming,
    streamFrozen,
    streamingContent,
    streamingReasoning,
    reasoningStreaming,
    streamingToolCalls,
    streamingSegments,
  ])

  const errorItem = useMemo<RenderItem | null>(() => {
    if (!error) return null
    const last = messages[messages.length - 1]
    const retryMessageId = last && last.role === 'user' ? last.id : null
    return { kind: 'error', key: 'error', text: error, retryMessageId }
  }, [error, messages])

  // Virtua's `data + render function` path avoids flattening an O(N) React
  // children tree every frame. Three fixed tail slot kinds resolve their current
  // payload in the render callback; the data array changes only with history.
  const renderSlots = useMemo<RenderSlot[]>(() => [
    ...historyItems.map((item): RenderSlot => ({ kind: 'history', item })),
    { kind: 'dynamic' },
    { kind: 'error' },
    { kind: 'bottom' },
  ], [historyItems])

  const navigatorNodes = useMemo(() => {
    const renderIndexByKey = new Map(historyItems.map((item, index) => [item.key, index]))
    return buildMessageNavigatorNodes({ folded, boundaries, renderIndexByKey })
  }, [boundaries, folded, historyItems])
  navigatorNodesRef.current = navigatorNodes
  const navigatorTurnCount = navigatorNodes.reduce(
    (count, node) => count + (node.kind === 'turn' ? 1 : 0),
    0,
  )

  const updateActiveNavigatorNode = useCallback((nodeId: string | null) => {
    if (activeNavigatorNodeIdRef.current === nodeId) return
    activeNavigatorNodeIdRef.current = nodeId
    setActiveNavigatorNodeId(nodeId)
  }, [])

  const updateVisibleNavigatorNodes = useCallback((nodeIds: string[]) => {
    const previous = visibleNavigatorNodeIdsRef.current
    if (previous.length === nodeIds.length && previous.every((id, index) => id === nodeIds[index])) return
    visibleNavigatorNodeIdsRef.current = nodeIds
    setVisibleNavigatorNodeIds(nodeIds)
  }, [])

  const navigateToNavigatorNode = useCallback((node: MessageNavigatorNode) => {
    const handle = virtualizerRef.current
    if (!handle) return
    stickToBottomRef.current = false
    updateAtBottom(false)
    updateActiveNavigatorNode(node.id)
    handle.scrollToIndex(node.targetRenderIndex, {
      align: 'start',
      smooth: !prefersReducedMotion(),
    })
  }, [updateActiveNavigatorNode, updateAtBottom])

  const handleNavigatorStep = useCallback((direction: -1 | 1) => {
    const nodes = navigatorNodesRef.current
    if (nodes.length === 0) return
    const currentId = activeNavigatorNodeIdRef.current
    const currentIndex = Math.max(0, nodes.findIndex((node) => node.id === currentId))
    const nextIndex = Math.min(nodes.length - 1, Math.max(0, currentIndex + direction))
    navigateToNavigatorNode(nodes[nextIndex])
  }, [navigateToNavigatorNode])

  const scrollToBottom = useCallback((smooth = false) => {
    const index = renderSlots.length - 1
    if (index < 0) return
    const handle = virtualizerRef.current
    if (handle) {
      handle.scrollToIndex(index, {
        align: 'end',
        smooth: smooth && !prefersReducedMotion(),
      })
      lastScrollOffsetRef.current = handle.scrollOffset
      return
    }

    const el = scrollRef.current
    if (!el) return
    if (smooth && !prefersReducedMotion()) { el.scrollTo({ top: el.scrollHeight, behavior: 'smooth' }); return }
    el.scrollTop = el.scrollHeight
    lastScrollOffsetRef.current = el.scrollTop
  }, [renderSlots.length])

  const scrollFrameRef = useRef<number | null>(null)
  const pendingSmoothScrollRef = useRef(false)
  const scheduleScrollToBottom = useCallback((smooth = false) => {
    pendingSmoothScrollRef.current ||= smooth
    if (scrollFrameRef.current != null) return
    scrollFrameRef.current = requestAnimationFrame(() => {
      scrollFrameRef.current = null
      const shouldSmooth = pendingSmoothScrollRef.current
      pendingSmoothScrollRef.current = false
      if (stickToBottomRef.current) scrollToBottom(shouldSmooth)
    })
  }, [scrollToBottom])

  useLayoutEffect(() => () => {
    if (scrollFrameRef.current != null) cancelAnimationFrame(scrollFrameRef.current)
  }, [])

  const handleJumpToBottom = useCallback(() => {
    stickToBottomRef.current = true
    updateAtBottom(true)
    scheduleScrollToBottom(true)
  }, [scheduleScrollToBottom, updateAtBottom])

  // 滚轮向上 = 明确的离开底部意图，立即解除跟随。按钮显隐仍只由实际滚动几何决定，
  // 避免在底部阈值内 wheel 先显示、scroll 又隐藏，造成一帧闪烁和重复重渲。
  const handleWheel = (e: React.WheelEvent) => {
    if (e.deltaY < 0) {
      stickToBottomRef.current = false
    }
  }

  // 滚动监听：用 virtua 的 scroll geometry 判断贴底/离开底部。
  const handleScroll = useCallback((nextOffset: number) => {
    const el = scrollRef.current
    const handle = virtualizerRef.current
    const offset = handle?.scrollOffset ?? nextOffset
    const scrollSize = handle?.scrollSize ?? el?.scrollHeight ?? 0
    const viewportSize = handle?.viewportSize ?? el?.clientHeight ?? 0
    const bottomDistance = scrollSize - offset - viewportSize
    // 离开底部仍沿用 32px；重新进入用更小的 16px，防止 virtua 测量抖动时
    // 在同一个临界值附近反复挂载/卸载按钮。
    const bottom = atBottomRef.current
      ? bottomDistance <= BOTTOM_LEAVE_THRESHOLD_PX
      : bottomDistance <= BOTTOM_REENTER_THRESHOLD_PX
    if (offset < lastScrollOffsetRef.current - 1) {
      stickToBottomRef.current = false
    } else if (bottom) {
      stickToBottomRef.current = true
    }
    lastScrollOffsetRef.current = offset
    updateAtBottom(bottom)

    if (handle) {
      const readingOffset = Math.min(
        Math.max(0, scrollSize - 1),
        offset + viewportSize * 0.3,
      )
      const renderIndex = handle.findItemIndex(readingOffset)
      updateActiveNavigatorNode(activeMessageNavigatorNodeId(navigatorNodesRef.current, renderIndex))
      const firstVisibleIndex = handle.findItemIndex(offset)
      const lastVisibleOffset = Math.min(
        Math.max(0, scrollSize - 1),
        Math.max(offset, offset + viewportSize - 1),
      )
      const lastVisibleIndex = handle.findItemIndex(lastVisibleOffset)
      updateVisibleNavigatorNodes(visibleMessageNavigatorNodeIds(
        navigatorNodesRef.current,
        firstVisibleIndex,
        lastVisibleIndex,
      ))
    }
  }, [updateActiveNavigatorNode, updateAtBottom, updateVisibleNavigatorNodes])

  // 切换会话：重置跟随并瞬间定位到底部
  useLayoutEffect(() => {
    stickToBottomRef.current = true
    updateAtBottom(true)
    const lastNode = navigatorNodesRef.current[navigatorNodesRef.current.length - 1]
    updateActiveNavigatorNode(lastNode?.id ?? null)
    updateVisibleNavigatorNodes(lastNode ? [lastNode.id] : [])
    // 等虚拟列表用最新 items 渲染后再对齐底部
    scheduleScrollToBottom()
    // 仅在 conversationId 变化时重置；scheduleScrollToBottom 依赖项数，故不列入依赖避免误触发
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [conversationId, updateActiveNavigatorNode, updateAtBottom, updateVisibleNavigatorNodes])

  // 自己发出新消息时强制回到底部（即使刚才正往上翻历史）
  useLayoutEffect(() => {
    const count = messages.length
    if (count > prevMessageCountRef.current && messages[count - 1]?.role === 'user') {
      stickToBottomRef.current = true
      updateAtBottom(true)
    }
    prevMessageCountRef.current = count
  }, [messages, updateAtBottom])

  // 仅在“贴底”时随内容增长钉住底部。virtua 内置 ResizeObserver 会在变高（KaTeX/图片
  // mount 后撑高）时重测，这里在每次内容/项数变化后重新对齐末尾，保证持续钉底。
  useLayoutEffect(() => {
    if (!stickToBottomRef.current) return
    scheduleScrollToBottom()
  }, [
    renderSlots,
    dynamicItem,
    errorItem,
    groupsVersion,
    scheduleScrollToBottom,
  ])

  const renderItem = useCallback(
    (item: RenderItem) => {
      switch (item.kind) {
        case 'spacer':
          return <div aria-hidden="true" style={{ height: item.size }} />
        case 'message': {
          const msg = item.message
          const assistantStats = msg.role === 'assistant'
            ? assistantStreamStatsByMessageId[msg.id]
            : undefined
          return (
            <MessageBubble
              message={msg}
              conversationId={conversationId}
              tokensPerSec={assistantStats?.tokensPerSec}
              reasoningDurationMs={assistantStats?.reasoningDurationMs}
              reasoningDurationMsBySegmentId={assistantStats?.reasoningDurationMsBySegmentId}
              sentModels={item.sentModels}
              onUpdateMessage={msg.role === 'assistant' ? onUpdateMessage : undefined}
              // 编辑/重生成入口在任何 run 在飞时都不可用（AC3）。streamFrozen 也算在飞：
              // 本地取消后 send invoke 尚未返回，此窗口内触发只会被 in-flight 兜底静默吞掉
              // （编辑文本会被无声丢弃），所以从入口处直接收起。
              onRegenerateMessage={streaming || streamFrozen ? undefined : onRegenerateMessage}
              onForkMessage={streaming || streamFrozen ? undefined : onForkMessage}
              onDeleteMessage={onDeleteMessage}
              onSaveMessageToNote={onSaveMessageToNote}
              agentPlanOverride={msg.id === legacyPlanMessageId ? agentPlanState : null}
              onExecuteAgentPlan={msg.role === 'assistant' ? onExecuteAgentPlan : undefined}
            />
          )
        }
        case 'group': {
          const selectedMessageId = groupSelections[item.groupId] ?? null
          return (
            <MessageGroup
              conversationId={conversationId}
              groupId={item.groupId}
              messages={item.messages}
              selectedMessageId={selectedMessageId}
              onSelectColumn={onSetGroupSelection}
              onUpdateMessage={onUpdateMessage}
              onRegenerateMessage={streaming || streamFrozen ? undefined : onRegenerateMessage}
              onForkMessage={streaming || streamFrozen ? undefined : onForkMessage}
              onDeleteMessage={onDeleteMessage}
              onSaveMessageToNote={onSaveMessageToNote}
            />
          )
        }
        case 'live-group':
          return (
            <MessageGroup
              conversationId={conversationId}
              groupId={item.groupId}
              messages={[]}
              onSaveMessageToNote={onSaveMessageToNote}
            />
          )
        case 'streaming':
          return (
            <MessageBubble
              message={item.message}
              conversationId={conversationId}
              messageStreaming={item.messageStreaming}
              reasoningStreaming={item.reasoningStreaming}
              reasoningDurationMs={streamingReasoningDurationMs}
              reasoningDurationMsBySegmentId={streamingReasoningDurationMsBySegmentId}
            />
          )
        case 'thinking':
          return (
            <div className="chat-motion-fade-up flex justify-start py-3">
              <span className="reasoning-shimmer-text text-sm font-medium">正在思考…</span>
            </div>
          )
        case 'compaction-divider':
          return (
            <CompactionDivider
              boundary={item.boundary}
              lang={lang}
              animate={item.animate}
            />
          )
        case 'compaction-summary':
          return (
            <CompactionSummaryPanel
              boundary={item.boundary}
              lang={lang}
            />
          )
        case 'compaction-progress':
          return <CompactionInProgress lang={lang} />
        case 'error':
          return (
            <div className="chat-motion-fade-up flex flex-col items-start gap-2 py-3">
              <p className="max-w-[85%] text-sm leading-relaxed text-red-600 dark:text-red-400">
                {item.text}
              </p>
              {item.retryMessageId && onRetryLastUser && (
                <button
                  type="button"
                  onClick={() => onRetryLastUser(item.retryMessageId!)}
                  className="inline-flex items-center gap-1 rounded-full border border-neutral-200 bg-white px-3 py-1 text-xs font-medium text-neutral-700 transition-colors hover:bg-neutral-50 active:scale-95 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-200 dark:hover:bg-neutral-700"
                >
                  <RotateCw size={13} strokeWidth={2} />
                  重试
                </button>
              )}
            </div>
          )
      }
    },
    [
      conversationId,
      assistantStreamStatsByMessageId,
      agentPlanState,
      legacyPlanMessageId,
      onUpdateMessage,
      onRegenerateMessage,
      onForkMessage,
      onDeleteMessage,
      onSaveMessageToNote,
      onExecuteAgentPlan,
      onRetryLastUser,
      streaming,
      streamFrozen,
      groupSelections,
      onSetGroupSelection,
      streamingReasoningDurationMs,
      streamingReasoningDurationMsBySegmentId,
      lang,
    ],
  )

  const renderSlot = useCallback((slot: RenderSlot) => {
    const item = slot.kind === 'history'
      ? slot.item
      : slot.kind === 'dynamic'
        ? dynamicItem
        : slot.kind === 'error'
          ? errorItem
          : BOTTOM_SPACER_ITEM
    if (!item) return <div aria-hidden="true" />
    return (
      <div
        className={item.kind === 'spacer' ? undefined : 'pb-0.5'}
        data-chat-message-list-item={item.kind}
      >
        {renderItem(item)}
      </div>
    )
  }, [dynamicItem, errorItem, renderItem])

  return (
    <div className={`relative flex min-h-0 flex-1 flex-col ${navigatorTurnCount >= 4 ? 'has-message-navigator' : ''}`}>
      {navigatorTurnCount >= 4 && (
        <MessageNavigator
          nodes={navigatorNodes}
          activeNodeId={activeNavigatorNodeId}
          visibleNodeIds={visibleNavigatorNodeIds}
          onNavigate={navigateToNavigatorNode}
          onNavigateStep={handleNavigatorStep}
        />
      )}
      <div
        ref={scrollRef}
        onWheel={handleWheel}
        className="chat-motion-view-in custom-scrollbar flex-1 overflow-y-auto"
      >
        <div className="chat-message-list-inner mx-auto w-full max-w-4xl px-6">
          <Virtualizer
            ref={virtualizerRef}
            scrollRef={scrollRef}
            onScroll={handleScroll}
            data={renderSlots}
          >
            {renderSlot}
          </Virtualizer>
        </div>
      </div>
      {!atBottom && (
        <button
          type="button"
          onClick={handleJumpToBottom}
          aria-label="回到底部"
          title="回到底部"
          className="chat-motion-pop absolute bottom-4 left-1/2 z-10 flex h-9 w-9 -translate-x-1/2 items-center justify-center rounded-full border border-neutral-200 bg-white/95 text-neutral-600 shadow-md backdrop-blur transition-transform duration-[var(--kv-dur-instant)] ease-[var(--kv-ease-spring)] hover:text-neutral-900 active:scale-90 dark:border-neutral-700 dark:bg-neutral-900/95 dark:text-neutral-300 dark:hover:text-neutral-100"
        >
          <ChevronDown size={18} strokeWidth={2} />
        </button>
      )}
    </div>
  )
}

// memo：列表本身订阅 streamingStore，父级 Chat 重渲（非流式 state 变化）时不跟着白渲。
export const MessageList = memo(MessageListBase)
