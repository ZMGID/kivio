import { memo, useCallback, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { ChevronDown, RotateCw } from 'lucide-react'
import { Virtualizer, type VirtualizerHandle } from 'virtua'
import type { AgentPlanState, ChatMessage, ConversationContextState } from './types'
import { MessageBubble } from './MessageBubble'
import { MessageGroup } from './MessageGroup'
import { CompactionDivider } from './CompactionDivider'
import { CompactionInProgress } from './CompactionInProgress'
import { CompactionSummaryPanel } from './CompactionSummaryPanel'
import { resolveCompactionBoundaries, resolvePendingCompactionAfterIndex, type CompactionBoundaryView } from './compactionBoundary'
import { isExecutableAgentPlanText } from './agentPlan'
import { foldMessageGroups } from './messageGroups'
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
  onDeleteMessage?: (messageId: string) => Promise<void>
  onExecuteAgentPlan?: (messageId: string) => Promise<void> | void
  // 失敗傳送後執行緒末尾留下的孤兒使用者訊息：點「重試」用它的 id 重新生成。
  onRetryLastUser?: (messageId: string) => void
  // 多模型一問多答（任務 06-30）：多答組「選中條」對映 + 點選回撥。
  groupSelections?: Record<string, string>
  onSetGroupSelection?: (groupId: string, messageId: string) => void
  contextState?: ConversationContextState | null
  compactionInProgress?: boolean
  animateCompactionBoundaryId?: string | null
  lang?: Lang
}

const LIST_EDGE_PADDING_PX = 16

// 列表裡每一項的統一形態。整條會話全量餵給虛擬列表（訊息都在記憶體，virtua 只渲可見項），
// 屏外的氣泡連同其 KaTeX host / Markdown / 圖片 DOM 真正從 DOM 解除安裝。
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

// R8（多模型一問多答）：多答組的「本次所發模型」列表，渲染在該組對應 user 訊息頂部。
type GroupModelLabel = { providerId: string | null; model: string | null }

function MessageListBase({
  conversationId,
  messages,
  agentPlanState = null,
  assistantStreamStatsByMessageId = {},
  onUpdateMessage,
  onRegenerateMessage,
  onDeleteMessage,
  onExecuteAgentPlan,
  onRetryLastUser,
  groupSelections = {},
  onSetGroupSelection,
  contextState = null,
  compactionInProgress = false,
  animateCompactionBoundaryId = null,
  lang = 'zh',
}: MessageListProps) {
  // 流式預覽狀態直接訂閱 streamingStore——只有本元件隨每幀內容重渲，Chat/側欄/輸入欄不動。
  const coarse = useStreamCoarse()
  const snapshot = useStreamSnapshot()
  // 多答組即時流：訂閱 group store 版本號，活躍組列內容更新時驅動重渲。
  const groupsVersion = useGroupsVersion()
  const liveGroup = conversationId ? getActiveGroup(conversationId) : undefined
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
  // 使用者是否“貼在底部”——決定流式生成時是否跟隨釘底。預設 true（初次渲染貼底）
  const stickToBottomRef = useRef(true)
  const prevMessageCountRef = useRef(0)
  // 是否貼在底部——驅動「回到底部」按鈕的顯隱（ref 不觸發渲染，故另用 state）
  const [atBottom, setAtBottom] = useState(true)
  const lastScrollOffsetRef = useRef(0)

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

  const boundariesByAfterIndex = useMemo(() => {
    const map = new Map<number, CompactionBoundaryView[]>()
    for (const boundary of resolveCompactionBoundaries(messages, contextState)) {
      const existing = map.get(boundary.afterIndex) ?? []
      existing.push(boundary)
      map.set(boundary.afterIndex, existing)
    }
    return map
  }, [contextState, messages])

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

  // 把訊息 + 流式預覽 + 佔位拼成統一的虛擬列表項陣列。
  const items = useMemo<RenderItem[]>(() => {
    const list: RenderItem[] = [
      { kind: 'spacer', key: 'padding-top', size: LIST_EDGE_PADDING_PX },
    ]

    // 多模型一問多答（任務 06-30）：把同一 group_id 的連續 assistant 訊息折成一個 group item，
    // 橫向並排多列；其餘訊息線性 push（摺疊邏輯是純函式 foldMessageGroups，便於單測）。
    // R8：先收集 group_id → 本次所發模型列表，給該組對應 user 訊息加模型標籤行。
    const folded = foldMessageGroups(messages)
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
    // 流式態下本組 assistant 尚未落庫 → 從即時列補出模型列表，讓 user 訊息標籤即時出現。
    if (liveGroup && liveGroup.columns.length > 0 && !sentModelsByGroup.has(liveGroup.groupId)) {
      sentModelsByGroup.set(
        liveGroup.groupId,
        liveGroup.columns.map((col) => ({ providerId: col.providerId, model: col.model })),
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

    // 即時多答組：流式中（active group 存在）追加一個 live-group item，取代單流預覽氣泡。
    const hasLiveGroup = Boolean(liveGroup && (coarse.streaming || coarse.streamFrozen))
    const hasStreamingPreview =
      !hasLiveGroup &&
      (streaming || streamFrozen) &&
      (streamingContent || streamingReasoning || streamingToolCalls.length > 0 || streamingSegments.length > 0)
    if (hasLiveGroup && liveGroup) {
      list.push({ kind: 'live-group', key: `live-group-${liveGroup.groupId}`, groupId: liveGroup.groupId })
    } else if (hasStreamingPreview) {
      list.push({
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
      })
    } else if (streaming) {
      list.push({ kind: 'thinking', key: 'thinking' })
    }

    if (error) {
      // 末尾是使用者訊息 = 失敗傳送遺留的孤兒，給它一個重試入口；其它錯誤不顯示重試。
      const last = messages[messages.length - 1]
      const retryMessageId = last && last.role === 'user' ? last.id : null
      list.push({ kind: 'error', key: 'error', text: error, retryMessageId })
    }
    list.push({ kind: 'spacer', key: 'padding-bottom', size: LIST_EDGE_PADDING_PX })
    return list
  }, [
    messages,
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
    error,
    appendCompactionSlot,
    messageIndexById,
  ])

  const scrollToBottom = useCallback((smooth = false) => {
    const index = items.length - 1
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
  }, [items.length])

  const handleJumpToBottom = useCallback(() => {
    stickToBottomRef.current = true
    setAtBottom(true)
    scrollToBottom(true)
  }, [scrollToBottom])

  // 滾輪向上 = 明確的離開底部意圖，立即解除跟隨（不設緩衝，消除“掙扎感”）
  const handleWheel = (e: React.WheelEvent) => {
    if (e.deltaY < 0) {
      stickToBottomRef.current = false
      setAtBottom(false)
    }
  }

  // 滾動監聽：用 virtua 的 scroll geometry 判斷貼底/離開底部。
  const handleScroll = useCallback((nextOffset: number) => {
    const el = scrollRef.current
    const handle = virtualizerRef.current
    const offset = handle?.scrollOffset ?? nextOffset
    const scrollSize = handle?.scrollSize ?? el?.scrollHeight ?? 0
    const viewportSize = handle?.viewportSize ?? el?.clientHeight ?? 0
    const bottom = scrollSize - offset - viewportSize <= 32
    if (offset < lastScrollOffsetRef.current - 1) {
      stickToBottomRef.current = false
    } else if (bottom) {
      stickToBottomRef.current = true
    }
    lastScrollOffsetRef.current = offset
    setAtBottom(bottom)
  }, [])

  // 切換會話：重置跟隨並瞬間定位到底部
  useLayoutEffect(() => {
    stickToBottomRef.current = true
    setAtBottom(true)
    // 等虛擬列表用最新 items 渲染後再對齊底部
    requestAnimationFrame(() => scrollToBottom())
    // 僅在 conversationId 變化時重置；scrollToBottom 依賴 items.length，故不列入依賴避免誤觸發
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [conversationId])

  // 自己發出新訊息時強制回到底部（即使剛才正往上翻歷史）
  useLayoutEffect(() => {
    const count = messages.length
    if (count > prevMessageCountRef.current && messages[count - 1]?.role === 'user') {
      stickToBottomRef.current = true
      setAtBottom(true)
    }
    prevMessageCountRef.current = count
  }, [messages])

  // 僅在“貼底”時隨內容增長釘住底部。virtua 內建 ResizeObserver 會在變高（KaTeX/圖片
  // mount 後撐高）時重測，這裡在每次內容/項數變化後重新對齊末尾，保證持續釘底。
  useLayoutEffect(() => {
    if (!stickToBottomRef.current) return
    scrollToBottom()
  }, [
    items,
    streaming,
    streamingContent,
    streamingReasoning,
    reasoningStreaming,
    streamingToolCalls,
    streamingSegments,
    groupsVersion,
    scrollToBottom,
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
              // 編輯/重生成入口在任何 run 在飛時都不可用（AC3）。streamFrozen 也算在飛：
              // 本機取消後 send invoke 尚未返回，此視窗內觸發只會被 in-flight 兜底靜默吞掉
              // （編輯文字會被無聲丟棄），所以從入口處直接收起。
              onRegenerateMessage={streaming || streamFrozen ? undefined : onRegenerateMessage}
              onDeleteMessage={onDeleteMessage}
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
              onDeleteMessage={onDeleteMessage}
            />
          )
        }
        case 'live-group':
          return (
            <MessageGroup
              conversationId={conversationId}
              groupId={item.groupId}
              messages={[]}
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
                  重試
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
      onDeleteMessage,
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

  return (
    <div className="relative flex min-h-0 flex-1 flex-col">
      <div
        ref={scrollRef}
        onWheel={handleWheel}
        className="chat-motion-fade custom-scrollbar flex-1 overflow-y-auto"
      >
        <div className="chat-message-list-inner mx-auto w-full max-w-3xl px-6">
          <Virtualizer ref={virtualizerRef} scrollRef={scrollRef} onScroll={handleScroll}>
            {items.map((item) => (
              <div
                key={item.key}
                className={item.kind === 'spacer' ? undefined : 'pb-0.5'}
                data-chat-message-list-item={item.kind}
              >
                {renderItem(item)}
              </div>
            ))}
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

// memo：列表本身訂閱 streamingStore，父級 Chat 重渲（非流式 state 變化）時不跟著白渲。
export const MessageList = memo(MessageListBase)
