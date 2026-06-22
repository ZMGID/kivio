import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Wrench, X } from 'lucide-react'
import { Sidebar, type ExtensionsNavItem } from './Sidebar'
import { ChatImageViewer } from './ChatImageViewer'
import { ChatTitlebarActions } from './ChatTitlebarActions'
import type { AssistantStreamStats } from './MessageList'
import { InputBar } from './InputBar'
import { ModelSelector } from './ModelSelector'
import { ExternalModelSelector, RuntimePicker } from './RuntimePicker'
import { PermissionPicker } from './PermissionPicker'
import { WindowControls } from './WindowControls'
import { ContextIndicator } from './ContextIndicator'
import { AgentTodoIndicator } from './AgentTodoIndicator'
import {
  agentRuntimesEqual,
  BUILTIN_AGENT_RUNTIME,
  chatApi,
  normalizeAgentRuntime,
  type AgentRuntimeConfig,
} from './api'
import {
  chatTitlebarMacInsetClass,
  chatTitlebarRowClass,
  usesNativeTitlebar,
} from './platform'
import type {
  ChatProject,
  ChatMessage,
  ChatAssistant,
  Conversation,
  ConversationListItem,
  ConversationContextState,
  AgentPlanMode,
  AgentPlanState,
  AgentTodoState,
  ChatMessageSegment,
  PendingAttachment,
  SkillMeta,
  ToolCallRecord,
} from './types'
import {
  api,
  type ChatExternalSendRequest,
  type ChatSessionConsentPayload,
  type ChatStreamPayload,
  type ChatToolConfirmPayload,
  type ChatToolDefinition,
  type ChatToolProgressPayload,
  type ChatUserPromptPayload,
} from '../api/tauri'
import type { SettingsShellHandle, SettingsTab } from '../settings/SettingsShell'
import { estimateTokens } from '../utils/tokens'
import {
  CHAT_MIN_SIZE_COLLAPSED,
  CHAT_MIN_SIZE_EXPANDED,
  forgetRememberedChatRoute,
  getChatPlatformWindowSize,
  getRememberedChatSidebarCollapsed,
  rememberChatSidebarCollapsed,
  rememberChatSize,
} from './persistence'
import { ChatDotGridBackground } from './ChatDotGridBackground'
import { normalizeToolCallStatus } from './toolStatus'
import { TypewriterText } from './TypewriterText'
import { pickRandomChatEmptyGreeting, isTauriRuntime } from './utils'
import { hasEnabledNativeBuiltinTool, hasEnabledSkillRuntime } from '../utils/chatTools'
import { onChatImageViewerOpen, type ChatImageViewerItem } from './imageViewer'
import {
  collectGeneratingConversationIds,
  createEmptyStreamSnapshot,
  isConversationBusy,
  isConversationInFlight,
  type ConversationStreamSnapshot,
} from './conversationRuns'
import { compareTimelineSegments, segmentStepNumber, segmentToolCallId } from './segments'

const AssistantCenter = lazy(() => import('./AssistantCenter').then((module) => ({
  default: module.AssistantCenter,
})))

const SettingsShell = lazy(() => import('../settings/SettingsShell').then((module) => ({
  default: module.SettingsShell,
})))

const SkillCenter = lazy(() => import('./SkillCenter').then((module) => ({
  default: module.SkillCenter,
})))

const MessageList = lazy(() => import('./MessageList').then((module) => ({
  default: module.MessageList,
})))

function ChatPaneLoading() {
  return (
    <div className="chat-themed-surface flex h-full w-full items-center justify-center">
      <div className="h-5 w-5 animate-spin rounded-full border-2 border-neutral-300 border-t-neutral-800 dark:border-neutral-700 dark:border-t-neutral-200" />
    </div>
  )
}

function MessageListLoading() {
  return (
    <div className="chat-themed-surface flex flex-1 items-center justify-center">
      <div className="h-5 w-5 animate-spin rounded-full border-2 border-neutral-300 border-t-neutral-800 dark:border-neutral-700 dark:border-t-neutral-200" />
    </div>
  )
}

type ChatView = 'conversation' | 'settings' | 'assistants' | 'skill'

interface ChatProps {
  onSettingsChange: () => void
}

function hashPath(): string {
  return window.location.hash.replace('#', '').split('?')[0]
}

function isChatSettingsPath(path: string): boolean {
  return path === 'chat/settings' || path.startsWith('chat/settings/')
}

function isChatAssistantCenterPath(path: string): boolean {
  return path === 'chat/assistants' || path.startsWith('chat/assistants/')
}

function isChatSkillCenterPath(path: string): boolean {
  return path === 'chat/skill' || path.startsWith('chat/skill/')
}

function scheduleIdleTask(callback: () => void, timeout = 1200): () => void {
  const idleWindow = window as Window & {
    requestIdleCallback?: (cb: () => void, options?: { timeout?: number }) => number
    cancelIdleCallback?: (handle: number) => void
  }
  if (idleWindow.requestIdleCallback && idleWindow.cancelIdleCallback) {
    const handle = idleWindow.requestIdleCallback(callback, { timeout })
    return () => idleWindow.cancelIdleCallback?.(handle)
  }

  const handle = window.setTimeout(callback, timeout)
  return () => window.clearTimeout(handle)
}

function toolEventToRecord(payload: ChatToolProgressPayload): ToolCallRecord {
  return {
    id: payload.id || payload.toolCallId,
    toolCallId: payload.toolCallId,
    conversationId: payload.conversationId,
    runId: payload.runId,
    messageId: payload.messageId,
    name: payload.name,
    source: payload.source,
    serverId: payload.serverId ?? undefined,
    status: normalizeToolCallStatus(payload.status),
    arguments: payload.argumentsPreview,
    argumentPreview: payload.argumentsPreview,
    argumentsPreview: payload.argumentsPreview,
    resultPreview: payload.resultPreview ?? undefined,
    error: payload.error ?? undefined,
    startedAt: payload.startedAt ?? undefined,
    completedAt: payload.completedAt ?? undefined,
    durationMs: payload.durationMs ?? undefined,
    round: payload.round,
    sensitive: payload.sensitive,
    artifacts: payload.artifacts ?? [],
    traceId: payload.traceId ?? undefined,
    spanId: payload.spanId ?? undefined,
    structuredContent: payload.structuredContent,
  }
}

function userPromptEventToRecord(payload: ChatUserPromptPayload): ToolCallRecord {
  return {
    id: payload.id || payload.toolCallId,
    toolCallId: payload.toolCallId,
    conversationId: payload.conversationId,
    runId: payload.runId,
    messageId: payload.messageId,
    name: payload.name || 'ask_user',
    source: payload.source || 'native',
    status: 'running',
    arguments: payload.prompt,
    args: payload.prompt,
    input: payload.prompt,
    sensitive: false,
    artifacts: [],
    structuredContent: payload.structuredContent ?? {
      askUser: {
        phase: 'awaiting',
        title: payload.prompt.title,
        questions: payload.prompt.questions,
        answers: {},
      },
    },
  }
}

function streamPayloadToSegment(payload: ChatStreamPayload): ChatMessageSegment | null {
  const raw = payload.segment ?? null
  const id = payload.segmentId ?? raw?.id
  const kind = payload.segmentKind ?? raw?.kind
  const phase = payload.phase ?? raw?.phase
  const order = payload.order ?? raw?.order
  if (!id || !kind || !phase || order == null) return null

  const stepNumber = raw?.step_number ?? raw?.stepNumber ?? payload.stepNumber ?? null
  const toolCallId = raw?.tool_call_id ?? raw?.toolCallId ?? payload.toolCallId ?? null
  return {
    id,
    kind,
    phase,
    order,
    step_number: stepNumber,
    stepNumber,
    round: raw?.round ?? payload.round ?? null,
    text: raw?.text ?? null,
    tool_call_id: toolCallId,
    toolCallId,
  }
}

function upsertStreamSegment(
  segments: ChatMessageSegment[],
  incoming: ChatMessageSegment,
  delta = '',
): ChatMessageSegment[] {
  const incomingToolCallId = segmentToolCallId(incoming)
  const index = segments.findIndex((segment) => (
    segment.id === incoming.id ||
    (incoming.kind === 'tool' &&
      segment.kind === 'tool' &&
      incomingToolCallId &&
      segmentToolCallId(segment) === incomingToolCallId)
  ))
  const existing = index >= 0 ? segments[index] : null
  const nextText = incoming.kind === 'tool'
    ? incoming.text ?? existing?.text ?? null
    : (() => {
        const base = existing?.text ?? incoming.text ?? ''
        const append = !existing && incoming.text && incoming.text === delta ? '' : delta
        return `${base}${append}`
      })()
  const existingStepNumber = existing ? segmentStepNumber(existing) : null
  const incomingStepNumber = segmentStepNumber(incoming)
  const nextSegment: ChatMessageSegment = {
    ...existing,
    ...incoming,
    step_number: incomingStepNumber ?? existingStepNumber ?? null,
    stepNumber: incomingStepNumber ?? existingStepNumber ?? null,
    tool_call_id: incoming.tool_call_id ?? incoming.toolCallId ?? existing?.tool_call_id ?? existing?.toolCallId ?? null,
    toolCallId: incoming.toolCallId ?? incoming.tool_call_id ?? existing?.toolCallId ?? existing?.tool_call_id ?? null,
    text: nextText,
  }
  const next = index < 0
    ? [...segments, nextSegment]
    : segments.map((segment, i) => (i === index ? nextSegment : segment))
  return next.sort(compareTimelineSegments)
}

function nextSegmentOrder(segments: ChatMessageSegment[]): number {
  if (segments.length === 0) return 1
  return Math.max(...segments.map((segment) => segment.order ?? 0)) + 1
}

function upsertToolStreamSegment(
  segments: ChatMessageSegment[],
  record: ToolCallRecord,
): ChatMessageSegment[] {
  const toolCallId = record.id || record.toolCallId || ''
  if (!toolCallId) return segments
  const exists = segments.some(
    (segment) => segment.kind === 'tool' && segmentToolCallId(segment) === toolCallId,
  )
  if (exists) return segments
  return upsertStreamSegment(segments, {
    id: `seg_tool_${toolCallId}`,
    kind: 'tool',
    phase: 'tool_loop',
    order: nextSegmentOrder(segments),
    round: record.round ?? 1,
    tool_call_id: toolCallId,
    toolCallId,
  })
}

function sameSegmentField<T>(left: T | null | undefined, right: T | null | undefined): boolean {
  return (left ?? null) === (right ?? null)
}

function findReasoningSegmentForText(
  segments: ChatMessageSegment[],
  textSegment: ChatMessageSegment,
): ChatMessageSegment | null {
  const reversedReasoning = [...segments]
    .reverse()
    .filter((item) => item.kind === 'reasoning')
  const textStepNumber = segmentStepNumber(textSegment)
  const textRound = textSegment.round ?? null

  return reversedReasoning.find((item) => (
    segmentStepNumber(item) === textStepNumber &&
    sameSegmentField(item.round, textRound) &&
    item.phase === textSegment.phase
  ))
    ?? reversedReasoning.find((item) => (
      segmentStepNumber(item) === textStepNumber &&
      sameSegmentField(item.round, textRound)
    ))
    ?? reversedReasoning.find((item) => segmentStepNumber(item) === textStepNumber)
    ?? reversedReasoning[0]
    ?? null
}

function updateReasoningSegmentDuration(
  snapshot: ConversationStreamSnapshot,
  segmentId: string,
  now = Date.now(),
) {
  const startedAt = snapshot.reasoningStartedAtBySegmentId[segmentId]
  if (startedAt == null) return
  snapshot.reasoningDurationMsBySegmentId = {
    ...snapshot.reasoningDurationMsBySegmentId,
    [segmentId]: Math.max(
      snapshot.reasoningDurationMsBySegmentId[segmentId] ?? 0,
      now - startedAt,
    ),
  }
}

function normalizeSkill(skill: import('../api/tauri').SkillMeta): SkillMeta {
  return {
    id: skill.id,
    name: skill.name,
    description: skill.description,
    source: skill.source,
    path: skill.path ?? undefined,
    recommendedTools: skill.recommendedTools,
    disableModelInvocation: skill.disableModelInvocation,
    files: skill.files,
  }
}

function skillRecommendedTools(skill?: SkillMeta | null): string[] {
  return skill?.recommended_tools ?? skill?.recommendedTools ?? []
}

function toolMatchesRecommendation(tool: ChatToolDefinition, recommended: string): boolean {
  const name = recommended.trim()
  if (!name) return false
  return (
    tool.name === name ||
    tool.id === name ||
    `${tool.serverId ?? ''}:${tool.name}` === name
  )
}

function attachmentExtension(name: string): string {
  return name.split('.').pop()?.toLowerCase() ?? ''
}

function documentSkillNameForAttachment(attachment: PendingAttachment): string | null {
  if (attachment.type === 'image') return null
  switch (attachmentExtension(attachment.name)) {
    case 'pdf':
      return 'pdf'
    case 'doc':
    case 'docx':
      return 'docx'
    case 'xls':
    case 'xlsx':
    case 'xlsm':
    case 'csv':
    case 'tsv':
      return 'xlsx'
    default:
      return null
  }
}

function findEnabledSkillId(skills: SkillMeta[], skillName: string): string | null {
  const normalized = skillName.toLowerCase()
  return skills.find((skill) => (
    skill.id.toLowerCase() === normalized || skill.name.toLowerCase() === normalized
  ))?.id ?? null
}

function inferSingleAttachmentSkillId(
  attachments: PendingAttachment[],
  skills: SkillMeta[],
): string | null {
  const skillNames = Array.from(new Set(
    attachments
      .map(documentSkillNameForAttachment)
      .filter((name): name is string => Boolean(name)),
  ))
  if (skillNames.length !== 1) return null
  return findEnabledSkillId(skills, skillNames[0])
}

function isLocallyCancelledPayload(
  payload: { conversationId: string; runId?: string },
  cancelledConversationId: string | null,
  cancelledRunId: string | null,
): boolean {
  if (cancelledConversationId !== payload.conversationId) return false
  return !cancelledRunId || !payload.runId || payload.runId === cancelledRunId
}

function isPlainBlankConversation(conversation: Conversation | null): boolean {
  return Boolean(
    conversation
    && conversation.messages.length === 0
    && !(conversation.assistant_id ?? conversation.assistantId),
  )
}

function conversationUsesModel(
  conversation: Conversation,
  providerId: string,
  model: string,
): boolean {
  return conversation.provider_id === providerId && conversation.model === model
}

function optimisticConversationTitle(content: string): string {
  const compact = content.replace(/\s+/g, ' ').trim()
  if (!compact) return '新对话'
  return compact.length > 30 ? `${compact.slice(0, 30)}...` : compact
}

function optimisticConversationListItem(
  conversation: Conversation,
  content: string,
): ConversationListItem {
  const preview = content.replace(/\s+/g, ' ').trim()
  const title = conversation.title === '新对话'
    ? optimisticConversationTitle(content)
    : conversation.title
  return {
    id: conversation.id,
    title,
    preview: preview.length > 100 ? `${preview.slice(0, 100)}...` : preview,
    provider_id: conversation.provider_id,
    model: conversation.model,
    message_count: Math.max(1, conversation.messages.length),
    created_at: conversation.created_at,
    updated_at: Math.floor(Date.now() / 1000),
    pinned: conversation.pinned,
    folder: conversation.folder,
    project_id: conversation.project_id ?? conversation.projectId ?? null,
    projectId: conversation.project_id ?? conversation.projectId ?? null,
    assistant_id: conversation.assistant_id ?? conversation.assistantId ?? null,
    assistantId: conversation.assistant_id ?? conversation.assistantId ?? null,
    assistant_name:
      conversation.assistant_snapshot?.name
      ?? conversation.assistantSnapshot?.name
      ?? null,
    assistantName:
      conversation.assistant_snapshot?.name
      ?? conversation.assistantSnapshot?.name
      ?? null,
  }
}

type SendMessageOptions = {
  forceNewConversation?: boolean
  conversationOverride?: Conversation | null
}

export default function Chat({ onSettingsChange }: ChatProps) {
  const [chatView, setChatView] = useState<ChatView>(() => {
    const path = hashPath()
    if (isChatSettingsPath(path)) return 'settings'
    if (isChatAssistantCenterPath(path)) return 'assistants'
    if (isChatSkillCenterPath(path)) return 'skill'
    return 'conversation'
  })
  const [currentConversation, setCurrentConversation] = useState<Awaited<
    ReturnType<typeof chatApi.getConversation>
  > | null>(null)
  const [sidebarCollapsed, setSidebarCollapsed] = useState(() => getRememberedChatSidebarCollapsed())
  const [searchOpen, setSearchOpen] = useState(false)
  const [selectedProject, setSelectedProject] = useState<ChatProject | null>(null)
  const [streaming, setStreaming] = useState(false)
  // 取消后冻结展示已生成的部分内容，直到 send invoke 返回持久化消息无缝替换。
  const [streamFrozen, setStreamFrozen] = useState(false)
  const [cancellingStream, setCancellingStream] = useState(false)
  const [streamingContent, setStreamingContent] = useState('')
  const [streamingReasoning, setStreamingReasoning] = useState('')
  const [streamingReasoningDurationMs, setStreamingReasoningDurationMs] = useState<number | null>(null)
  const [streamingReasoningDurationMsBySegmentId, setStreamingReasoningDurationMsBySegmentId] =
    useState<Record<string, number>>({})
  const [reasoningStreaming, setReasoningStreaming] = useState(false)
  const [streamError, setStreamError] = useState('')
  const [streamingSegments, setStreamingSegments] = useState<ChatMessageSegment[]>([])
  /** 发送中待显示的用户消息（与 conversation 分离，避免 route reload 冲掉） */
  const [pendingUserMessage, setPendingUserMessage] = useState<ChatMessage | null>(null)
  const [pendingUserMessageConversationId, setPendingUserMessageConversationId] = useState<string | null>(null)
  const [assistantStreamStatsByMessageId, setAssistantStreamStatsByMessageId] =
    useState<Record<string, AssistantStreamStats>>({})
  const [sidebarRefreshKey, setSidebarRefreshKey] = useState(0)
  const [optimisticSidebarConversations, setOptimisticSidebarConversations] =
    useState<ConversationListItem[]>([])
  const [generatingConversationIds, setGeneratingConversationIds] = useState<ReadonlySet<string>>(
    () => new Set(),
  )
  const [sidebarProfileRefreshKey, setSidebarProfileRefreshKey] = useState(0)
  const [draftProviderId, setDraftProviderId] = useState('')
  const [draftModel, setDraftModel] = useState('')
  const [draftAgentRuntime, setDraftAgentRuntime] = useState<AgentRuntimeConfig>(
    BUILTIN_AGENT_RUNTIME,
  )
  const [skills, setSkills] = useState<SkillMeta[]>([])
  const [disabledSkillIds, setDisabledSkillIds] = useState<string[]>([])
  const [settingsInitialTab, setSettingsInitialTab] = useState<SettingsTab>('chat')
  const [extensionsNavItem, setExtensionsNavItem] = useState<ExtensionsNavItem | null>(null)
  const [streamingToolCalls, setStreamingToolCalls] = useState<ToolCallRecord[]>([])
  const [enabledTools, setEnabledTools] = useState<ChatToolDefinition[]>([])
  const [enabledToolCount, setEnabledToolCount] = useState<number | null>(null)
  const [toolsDisabledReason, setToolsDisabledReason] = useState('')
  const [toolsRequested, setToolsRequested] = useState(false)
  const [approvalPolicy, setApprovalPolicy] = useState('readonly_auto_sensitive_confirm')
  const [pendingToolConfirm, setPendingToolConfirm] = useState<ChatToolConfirmPayload | null>(null)
  const [pendingSessionConsent, setPendingSessionConsent] = useState<ChatSessionConsentPayload | null>(null)
  const [contextState, setContextState] = useState<ConversationContextState | null>(null)
  const [contextLoading, setContextLoading] = useState(false)
  const [contextCompressing, setContextCompressing] = useState(false)
  const [contextError, setContextError] = useState('')
  const [imageViewerItem, setImageViewerItem] = useState<ChatImageViewerItem | null>(null)
  const currentConversationIdRef = useRef<string | null>(null)
  const activeRunIdRef = useRef<string | null>(null)
  const locallyCancelledConversationIdRef = useRef<string | null>(null)
  const locallyCancelledRunIdRef = useRef<string | null>(null)
  const inFlightConversationsRef = useRef<Set<string>>(new Set())
  const externalSendQueueRef = useRef<ChatExternalSendRequest[]>([])
  const externalSendDrainProcessingRef = useRef(false)
  const externalSendDrainRequestedRef = useRef(false)
  const pendingStreamDoneRef = useRef<Record<string, () => Promise<void>>>({})
  const streamSnapshotsRef = useRef<Record<string, ConversationStreamSnapshot>>({})
  const streamErrorsRef = useRef<Record<string, string>>({})
  const pendingToolConfirmsRef = useRef<Record<string, ChatToolConfirmPayload>>({})
  const pendingSessionConsentsRef = useRef<Record<string, ChatSessionConsentPayload>>({})
  const streamStartedAtRef = useRef<number | null>(null)
  const streamingContentRef = useRef('')
  const streamingReasoningRef = useRef('')
  const settingsRef = useRef<SettingsShellHandle>(null)
  const pendingAfterSettingsCloseRef = useRef<(() => void) | null>(null)
  // A 合帧（render coalescing）：高频 stream/tool/subagent/userprompt 事件不再每条都同步
  // setState 重渲，而是把"待显示的快照"记到 ref，用 requestAnimationFrame 每帧最多 flush 一次。
  const pendingStreamRenderRef = useRef<{ conversationId: string; snapshot: ConversationStreamSnapshot } | null>(null)
  const streamRenderRafRef = useRef<number | null>(null)

  useEffect(() => onChatImageViewerOpen(setImageViewerItem), [])

  const syncGeneratingConversationIds = useCallback(() => {
    setGeneratingConversationIds(collectGeneratingConversationIds(
      inFlightConversationsRef.current,
      streamSnapshotsRef.current,
      pendingToolConfirmsRef.current,
    ))
  }, [])

  const markConversationInFlight = useCallback((conversationId: string) => {
    inFlightConversationsRef.current.add(conversationId)
    syncGeneratingConversationIds()
  }, [syncGeneratingConversationIds])

  const clearConversationInFlight = useCallback((conversationId: string) => {
    inFlightConversationsRef.current.delete(conversationId)
    syncGeneratingConversationIds()
  }, [syncGeneratingConversationIds])

  // B：彻底把一个会话从所有本地乐观/in-flight/快照状态中剔除（ghost 清理）。
  // 不触碰 currentConversation/route，由调用方按场景决定。
  const dropConversationLocally = useCallback((conversationId: string) => {
    inFlightConversationsRef.current.delete(conversationId)
    delete streamSnapshotsRef.current[conversationId]
    delete pendingToolConfirmsRef.current[conversationId]
    delete pendingSessionConsentsRef.current[conversationId]
    delete pendingStreamDoneRef.current[conversationId]
    delete streamErrorsRef.current[conversationId]
    // 若该会话还挂着待刷新的合帧，连带取消，避免被剔除的 ghost 还闪一帧。
    if (pendingStreamRenderRef.current?.conversationId === conversationId) {
      if (streamRenderRafRef.current != null) {
        cancelAnimationFrame(streamRenderRafRef.current)
        streamRenderRafRef.current = null
      }
      pendingStreamRenderRef.current = null
    }
    setOptimisticSidebarConversations((items) => items.filter((item) => item.id !== conversationId))
    syncGeneratingConversationIds()
  }, [syncGeneratingConversationIds])

  const setStreamErrorForConversation = useCallback((conversationId: string, error: string) => {
    if (error) {
      streamErrorsRef.current[conversationId] = error
    } else {
      delete streamErrorsRef.current[conversationId]
    }
    if (currentConversationIdRef.current === conversationId) {
      setStreamError(error)
    }
  }, [])

  const isCurrentConversationBusy = useCallback(() => (
    isConversationBusy(
      currentConversationIdRef.current,
      inFlightConversationsRef.current,
      streamSnapshotsRef.current,
    )
  ), [])

  const applyConversation = useCallback((conversation: Conversation | null) => {
    setCurrentConversation(conversation)
    setContextState(conversation?.context_state ?? conversation?.contextState ?? null)
  }, [])

  const patchContextState = useCallback((nextState: ConversationContextState) => {
    setContextState(nextState)
    setCurrentConversation((prev) => prev
      ? { ...prev, context_state: nextState, contextState: nextState }
      : prev)
  }, [])

  const patchAgentTodoState = useCallback((nextState: AgentTodoState) => {
    setCurrentConversation((prev) => prev
      ? { ...prev, agent_todo_state: nextState, agentTodoState: nextState }
      : prev)
  }, [])

  const patchAgentPlanState = useCallback((nextState: AgentPlanState) => {
    setCurrentConversation((prev) => prev
      ? { ...prev, agent_plan_state: nextState, agentPlanState: nextState }
      : prev)
  }, [])

  const clearStreamingPreview = useCallback(() => {
    // 取消挂起的合帧，避免旧快照在清空后又被刷回来产生空帧/串帧。
    if (streamRenderRafRef.current != null) {
      cancelAnimationFrame(streamRenderRafRef.current)
      streamRenderRafRef.current = null
    }
    pendingStreamRenderRef.current = null
    setStreaming(false)
    setStreamFrozen(false)
    setCancellingStream(false)
    setStreamingContent('')
    setStreamingReasoning('')
    setStreamingReasoningDurationMs(null)
    setStreamingReasoningDurationMsBySegmentId({})
    setReasoningStreaming(false)
    setStreamingToolCalls([])
    setStreamingSegments([])
    activeRunIdRef.current = null
    streamStartedAtRef.current = null
    streamingContentRef.current = ''
    streamingReasoningRef.current = ''
  }, [])

  const ensureStreamSnapshot = useCallback((conversationId: string) => {
    const existing = streamSnapshotsRef.current[conversationId]
    if (existing) return existing
    const snapshot = createEmptyStreamSnapshot()
    streamSnapshotsRef.current[conversationId] = snapshot
    syncGeneratingConversationIds()
    return snapshot
  }, [syncGeneratingConversationIds])

  const restoreStreamingPreview = useCallback((conversationId: string | null) => {
    // 切换会话/恢复预览前取消任何挂起的合帧，避免上一个会话的快照被刷到当前视图。
    if (streamRenderRafRef.current != null) {
      cancelAnimationFrame(streamRenderRafRef.current)
      streamRenderRafRef.current = null
    }
    pendingStreamRenderRef.current = null
    if (!conversationId) {
      clearStreamingPreview()
      setPendingToolConfirm(null)
      setPendingSessionConsent(null)
      setStreamError('')
      return
    }
    const snapshot = streamSnapshotsRef.current[conversationId]
    if (!snapshot) {
      clearStreamingPreview()
    } else {
      setStreaming(snapshot.streaming)
      setStreamFrozen(false)
      setCancellingStream(false)
      setStreamingContent(snapshot.content)
      setStreamingReasoning(snapshot.reasoning)
      setStreamingReasoningDurationMs(snapshot.reasoningDurationMs)
      setStreamingReasoningDurationMsBySegmentId(snapshot.reasoningDurationMsBySegmentId)
      setReasoningStreaming(snapshot.reasoningStreaming)
      setStreamingToolCalls(snapshot.toolCalls)
      setStreamingSegments(snapshot.segments)
      activeRunIdRef.current = snapshot.runId
      streamStartedAtRef.current = snapshot.startedAt
      streamingContentRef.current = snapshot.content
      streamingReasoningRef.current = snapshot.reasoning
    }
    setStreamError(streamErrorsRef.current[conversationId] ?? '')
    setPendingToolConfirm(pendingToolConfirmsRef.current[conversationId] ?? null)
    setPendingSessionConsent(pendingSessionConsentsRef.current[conversationId] ?? null)
  }, [clearStreamingPreview])

  const applyStreamSnapshotToState = useCallback((snapshot: ConversationStreamSnapshot) => {
    setStreaming(snapshot.streaming)
    setCancellingStream(false)
    setStreamingContent(snapshot.content)
    setStreamingReasoning(snapshot.reasoning)
    setStreamingReasoningDurationMs(snapshot.reasoningDurationMs)
    setStreamingReasoningDurationMsBySegmentId(snapshot.reasoningDurationMsBySegmentId)
    setReasoningStreaming(snapshot.reasoningStreaming)
    setStreamingToolCalls(snapshot.toolCalls)
    setStreamingSegments(snapshot.segments)
    activeRunIdRef.current = snapshot.runId
    streamStartedAtRef.current = snapshot.startedAt
    streamingContentRef.current = snapshot.content
    streamingReasoningRef.current = snapshot.reasoning
  }, [])

  // 立即把挂起帧刷出去（done/结束、卸载、切换会话前调用），保证不丢最后一帧。
  const flushStreamRender = useCallback(() => {
    if (streamRenderRafRef.current != null) {
      cancelAnimationFrame(streamRenderRafRef.current)
      streamRenderRafRef.current = null
    }
    const pending = pendingStreamRenderRef.current
    pendingStreamRenderRef.current = null
    if (!pending) return
    if (currentConversationIdRef.current !== pending.conversationId) return
    applyStreamSnapshotToState(pending.snapshot)
  }, [applyStreamSnapshotToState])

  // 取消挂起帧而不应用（切换会话/卸载时调用，避免把旧会话快照刷到新会话）。
  // 注：clearStreamingPreview / restoreStreamingPreview 已内联同样的取消逻辑。

  // A 合帧：事件本身仍即时累积到 snapshot 对象，这里只把"渲染"节流到每帧一次。
  // immediate=true（done 等终止帧）立即 flush，不再等下一帧。
  const showStreamSnapshotIfCurrent = useCallback((
    conversationId: string,
    snapshot: ConversationStreamSnapshot,
    immediate = false,
  ) => {
    if (currentConversationIdRef.current !== conversationId) return
    pendingStreamRenderRef.current = { conversationId, snapshot }
    if (immediate) {
      flushStreamRender()
      return
    }
    if (streamRenderRafRef.current != null) return
    streamRenderRafRef.current = requestAnimationFrame(() => {
      streamRenderRafRef.current = null
      const pending = pendingStreamRenderRef.current
      pendingStreamRenderRef.current = null
      if (!pending) return
      if (currentConversationIdRef.current !== pending.conversationId) return
      applyStreamSnapshotToState(pending.snapshot)
    })
  }, [applyStreamSnapshotToState, flushStreamRender])

  useEffect(() => () => {
    if (streamRenderRafRef.current != null) {
      cancelAnimationFrame(streamRenderRafRef.current)
      streamRenderRafRef.current = null
    }
  }, [])

  const clearStreamSnapshot = useCallback((conversationId: string | null) => {
    if (!conversationId) return
    delete streamSnapshotsRef.current[conversationId]
    delete pendingToolConfirmsRef.current[conversationId]
    delete pendingSessionConsentsRef.current[conversationId]
    syncGeneratingConversationIds()
    if (currentConversationIdRef.current === conversationId) {
      setPendingToolConfirm(null)
      setPendingSessionConsent(null)
      clearStreamingPreview()
    }
  }, [clearStreamingPreview, syncGeneratingConversationIds])

  const cancelCurrentRunLocally = useCallback(() => {
    locallyCancelledConversationIdRef.current = currentConversationIdRef.current
    locallyCancelledRunIdRef.current = activeRunIdRef.current
    // 立即停掉"生成中"视觉（撤掉取消按钮 + 停 shimmer），但保留已生成文本：
    // 切到 frozen 态冻结展示，等 send invoke 返回持久化消息时由
    // finishStreamingRunWithConversation 无缝替换（clearStreamingPreview 会清除 frozen）。
    // 后续迟到的流事件已被 isLocallyCancelledPayload 过滤，预览不会再变动。
    setStreaming(false)
    setReasoningStreaming(false)
    setStreamFrozen(true)
    const conversationId = currentConversationIdRef.current
    if (conversationId) {
      delete pendingToolConfirmsRef.current[conversationId]
      delete pendingSessionConsentsRef.current[conversationId]
    }
    setPendingToolConfirm(null)
    setPendingSessionConsent(null)
  }, [])

  const resetLocalCancellation = useCallback(() => {
    locallyCancelledConversationIdRef.current = null
    locallyCancelledRunIdRef.current = null
  }, [])

  const activeAgentRuntime = currentConversation
    ? normalizeAgentRuntime(currentConversation)
    : draftAgentRuntime
  const usesExternalRuntime = activeAgentRuntime.kind === 'external' && !!activeAgentRuntime.externalAgentId
  const currentConversationIsBlank = isPlainBlankConversation(currentConversation)
  const activeProviderId = currentConversation && !currentConversationIsBlank
    ? currentConversation.provider_id
    : draftProviderId
  const activeModel = currentConversation && !currentConversationIsBlank
    ? currentConversation.model
    : draftModel
  const storedActiveSkillId = currentConversation
    ? currentConversation.active_skill_id ?? currentConversation.activeSkillId ?? null
    : null
  const enabledSkills = useMemo(
    () => skills.filter((skill) => !disabledSkillIds.includes(skill.id)),
    [disabledSkillIds, skills],
  )
  const slashSkills = useMemo(
    () => enabledSkills.map((skill) => ({
      id: skill.id,
      name: skill.name,
      description: skill.description,
      argumentHint: skill.argumentHint ?? skill.argument_hint ?? undefined,
      disableModelInvocation: skill.disableModelInvocation ?? skill.disable_model_invocation,
    })),
    [enabledSkills],
  )
  const effectiveSkillId = useMemo(() => {
    if (
      storedActiveSkillId
      && enabledSkills.some((skill) => skill.id === storedActiveSkillId)
    ) {
      return storedActiveSkillId
    }
    return null
  }, [enabledSkills, storedActiveSkillId])
  const effectiveSkill = useMemo(
    () => enabledSkills.find((skill) => skill.id === effectiveSkillId) ?? null,
    [effectiveSkillId, enabledSkills],
  )
  const effectiveSkillRecommendedTools = useMemo(
    () => skillRecommendedTools(effectiveSkill),
    [effectiveSkill],
  )

  const currentAssistantSnapshot =
    currentConversation?.assistant_snapshot ?? currentConversation?.assistantSnapshot ?? null
  const currentAssistantId =
    currentConversation?.assistant_id
    ?? currentConversation?.assistantId
    ?? currentAssistantSnapshot?.id
    ?? null

  const refreshToolIndicator = useCallback(async () => {
    if (!isTauriRuntime()) {
      setEnabledTools([])
      setEnabledToolCount(null)
      setToolsDisabledReason('')
      setToolsRequested(false)
      setApprovalPolicy('readonly_auto_sensitive_confirm')
      return
    }
    try {
      const settings = await api.getSettings()
      const chatTools = settings.chatTools
      setApprovalPolicy(chatTools?.approvalPolicy || 'readonly_auto_sensitive_confirm')
      const nextDisabledSkillIds = chatTools?.disabledSkillIds ?? []
      setDisabledSkillIds((prev) =>
        prev.length === nextDisabledSkillIds.length
        && prev.every((id, index) => id === nextDisabledSkillIds[index])
          ? prev
          : nextDisabledSkillIds,
      )
      if (!chatTools) {
        setEnabledTools([])
        setEnabledToolCount(null)
        setToolsDisabledReason('')
        setToolsRequested(false)
        setApprovalPolicy('readonly_auto_sensitive_confirm')
        return
      }
      const provider = settings.providers.find((item) => item.id === activeProviderId)
      const anyMcpEnabled = chatTools.enabled && chatTools.servers.some((server) => server.enabled)
      const anyNativeEnabled = hasEnabledNativeBuiltinTool(chatTools.nativeTools)
      const skillRuntimeEnabled = hasEnabledSkillRuntime(chatTools.nativeTools)
      const requested = anyMcpEnabled || anyNativeEnabled || skillRuntimeEnabled
      setToolsRequested(requested)
      if (!requested) {
        setEnabledTools([])
        setEnabledToolCount(null)
        setToolsDisabledReason('')
        return
      }
      if (provider?.supportsTools === false) {
        setEnabledTools([])
        setEnabledToolCount(0)
        if (skillRuntimeEnabled && effectiveSkillId) {
          setToolsDisabledReason('当前模型不支持 tools；已选 Skill 时将注入 SKILL.md')
        } else if (skillRuntimeEnabled) {
          setToolsDisabledReason('Skill 渐进式加载需要 tools 支持；已选 Skill 时将注入 SKILL.md')
        } else {
          setToolsDisabledReason('当前模型不支持 tools')
        }
        return
      }
      const result = await api.chatMcpListTools()
      const tools = result.success ? result.tools : []
      setEnabledTools(tools)
      setEnabledToolCount(tools.length)
      setToolsDisabledReason(result.success ? '' : result.error || '工具不可用')
    } catch (err) {
      setEnabledTools([])
      setToolsRequested(false)
      setEnabledToolCount(null)
      setApprovalPolicy('readonly_auto_sensitive_confirm')
      setToolsDisabledReason(err instanceof Error ? err.message : String(err))
    }
  }, [activeProviderId, effectiveSkillId])

  const handleApprovalPolicyChange = useCallback(async (nextApprovalPolicy: string) => {
    setApprovalPolicy(nextApprovalPolicy)
    try {
      const settings = await api.getSettings()
      await api.saveSettings({
        ...settings,
        chatTools: {
          ...settings.chatTools,
          approvalPolicy: nextApprovalPolicy,
        },
      })
      onSettingsChange()
    } catch (err) {
      console.error('Failed to update approval policy:', err)
      void refreshToolIndicator()
    }
  }, [onSettingsChange, refreshToolIndicator])

  const unavailableRecommendedTools = useMemo(
    () =>
      effectiveSkillRecommendedTools.filter(
        (recommended) => !enabledTools.some((tool) => toolMatchesRecommendation(tool, recommended)),
      ),
    [effectiveSkillRecommendedTools, enabledTools],
  )

  const toolStatusHint = useMemo(() => {
    if (toolsDisabledReason && (enabledToolCount ?? 0) === 0 && (toolsRequested || effectiveSkillRecommendedTools.length > 0)) {
      if (toolsDisabledReason.includes('不支持 tools') && effectiveSkillId) {
        return toolsDisabledReason
      }
      return effectiveSkillRecommendedTools.length > 0
        ? `当前 Skill 需要工具，但${toolsDisabledReason}`
        : toolsDisabledReason
    }
    if (toolsDisabledReason && (enabledToolCount ?? 0) === 0) {
      return ''
    }
    if (unavailableRecommendedTools.length > 0) {
      return `当前 Skill 推荐的工具不可用：${unavailableRecommendedTools.slice(0, 3).join(', ')}`
    }
    return ''
  }, [effectiveSkillId, effectiveSkillRecommendedTools.length, enabledToolCount, toolsDisabledReason, toolsRequested, unavailableRecommendedTools])

  const sendDisabledReason = effectiveSkillRecommendedTools.length > 0 ? toolStatusHint : ''

  const getRouteConversationId = useCallback(() => {
    const path = hashPath()
    if (!path.startsWith('chat/')) return null
    const rest = path.slice('chat/'.length)
    if (rest === 'settings' || rest.startsWith('settings/')) return null
    if (rest === 'assistants' || rest.startsWith('assistants/')) return null
    return decodeURIComponent(rest)
  }, [])

  const syncConversationRoute = useCallback((conversationId: string | null) => {
    const nextHash = conversationId ? `#chat/${encodeURIComponent(conversationId)}` : '#chat'
    if (window.location.hash !== nextHash) {
      window.location.hash = nextHash
    }
  }, [])

  const syncSettingsRoute = useCallback(() => {
    if (window.location.hash !== '#chat/settings') {
      window.location.hash = '#chat/settings'
    }
  }, [])

  const syncAssistantCenterRoute = useCallback(() => {
    if (window.location.hash !== '#chat/assistants') {
      window.location.hash = '#chat/assistants'
    }
  }, [])

  const syncSkillCenterRoute = useCallback(() => {
    if (window.location.hash !== '#chat/skill') {
      window.location.hash = '#chat/skill'
    }
  }, [])

  const refreshSidebar = useCallback(() => {
    setSidebarRefreshKey((key) => key + 1)
  }, [])

  const loadDefaultModel = useCallback(async () => {
    try {
      const settings = await api.getSettings()
      const chatDefault = settings.defaultModels.chat
      if (chatDefault.providerId) {
        setDraftProviderId(chatDefault.providerId)
        setDraftModel(chatDefault.model)
      } else if (settings.lens?.providerId) {
        setDraftProviderId(settings.lens.providerId)
        setDraftModel(settings.lens.model || '')
      } else {
        setDraftProviderId(settings.translatorProviderId || '')
        setDraftModel(settings.translatorModel || '')
      }
    } catch {
      setDraftProviderId('dev-provider')
      setDraftModel('dev-model')
    }
  }, [])

  const loadSkills = useCallback(async () => {
    if (!isTauriRuntime()) {
      setSkills([])
      return
    }
    try {
      const result = await api.chatSkillsList()
      if (result.success) {
        setSkills(result.skills.map(normalizeSkill))
        if (result.error) {
          console.warn('Some chat skills could not be loaded:', result.error)
        }
      } else {
        setSkills([])
        console.error('Failed to load chat skills:', result.error)
      }
    } catch (err) {
      console.error('Failed to load chat skills:', err)
    }
  }, [])

  useEffect(() => {
    void loadDefaultModel()
    const cancelIdleLoad = scheduleIdleTask(() => {
      void loadSkills()
    })
    return cancelIdleLoad
  }, [loadDefaultModel, loadSkills])

  useEffect(() => {
    return scheduleIdleTask(() => {
      void refreshToolIndicator()
    }, 1500)
  }, [refreshToolIndicator])

  const openEmbeddedSettings = useCallback((tab: SettingsTab = 'chat') => {
    setSettingsInitialTab(tab)
    setChatView('settings')
    syncSettingsRoute()
  }, [syncSettingsRoute])

  const openAssistantCenter = useCallback(() => {
    setChatView('assistants')
    syncAssistantCenterRoute()
  }, [syncAssistantCenterRoute])

  const openSkillCenter = useCallback(() => {
    setChatView('skill')
    syncSkillCenterRoute()
  }, [syncSkillCenterRoute])

  const openExtensionsItem = useCallback((item: ExtensionsNavItem) => {
    setExtensionsNavItem(item)
    if (item === 'assistants') {
      openAssistantCenter()
      return
    }
    if (item === 'skill') {
      openSkillCenter()
      return
    }
    openEmbeddedSettings(item)
  }, [openAssistantCenter, openSkillCenter, openEmbeddedSettings])

  const extensionsActive = useMemo<ExtensionsNavItem | null>(() => {
    if (chatView === 'assistants') return 'assistants'
    if (chatView === 'skill') return 'skill'
    if (chatView === 'settings' && extensionsNavItem === 'mcp') return 'mcp'
    return null
  }, [chatView, extensionsNavItem])

  const handleSettingsClose = useCallback(() => {
    setChatView('conversation')
    syncConversationRoute(currentConversationIdRef.current)
    void loadSkills()
    void refreshToolIndicator()
    const pending = pendingAfterSettingsCloseRef.current
    pendingAfterSettingsCloseRef.current = null
    pending?.()
  }, [loadSkills, refreshToolIndicator, syncConversationRoute])

  const handleAssistantCenterClose = useCallback(() => {
    setChatView('conversation')
    syncConversationRoute(currentConversationIdRef.current)
  }, [syncConversationRoute])

  const handleSkillCenterClose = useCallback(() => {
    setChatView('conversation')
    syncConversationRoute(currentConversationIdRef.current)
    void loadSkills()
  }, [loadSkills, syncConversationRoute])

  const runAfterLeavingSettings = useCallback((action: () => void) => {
    if (chatView !== 'settings') {
      action()
      return
    }
    if (!settingsRef.current) {
      setChatView('conversation')
      syncConversationRoute(currentConversationIdRef.current)
      action()
      return
    }
    pendingAfterSettingsCloseRef.current = action
    settingsRef.current?.requestClose()
  }, [chatView, syncConversationRoute])

  const handleSettingsChange = useCallback(() => {
    onSettingsChange()
    void loadDefaultModel()
    void loadSkills()
    void refreshToolIndicator()
    setSidebarProfileRefreshKey((key) => key + 1)
  }, [loadDefaultModel, loadSkills, onSettingsChange, refreshToolIndicator])

  const reloadConversation = useCallback(async (conversationId: string, options?: { force?: boolean }) => {
    if (isConversationInFlight(inFlightConversationsRef.current, conversationId) && !options?.force) {
      return
    }
    try {
      const conv = await chatApi.getConversation(conversationId)
      currentConversationIdRef.current = conversationId
      applyConversation(conv)
      restoreStreamingPreview(conversationId)
      setCancellingStream(false)
    } catch (err) {
      console.error('Failed to reload conversation:', err)
      // B2：reload 失败（尤其"对话不存在"）——把 ghost 从乐观列表/in-flight/快照剔除并刷新侧栏。
      dropConversationLocally(conversationId)
      forgetRememberedChatRoute()
      if (currentConversationIdRef.current === conversationId || currentConversationIdRef.current === null) {
        currentConversationIdRef.current = null
        applyConversation(null)
        syncConversationRoute(null)
      }
      refreshSidebar()
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '对话加载失败，已从列表移除')
    }
  }, [applyConversation, dropConversationLocally, refreshSidebar, restoreStreamingPreview, syncConversationRoute])

  const refreshContextStats = useCallback(async (conversationId?: string) => {
    const targetConversationId = conversationId ?? currentConversationIdRef.current
    if (!targetConversationId) {
      setContextState(null)
      setContextError('')
      return
    }
    setContextLoading(true)
    setContextError('')
    try {
      const result = await chatApi.getContextStats(targetConversationId)
      if (currentConversationIdRef.current === targetConversationId) {
        patchContextState(result.contextState)
      }
    } catch (err) {
      if (currentConversationIdRef.current === targetConversationId) {
        setContextError(typeof err === 'string' ? err : (err as Error).message || '上下文统计失败')
      }
    } finally {
      if (currentConversationIdRef.current === targetConversationId) {
        setContextLoading(false)
      }
    }
  }, [patchContextState])

  const handleRefreshContext = useCallback(() => {
    const conversationId = currentConversationIdRef.current
    if (conversationId) void refreshContextStats(conversationId)
  }, [refreshContextStats])

  const handleCompressContext = useCallback(async () => {
    const conversationId = currentConversationIdRef.current
    if (!conversationId || contextCompressing) return
    setContextCompressing(true)
    setContextError('')
    try {
      const result = await chatApi.compressContext(conversationId)
      if (currentConversationIdRef.current === conversationId) {
        applyConversation(result.conversation)
        setContextState(result.contextState)
        refreshSidebar()
      }
    } catch (err) {
      if (currentConversationIdRef.current === conversationId) {
        setContextError(typeof err === 'string' ? err : (err as Error).message || '上下文压缩失败')
      }
    } finally {
      if (currentConversationIdRef.current === conversationId) {
        setContextCompressing(false)
      }
    }
  }, [applyConversation, contextCompressing, refreshSidebar])

  const finishStreamingRun = useCallback(
    async (payload: { reason?: string; conversationId?: string }) => {
      const conversationId = payload.conversationId ?? currentConversationIdRef.current
      if (payload.reason !== 'cancelled') {
        resetLocalCancellation()
      }
      if (payload.reason === 'error' && conversationId) {
        setStreamErrorForConversation(
          conversationId,
          streamErrorsRef.current[conversationId] || '回复生成失败，请稍后重试。',
        )
      }
      if (conversationId && payload.reason !== 'cancelled') {
        if (currentConversationIdRef.current === conversationId) {
          await reloadConversation(conversationId, { force: true })
        }
        refreshSidebar()
      }
      if (conversationId) {
        delete streamSnapshotsRef.current[conversationId]
        delete pendingToolConfirmsRef.current[conversationId]
        delete pendingSessionConsentsRef.current[conversationId]
        syncGeneratingConversationIds()
      }
      if (conversationId && currentConversationIdRef.current === conversationId) {
        setPendingToolConfirm(null)
        setPendingSessionConsent(null)
        clearStreamingPreview()
      }
    },
    [clearStreamingPreview, refreshSidebar, reloadConversation, resetLocalCancellation, setStreamErrorForConversation, syncGeneratingConversationIds],
  )

  const flushPendingStreamDone = useCallback(async (conversationId?: string): Promise<boolean> => {
    if (conversationId) {
      const pending = pendingStreamDoneRef.current[conversationId]
      delete pendingStreamDoneRef.current[conversationId]
      if (!pending) return false
      await pending()
      return true
    }
    const pendingByConversation = pendingStreamDoneRef.current
    pendingStreamDoneRef.current = {}
    let flushed = false
    for (const pending of Object.values(pendingByConversation)) {
      await pending()
      flushed = true
    }
    return flushed
  }, [])

  const finishStreamingRunWithConversation = useCallback((
    conversationId: string,
    conversation: Conversation,
  ) => {
    if (currentConversationIdRef.current === conversationId) {
      applyConversation(conversation)
      setPendingToolConfirm(null)
      setPendingSessionConsent(null)
    }
    delete streamSnapshotsRef.current[conversationId]
    delete pendingToolConfirmsRef.current[conversationId]
    delete pendingSessionConsentsRef.current[conversationId]
    syncGeneratingConversationIds()
    if (currentConversationIdRef.current === conversationId) {
      clearStreamingPreview()
    }
  }, [applyConversation, clearStreamingPreview, syncGeneratingConversationIds])

  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined

    const setupListener = async () => {
      unlisten = await api.onChatStream((payload) => {
        if (cancelled) return
        if (isLocallyCancelledPayload(
          payload,
          locallyCancelledConversationIdRef.current,
          locallyCancelledRunIdRef.current,
        )) {
          return
        }
        if (!streamSnapshotsRef.current[payload.conversationId]) {
          if (!isConversationInFlight(inFlightConversationsRef.current, payload.conversationId)) {
            if (payload.done) {
              void finishStreamingRun(payload)
            }
            return
          }
        }
        const snapshot = ensureStreamSnapshot(payload.conversationId)
        if (payload.runId) {
          if (snapshot.runId && snapshot.runId !== payload.runId) return
          snapshot.runId = payload.runId
        }
        const segment = streamPayloadToSegment(payload)
        if (segment) {
          snapshot.segments = upsertStreamSegment(
            snapshot.segments,
            segment,
            segment.kind === 'reasoning' ? payload.reasoningDelta ?? '' : payload.delta ?? '',
          )
        }
        if (payload.reasoningDelta) {
          const now = Date.now()
          if (snapshot.reasoningStartedAt == null) {
            snapshot.reasoningStartedAt = now
          }
          if (segment?.kind === 'reasoning') {
            const segmentStartedAt = snapshot.reasoningStartedAtBySegmentId[segment.id] ?? now
            snapshot.reasoningStartedAtBySegmentId[segment.id] = segmentStartedAt
            updateReasoningSegmentDuration(snapshot, segment.id, now)
          }
          snapshot.streaming = true
          snapshot.reasoningStreaming = true
          snapshot.reasoning += payload.reasoningDelta
          snapshot.reasoningDurationMs = Math.max(
            snapshot.reasoningDurationMs ?? 0,
            now - snapshot.reasoningStartedAt,
          )
        }
        if (payload.delta) {
          if (snapshot.reasoningStreaming && snapshot.reasoningStartedAt != null) {
            snapshot.reasoningDurationMs = Math.max(
              snapshot.reasoningDurationMs ?? 0,
              Date.now() - snapshot.reasoningStartedAt,
            )
          }
          if (segment?.kind === 'text') {
            const activeReasoningSegment = findReasoningSegmentForText(snapshot.segments, segment)
            if (activeReasoningSegment) {
              updateReasoningSegmentDuration(snapshot, activeReasoningSegment.id)
            }
          }
          snapshot.streaming = true
          snapshot.reasoningStreaming = false
          snapshot.content += payload.delta
        }
        syncGeneratingConversationIds()
        showStreamSnapshotIfCurrent(payload.conversationId, snapshot)
        if (payload.done) {
          if (snapshot.reasoningStartedAt != null && snapshot.reasoningStreaming) {
            snapshot.reasoningDurationMs = Math.max(
              snapshot.reasoningDurationMs ?? 0,
              Date.now() - snapshot.reasoningStartedAt,
            )
            const activeReasoningSegment = [...snapshot.segments]
              .reverse()
              .find((item) => item.kind === 'reasoning')
            if (activeReasoningSegment) {
              updateReasoningSegmentDuration(snapshot, activeReasoningSegment.id)
            }
          }
          // done：立即 flush 最后一帧，别让合帧吞掉收尾内容。
          showStreamSnapshotIfCurrent(payload.conversationId, snapshot, true)
          // invoke 未完成前不要 reload；延后到 flushPendingStreamDone，避免与 send 写盘竞态。
          if (isConversationInFlight(inFlightConversationsRef.current, payload.conversationId)) {
            pendingStreamDoneRef.current[payload.conversationId] = () => finishStreamingRun(payload)
            return
          }
          void finishStreamingRun(payload)
        }
      })
      if (cancelled) {
        unlisten()
      }
    }

    setupListener()
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [ensureStreamSnapshot, finishStreamingRun, showStreamSnapshotIfCurrent, syncGeneratingConversationIds])

  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined

    const setupListener = async () => {
      unlisten = await api.onChatContext((payload) => {
        if (cancelled) return
        const currentConversationId = currentConversationIdRef.current
        if (!currentConversationId || payload.conversationId !== currentConversationId) {
          return
        }
        patchContextState(payload.contextState)
        setContextError('')
      })
      if (cancelled) {
        unlisten()
      }
    }

    setupListener()
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [patchContextState])

  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined

    const setupListener = async () => {
      unlisten = await api.onChatTodo((payload) => {
        if (cancelled) return
        const currentConversationId = currentConversationIdRef.current
        if (!currentConversationId || payload.conversationId !== currentConversationId) {
          return
        }
        patchAgentTodoState(payload.todoState)
      })
      if (cancelled) {
        unlisten()
      }
    }

    setupListener()
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [patchAgentTodoState])

  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined

    const setupListener = async () => {
      unlisten = await api.onChatPlan((payload) => {
        if (cancelled) return
        const currentConversationId = currentConversationIdRef.current
        if (!currentConversationId || payload.conversationId !== currentConversationId) {
          return
        }
        patchAgentPlanState(payload.planState)
      })
      if (cancelled) {
        unlisten()
      }
    }

    setupListener()
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [patchAgentPlanState])

  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined

    const setupListener = async () => {
      unlisten = await api.onChatTool((payload) => {
        if (cancelled) return
        if (isLocallyCancelledPayload(
          payload,
          locallyCancelledConversationIdRef.current,
          locallyCancelledRunIdRef.current,
        )) {
          return
        }
        // 忽略 invoke 结束后的迟到 tool 事件，否则会重新 setStreaming(true) 卡死输入栏。
        if (!isConversationInFlight(inFlightConversationsRef.current, payload.conversationId)) return
        const snapshot = ensureStreamSnapshot(payload.conversationId)
        if (payload.runId) {
          if (snapshot.runId && snapshot.runId !== payload.runId) return
          snapshot.runId = payload.runId
        }
        const record = toolEventToRecord(payload)
        snapshot.streaming = true
        snapshot.reasoningStreaming = false
        const index = snapshot.toolCalls.findIndex((item) => item.id === record.id)
        snapshot.toolCalls = index < 0
          ? [...snapshot.toolCalls, record]
          : snapshot.toolCalls.map((item, i) => (i === index ? { ...item, ...record } : item))
        snapshot.segments = upsertToolStreamSegment(snapshot.segments, record)
        syncGeneratingConversationIds()
        showStreamSnapshotIfCurrent(payload.conversationId, snapshot)
      })
      if (cancelled) {
        unlisten()
      }
    }

    setupListener()
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [ensureStreamSnapshot, showStreamSnapshotIfCurrent, syncGeneratingConversationIds])

  // Live nested sub-agent progress (P3): merge onto the parent tool card's
  // structuredContent.subagentProgress, addressed by parentToolCallId.
  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined

    const setupListener = async () => {
      unlisten = await api.onChatSubagent((payload) => {
        if (cancelled) return
        if (!isConversationInFlight(inFlightConversationsRef.current, payload.parentConversationId)) return
        const snapshot = ensureStreamSnapshot(payload.parentConversationId)
        if (payload.parentRunId && snapshot.runId && snapshot.runId !== payload.parentRunId) return
        const index = snapshot.toolCalls.findIndex((item) => item.id === payload.parentToolCallId)
        if (index < 0) return
        const progress = {
          taskId: payload.taskId,
          name: payload.name,
          depth: payload.depth,
          status: payload.status,
          preview: payload.preview ?? '',
          steps: payload.steps ?? [],
        }
        snapshot.toolCalls = snapshot.toolCalls.map((item, i) => {
          if (i !== index) return item
          const existing =
            item.structuredContent && typeof item.structuredContent === 'object'
              ? (item.structuredContent as Record<string, unknown>)
              : {}
          return { ...item, structuredContent: { ...existing, subagentProgress: progress } }
        })
        showStreamSnapshotIfCurrent(payload.parentConversationId, snapshot)
      })
      if (cancelled) {
        unlisten()
      }
    }

    setupListener()
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [ensureStreamSnapshot, showStreamSnapshotIfCurrent])

  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined

    const setupListener = async () => {
      unlisten = await api.onChatUserPrompt((payload) => {
        if (cancelled) return
        if (isLocallyCancelledPayload(
          payload,
          locallyCancelledConversationIdRef.current,
          locallyCancelledRunIdRef.current,
        )) {
          return
        }
        if (!isConversationInFlight(inFlightConversationsRef.current, payload.conversationId)) return
        const snapshot = ensureStreamSnapshot(payload.conversationId)
        if (payload.runId) {
          if (snapshot.runId && snapshot.runId !== payload.runId) return
          snapshot.runId = payload.runId
        }
        const record = userPromptEventToRecord(payload)
        snapshot.streaming = true
        snapshot.reasoningStreaming = false
        const index = snapshot.toolCalls.findIndex((item) => item.id === record.id)
        snapshot.toolCalls = index < 0
          ? [...snapshot.toolCalls, record]
          : snapshot.toolCalls.map((item, i) => (i === index ? { ...item, ...record } : item))
        syncGeneratingConversationIds()
        showStreamSnapshotIfCurrent(payload.conversationId, snapshot)
      })
      if (cancelled) {
        unlisten()
      }
    }

    setupListener()
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [ensureStreamSnapshot, showStreamSnapshotIfCurrent, syncGeneratingConversationIds])

  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined

    const setupListener = async () => {
      unlisten = await api.onChatToolConfirm((payload) => {
        if (cancelled) return
        pendingToolConfirmsRef.current[payload.conversationId] = payload
        syncGeneratingConversationIds()
        if (currentConversationIdRef.current === payload.conversationId) {
          setPendingToolConfirm(payload)
        }
      })
      if (cancelled) {
        unlisten()
      }
    }

    setupListener()
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [syncGeneratingConversationIds])

  const resolvePendingToolConfirm = useCallback((approved: boolean) => {
    if (!pendingToolConfirm) return
    delete pendingToolConfirmsRef.current[pendingToolConfirm.conversationId]
    syncGeneratingConversationIds()
    void api.chatConfirmToolCall(pendingToolConfirm.toolCallId, approved)
    setPendingToolConfirm(null)
  }, [pendingToolConfirm, syncGeneratingConversationIds])

  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined

    const setupListener = async () => {
      unlisten = await api.onChatSessionConsent((payload) => {
        if (cancelled) return
        pendingSessionConsentsRef.current[payload.conversationId] = payload
        if (currentConversationIdRef.current === payload.conversationId) {
          setPendingSessionConsent(payload)
        }
      })
      if (cancelled) {
        unlisten()
      }
    }

    setupListener()
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [])

  const resolvePendingSessionConsent = useCallback((granted: boolean) => {
    if (!pendingSessionConsent) return
    delete pendingSessionConsentsRef.current[pendingSessionConsent.conversationId]
    void api.chatRespondSessionConsent(pendingSessionConsent.conversationId, granted)
    setPendingSessionConsent(null)
  }, [pendingSessionConsent])

  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined

    const setupListener = async () => {
      unlisten = await api.onChatRunPython((payload) => {
        if (cancelled) return
        void (async () => {
          try {
            const { runPythonInSandbox } = await import('./pyodideRunner')
            const outcome = await runPythonInSandbox(payload.code, payload.timeoutMs, payload.files)
            await api.chatPythonComplete(
              payload.runId,
              outcome.content,
              outcome.isError,
              outcome.artifacts,
            )
          } catch (err) {
            const message = err instanceof Error
              ? err.message || err.stack || err.name
              : String(err)
            await api.chatPythonComplete(
              payload.runId,
              `Python 沙盒调用失败：${message || 'Unknown error'}。不要使用 run_command/pip 安装或修改本机 Python 环境来绕过沙盒；请直接基于已有数据回答，除非用户明确要求修改本机环境。`,
              true,
              [],
            )
          }
        })()
      })
      if (cancelled) {
        unlisten()
      }
    }

    setupListener()
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [])

  useEffect(() => {
    currentConversationIdRef.current = currentConversation?.id ?? null
  }, [currentConversation?.id])

  useEffect(() => {
    if (!currentConversation?.id || chatView !== 'conversation') {
      setContextLoading(false)
      return
    }
    void refreshContextStats(currentConversation.id)
  }, [chatView, currentConversation?.id, activeModel, effectiveSkillId, refreshContextStats])

  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined

    api.onOpenSettings(() => {
      if (cancelled) return
      const path = hashPath()
      if (!path.startsWith('chat')) return
      openEmbeddedSettings()
    }).then((dispose) => {
      if (cancelled) {
        dispose()
      } else {
        unlisten = dispose
      }
    })
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [openEmbeddedSettings])

  useEffect(() => {
    const loadFromRoute = () => {
      const path = hashPath()
      if (isChatSettingsPath(path)) {
        setChatView('settings')
        return
      }
      if (isChatAssistantCenterPath(path)) {
        setChatView('assistants')
        return
      }
      if (isChatSkillCenterPath(path)) {
        setChatView('skill')
        return
      }
      setChatView('conversation')
      const conversationId = getRouteConversationId()
      if (!conversationId) {
        currentConversationIdRef.current = null
        applyConversation(null)
        restoreStreamingPreview(null)
        return
      }
      void reloadConversation(conversationId, { force: true })
    }
    loadFromRoute()
    window.addEventListener('hashchange', loadFromRoute)
    return () => window.removeEventListener('hashchange', loadFromRoute)
  }, [applyConversation, getRouteConversationId, reloadConversation, restoreStreamingPreview])

  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined

    api.onChatOpenConversation((payload) => {
      if (cancelled || !payload.conversationId) return
      setChatView('conversation')
      syncConversationRoute(payload.conversationId)
      if (payload.reload !== false) {
        void reloadConversation(payload.conversationId, { force: true })
      }
      refreshSidebar()
    }).then((dispose) => {
      if (cancelled) dispose()
      else unlisten = dispose
    }).catch(err => console.error(err))

    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [refreshSidebar, reloadConversation, syncConversationRoute])

  const handleSelectConversation = useCallback(async (conversationId: string) => {
    setAssistantStreamStatsByMessageId({})
    try {
      const conv = await chatApi.getConversation(conversationId)
      currentConversationIdRef.current = conversationId
      applyConversation(conv)
      restoreStreamingPreview(conversationId)
      syncConversationRoute(conversationId)
      setStreamError('')
    } catch (err) {
      console.error('Failed to load conversation:', err)
      // B2：点开一个不存在/加载失败的 ghost——从乐观列表 + in-flight + 快照剔除，
      // 清空当前会话并刷新侧栏，让 ghost 自动消失而不是卡住。
      dropConversationLocally(conversationId)
      if (currentConversationIdRef.current === conversationId) {
        currentConversationIdRef.current = null
        applyConversation(null)
      }
      forgetRememberedChatRoute()
      syncConversationRoute(null)
      refreshSidebar()
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '对话加载失败，已从列表移除')
    }
  }, [applyConversation, dropConversationLocally, refreshSidebar, restoreStreamingPreview, syncConversationRoute])

  const handleNewConversation = useCallback(async () => {
    setSelectedProject(null)
    setAssistantStreamStatsByMessageId({})
    setDraftProviderId(activeProviderId)
    setDraftModel(activeModel)
    setDraftAgentRuntime(activeAgentRuntime)
    currentConversationIdRef.current = null
    forgetRememberedChatRoute()
    applyConversation(null)
    restoreStreamingPreview(null)
    syncConversationRoute(null)
    setPendingUserMessage(null)
    setPendingUserMessageConversationId(null)
    setContextError('')
    setContextLoading(false)
    setContextCompressing(false)
    setStreamError('')
  }, [
    activeAgentRuntime,
    activeModel,
    activeProviderId,
    applyConversation,
    restoreStreamingPreview,
    syncConversationRoute,
  ])

  const handleClearChat = useCallback(async () => {
    const conversationId = currentConversationIdRef.current
    if (conversationId && isConversationBusy(
      conversationId,
      inFlightConversationsRef.current,
      streamSnapshotsRef.current,
    )) {
      setStreamErrorForConversation(conversationId, '请先停止当前回复，再清空对话。')
      return
    }

    if (!conversationId) {
      setAssistantStreamStatsByMessageId({})
      setPendingUserMessage(null)
      setPendingUserMessageConversationId(null)
      setStreamError('')
      return
    }

    if (!window.confirm('Clear this chat? This will delete the current conversation history.')) {
      return
    }

    try {
      await chatApi.deleteConversation(conversationId)
      if (isConversationInFlight(inFlightConversationsRef.current, conversationId)) {
        await chatApi.cancelStream(conversationId)
      }
      delete streamSnapshotsRef.current[conversationId]
      delete pendingToolConfirmsRef.current[conversationId]
      delete pendingSessionConsentsRef.current[conversationId]
      delete streamErrorsRef.current[conversationId]
      clearConversationInFlight(conversationId)
      forgetRememberedChatRoute()
      currentConversationIdRef.current = null
      setAssistantStreamStatsByMessageId({})
      setPendingUserMessage(null)
      setPendingUserMessageConversationId(null)
      setContextState(null)
      setContextError('')
      applyConversation(null)
      restoreStreamingPreview(null)
      syncConversationRoute(null)
      refreshSidebar()
      setStreamError('')
    } catch (err) {
      console.error('Failed to clear chat:', err)
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '清空对话失败')
    }
  }, [applyConversation, clearConversationInFlight, refreshSidebar, restoreStreamingPreview, setStreamErrorForConversation, syncConversationRoute])

  const handleStartAssistantChat = useCallback(async (assistant: ChatAssistant) => {
    setAssistantStreamStatsByMessageId({})
    try {
      const assistantProviderId = assistant.provider_id ?? assistant.providerId ?? ''
      const assistantModel = assistant.model ?? ''
      const conv = await chatApi.createConversation(
        assistantProviderId || activeProviderId || undefined,
        assistantModel || activeModel || undefined,
        selectedProject?.name,
        selectedProject?.id ?? null,
        assistant.id,
      )
      currentConversationIdRef.current = conv.id
      applyConversation(conv)
      restoreStreamingPreview(conv.id)
      syncConversationRoute(conv.id)
      refreshSidebar()
      setStreamError('')
    } catch (err) {
      console.error('Failed to start assistant conversation:', err)
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '创建助手对话失败')
    }
  }, [activeModel, activeProviderId, applyConversation, refreshSidebar, restoreStreamingPreview, selectedProject?.id, selectedProject?.name, syncConversationRoute])

  const handleStartBuilderChat = useCallback(async () => {
    setAssistantStreamStatsByMessageId({})
    try {
      const conv = await chatApi.createBuilderConversation(
        activeProviderId || undefined,
        activeModel || undefined,
        selectedProject?.id ?? null,
      )
      currentConversationIdRef.current = conv.id
      applyConversation(conv)
      restoreStreamingPreview(conv.id)
      syncConversationRoute(conv.id)
      refreshSidebar()
      setStreamError('')
    } catch (err) {
      console.error('Failed to start builder conversation:', err)
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '创建搭建对话失败')
    }
  }, [activeModel, activeProviderId, applyConversation, refreshSidebar, restoreStreamingPreview, selectedProject?.id, syncConversationRoute])

  const handleApplyAssistant = useCallback(async (assistantId: string | null) => {
    if (!currentConversation) return
    try {
      const updated = await chatApi.updateConversation(currentConversation.id, {
        assistantId: assistantId ?? '',
      })
      applyConversation(updated)
      refreshSidebar()
      if (assistantId) void refreshContextStats(updated.id)
    } catch (err) {
      console.error('Failed to update conversation assistant:', err)
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '助手切换失败')
    }
  }, [applyConversation, currentConversation, refreshContextStats, refreshSidebar])

  const ensureConversationForAgentPlan = useCallback(async () => {
    if (currentConversation) return currentConversation
    let conversation = await chatApi.createConversation(
      activeProviderId || undefined,
      activeModel || undefined,
      selectedProject?.name,
      selectedProject?.id ?? null,
    )
    if (!agentRuntimesEqual(normalizeAgentRuntime(conversation), draftAgentRuntime)) {
      conversation = await chatApi.setAgentRuntime(conversation.id, draftAgentRuntime)
    }
    currentConversationIdRef.current = conversation.id
    applyConversation(conversation)
    syncConversationRoute(conversation.id)
    refreshSidebar()
    return conversation
  }, [activeModel, activeProviderId, applyConversation, currentConversation, draftAgentRuntime, refreshSidebar, selectedProject?.id, selectedProject?.name, syncConversationRoute])

  const handleAgentPlanModeChange = useCallback(async (mode: AgentPlanMode) => {
    try {
      const conversation = await ensureConversationForAgentPlan()
      const updated = await chatApi.setAgentPlanMode(conversation.id, mode)
      applyConversation(updated)
      void refreshContextStats(updated.id)
      refreshSidebar()
    } catch (err) {
      console.error('Failed to update agent plan mode:', err)
      setStreamError(typeof err === 'string' ? err : (err as Error).message || 'Plan 模式切换失败')
    }
  }, [applyConversation, ensureConversationForAgentPlan, refreshContextStats, refreshSidebar])

  const handleSelectProject = useCallback((project: ChatProject | null) => {
    setSelectedProject(project)
    setAssistantStreamStatsByMessageId({})
    setPendingUserMessage(null)
    setPendingUserMessageConversationId(null)
    currentConversationIdRef.current = null
    applyConversation(null)
    restoreStreamingPreview(null)
    syncConversationRoute(null)
    setStreamError('')
  }, [applyConversation, restoreStreamingPreview, syncConversationRoute])

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (chatView === 'settings') return
      const mod = e.metaKey || e.ctrlKey
      if (!mod) return
      if (e.key === 'n' || e.key === 'N') {
        e.preventDefault()
        void handleNewConversation()
      }
      if (e.key === 'k' || e.key === 'K') {
        e.preventDefault()
        setSearchOpen((open) => !open)
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [chatView, handleNewConversation])

  const applyAssistantStreamStats = useCallback((updatedConv: Conversation) => {
    const lastAssistant = [...updatedConv.messages]
      .reverse()
      .find((message) => message.role === 'assistant')
    if (!lastAssistant || !streamStartedAtRef.current) return

    const elapsedSec = Math.max((Date.now() - streamStartedAtRef.current) / 1000, 0.1)
    const streamedText = `${streamingContentRef.current}${streamingReasoningRef.current ? `\n${streamingReasoningRef.current}` : ''}`
    const tokenEstimate = estimateTokens(
      streamedText.trim().length > 0
        ? streamedText
        : `${lastAssistant.content}${lastAssistant.reasoning ? `\n${lastAssistant.reasoning}` : ''}`,
    )
    const stats: AssistantStreamStats = {
      messageId: lastAssistant.id,
      tokensPerSec: tokenEstimate / elapsedSec,
      reasoningDurationMs: streamSnapshotsRef.current[updatedConv.id]?.reasoningDurationMs ?? null,
      reasoningDurationMsBySegmentId: streamSnapshotsRef.current[updatedConv.id]?.reasoningDurationMsBySegmentId ?? {},
    }
    setAssistantStreamStatsByMessageId((prev) => ({
      ...prev,
      [lastAssistant.id]: stats,
    }))
  }, [])

  const handleSendMessage = useCallback(async (
    content: string,
    attachments: PendingAttachment[] = [],
    options: SendMessageOptions = {},
  ) => {
    const trimmed = content.trim()
    if (!trimmed && attachments.length === 0) return false
    if (!options.forceNewConversation && sendDisabledReason) {
      const targetId = currentConversationIdRef.current
      if (targetId) {
        setStreamErrorForConversation(targetId, sendDisabledReason)
      } else {
        setStreamError(sendDisabledReason)
      }
      return false
    }

    let conversation = options.conversationOverride
      ?? (options.forceNewConversation ? null : currentConversation)
    if (
      conversation
      && !options.conversationOverride
      && isPlainBlankConversation(conversation)
      && !conversationUsesModel(conversation, activeProviderId, activeModel)
    ) {
      conversation = null
    }
    if (!conversation) {
      try {
        conversation = await chatApi.createConversation(
          activeProviderId || undefined,
          activeModel || undefined,
          selectedProject?.name,
          selectedProject?.id ?? null,
        )
        currentConversationIdRef.current = conversation.id
        applyConversation(conversation)
        syncConversationRoute(conversation.id)
      } catch (err) {
        console.error('Failed to create conversation before send:', err)
        setStreamError(typeof err === 'string' ? err : (err as Error).message || '创建对话失败')
        return false
      }
    }

    if (!agentRuntimesEqual(normalizeAgentRuntime(conversation), draftAgentRuntime)) {
      try {
        conversation = await chatApi.setAgentRuntime(conversation.id, draftAgentRuntime)
        applyConversation(conversation)
      } catch (err) {
        console.error('Failed to apply agent runtime before send:', err)
        setStreamError(typeof err === 'string' ? err : (err as Error).message || 'Agent 切换失败')
        return false
      }
    }

    const conversationId = conversation.id
    if (isConversationInFlight(inFlightConversationsRef.current, conversationId)) {
      setStreamErrorForConversation(conversationId, '该对话正在生成中，请稍后再试')
      return false
    }
    setOptimisticSidebarConversations((items) => [
      optimisticConversationListItem(conversation, trimmed),
      ...items.filter((item) => item.id !== conversationId),
    ])

    const pendingUserId = `pending-user-${Date.now()}`
    const optimisticUserMessage: ChatMessage = {
      id: pendingUserId,
      role: 'user',
      content: trimmed,
      attachments: attachments.map((attachment) => ({
        id: attachment.id,
        type: attachment.type,
        name: attachment.name,
        path: attachment.path,
      })),
      timestamp: Math.floor(Date.now() / 1000),
    }

    resetLocalCancellation()
    const startedAt = Date.now()
    const snapshot = ensureStreamSnapshot(conversationId)
    snapshot.streaming = true
    snapshot.content = ''
    snapshot.reasoning = ''
    snapshot.reasoningStreaming = false
    snapshot.toolCalls = []
    snapshot.segments = []
    snapshot.startedAt = startedAt
    snapshot.reasoningStartedAt = null
    snapshot.reasoningDurationMs = null
    snapshot.reasoningStartedAtBySegmentId = {}
    snapshot.reasoningDurationMsBySegmentId = {}
    snapshot.runId = null
    syncGeneratingConversationIds()

    if (currentConversationIdRef.current === conversationId) {
      setStreaming(true)
      setStreamFrozen(false)
      setCancellingStream(false)
      setStreamingContent('')
      setStreamingReasoning('')
      setStreamingReasoningDurationMs(null)
      setStreamingReasoningDurationMsBySegmentId({})
      setReasoningStreaming(false)
      setStreamingToolCalls([])
      setStreamingSegments([])
      setStreamErrorForConversation(conversationId, '')
      activeRunIdRef.current = null
      streamStartedAtRef.current = startedAt
      streamingContentRef.current = ''
      streamingReasoningRef.current = ''
      setPendingUserMessage(optimisticUserMessage)
      setPendingUserMessageConversationId(conversationId)
    }

    markConversationInFlight(conversationId)
    const attachmentSkillId = options.forceNewConversation
      ? inferSingleAttachmentSkillId(attachments, enabledSkills)
      : effectiveSkillId ?? inferSingleAttachmentSkillId(attachments, enabledSkills)

    let persistedConversation: Conversation | null = null
    try {
      const updatedConv = await chatApi.sendMessage(
        conversationId,
        trimmed,
        attachments,
        attachmentSkillId,
      )
      persistedConversation = updatedConv
      if (currentConversationIdRef.current === conversationId) {
        applyAssistantStreamStats(updatedConv)
        setPendingUserMessage(null)
        setPendingUserMessageConversationId(null)
        setOptimisticSidebarConversations((items) => items.filter((item) => item.id !== conversationId))
        applyConversation(updatedConv)
        refreshSidebar()
        if (!locallyCancelledConversationIdRef.current) {
          resetLocalCancellation()
        }
      } else {
        refreshSidebar()
      }
    } catch (err) {
      console.error('Failed to send message:', err)
      if (currentConversationIdRef.current === conversationId) {
        setPendingUserMessage(null)
        setPendingUserMessageConversationId(null)
      }
      setOptimisticSidebarConversations((items) => items.filter((item) => item.id !== conversationId))
      clearStreamSnapshot(conversationId)
      const message = typeof err === 'string' ? err : (err as Error).message || '发送失败'
      setStreamErrorForConversation(conversationId, message)
    } finally {
      clearConversationInFlight(conversationId)
      if (persistedConversation) {
        // invoke 已返回持久化后的完整对话且上面已 applyConversation。
        // 丢弃被延后的 finishStreamingRun(它会再次全量 reloadConversation),避免每轮随历史线性变慢。
        delete pendingStreamDoneRef.current[conversationId]
        finishStreamingRunWithConversation(conversationId, persistedConversation)
      } else if (!(await flushPendingStreamDone(conversationId))) {
        clearStreamSnapshot(conversationId)
      }
    }
    return true
  }, [
    activeModel,
    activeProviderId,
    applyAssistantStreamStats,
    applyConversation,
    clearConversationInFlight,
    clearStreamSnapshot,
    currentConversation,
    draftAgentRuntime,
    effectiveSkillId,
    enabledSkills,
    ensureStreamSnapshot,
    finishStreamingRunWithConversation,
    flushPendingStreamDone,
    markConversationInFlight,
    refreshSidebar,
    resetLocalCancellation,
    selectedProject?.id,
    selectedProject?.name,
    sendDisabledReason,
    setStreamErrorForConversation,
    syncConversationRoute,
    syncGeneratingConversationIds,
  ])

  // 用 ref 持有最新 handleSendMessage，使下方的 drainExternalSends 保持稳定身份，
  // 避免其依赖抖动导致订阅 effect 反复 cleanup/重订阅（重订阅缝隙会丢掉外部发送事件）。
  const handleSendMessageRef = useRef(handleSendMessage)
  handleSendMessageRef.current = handleSendMessage

  // 历史预置（Lens「在 AI 客户端继续」交接）：用最新 reactive 值（provider/model/project）创建带历史的新会话。
  // 同 handleSendMessageRef 思路用 ref 持有，保持 drainExternalSends 稳定身份。
  const importExternalConversation = useCallback(async (
    messages: { role: string; content: string }[],
    attachmentPaths: string[],
  ): Promise<boolean> => {
    try {
      const conversation = await chatApi.importExternalConversation(
        messages,
        attachmentPaths,
        activeProviderId || undefined,
        activeModel || undefined,
        selectedProject?.id ?? null,
      )
      currentConversationIdRef.current = conversation.id
      applyConversation(conversation)
      syncConversationRoute(conversation.id)
      refreshSidebar()
      return true
    } catch (err) {
      console.error('Failed to import external conversation:', err)
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '导入对话失败')
      return false
    }
  }, [activeModel, activeProviderId, applyConversation, refreshSidebar, selectedProject?.id, syncConversationRoute])
  const importExternalConversationRef = useRef(importExternalConversation)
  importExternalConversationRef.current = importExternalConversation

  const handleExecuteAgentPlan = useCallback(async () => {
    const conversation = currentConversation
    if (!conversation) return
    const planText = (conversation.agent_plan_state?.plan ?? conversation.agentPlanState?.plan ?? '').trim()
    if (!planText) return
    if (isConversationInFlight(inFlightConversationsRef.current, conversation.id)) {
      setStreamErrorForConversation(conversation.id, '该对话正在生成中，请稍后再试')
      return
    }

    try {
      const updated = await chatApi.executeAgentPlan(conversation.id)
      applyConversation(updated)
      refreshSidebar()
      void refreshContextStats(updated.id)
      void handleSendMessage('按刚才的计划开始执行。', [], { conversationOverride: updated })
    } catch (err) {
      console.error('Failed to execute agent plan:', err)
      setStreamErrorForConversation(
        conversation.id,
        typeof err === 'string' ? err : (err as Error).message || '执行计划失败',
      )
    }
  }, [
    applyConversation,
    currentConversation,
    handleSendMessage,
    refreshContextStats,
    refreshSidebar,
    setStreamErrorForConversation,
  ])

  const drainExternalSends = useCallback(async () => {
    if (externalSendDrainProcessingRef.current) {
      externalSendDrainRequestedRef.current = true
      return
    }

    externalSendDrainProcessingRef.current = true
    try {
      do {
        externalSendDrainRequestedRef.current = false

        const result = await api.chatTakeExternalSends()
        if (!result.success) {
          const error = 'error' in result && typeof result.error === 'string'
            ? result.error
            : ''
          throw new Error(error || 'Failed to take external Chat messages')
        }
        const requests = result.requests ?? []
        if (requests.length > 0) {
          externalSendQueueRef.current.push(...requests)
        }

        const request = externalSendQueueRef.current[0]
        if (!request) continue
        setChatView('conversation')
        const attachmentPaths = (request.attachments ?? [])
          .map((attachment) => attachment.path)
          .filter((path): path is string => !!path)

        // 历史预置分支：把 Lens 完整多轮历史 + 截图搬成一个新会话（不发消息、不触发回复），落地末尾可续聊。
        if (request.messages && request.messages.length > 0) {
          await importExternalConversationRef.current(request.messages, attachmentPaths)
          externalSendQueueRef.current.shift()
          continue
        }

        const attachments = (request.attachments ?? [])
          .filter((attachment) => attachment.path)
          .map<PendingAttachment>((attachment, index) => ({
            id: attachment.id || `external-${request.id}-${index}`,
            type: attachment.type === 'file' ? 'file' : 'image',
            name: attachment.name || (attachment.type === 'file' ? 'Attachment' : 'Image'),
            path: attachment.path,
          }))
        const accepted = await handleSendMessageRef.current(
          request.content ?? '',
          attachments,
          { forceNewConversation: true },
        )
        if (accepted) {
          externalSendQueueRef.current.shift()
        } else {
          externalSendDrainRequestedRef.current = true
          break
        }
      } while (externalSendDrainRequestedRef.current || externalSendQueueRef.current.length > 0)
    } catch (err) {
      console.error('Failed to process external Chat message:', err)
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '外部消息发送失败')
    } finally {
      externalSendDrainProcessingRef.current = false
      if (externalSendDrainRequestedRef.current) {
        window.setTimeout(() => {
          void drainExternalSends()
        }, 0)
      }
    }
  }, [])

  useEffect(() => {
    let cancelled = false
    const disposers: Array<() => void> = []
    const register = (p: Promise<() => void>) => {
      p.then((dispose) => {
        if (cancelled) dispose()
        else disposers.push(dispose)
      }).catch((err) => console.error(err))
    }

    // 外部发送（如 Lens 交接）的投递不依赖某个一次性事件的时序：
    // 任意可靠时机都主动从后端取走 pending（chat_take_external_sends 幂等，取空即 no-op）。
    void drainExternalSends()
    // 1) 后端就绪事件
    register(api.onChatExternalSendReady(() => {
      if (!cancelled) void drainExternalSends()
    }))
    // 2) 窗口获得焦点 —— 覆盖复用窗口被重新唤起、以及冷启动时就绪事件丢失的情况
    register(
      import('@tauri-apps/api/window')
        .then(({ getCurrentWindow }) =>
          getCurrentWindow().onFocusChanged(({ payload: focused }) => {
            if (!cancelled && focused) void drainExternalSends()
          }),
        ),
    )

    return () => {
      cancelled = true
      disposers.forEach((dispose) => dispose())
    }
  }, [drainExternalSends])

  useEffect(() => {
    if (!streaming && externalSendDrainRequestedRef.current) {
      void drainExternalSends()
    }
  }, [drainExternalSends, streaming])

  const handleUpdateMessage = useCallback(
    async (messageId: string, content: string) => {
      if (!currentConversation) return
      try {
        const updated = await chatApi.updateMessage(currentConversation.id, messageId, content)
        applyConversation(updated)
        refreshSidebar()
      } catch (err) {
        console.error('Failed to update message:', err)
        setStreamError(typeof err === 'string' ? err : (err as Error).message || '保存失败')
      }
    },
    [applyConversation, currentConversation, refreshSidebar],
  )

  const handleDeleteMessage = useCallback(
    async (messageId: string) => {
      if (!currentConversation) return
      if (!window.confirm('确定删除这条消息吗？')) return
      try {
        const updated = await chatApi.deleteMessage(currentConversation.id, messageId)
        applyConversation(updated)
        setAssistantStreamStatsByMessageId((prev) => {
          const next = { ...prev }
          delete next[messageId]
          return next
        })
        refreshSidebar()
      } catch (err) {
        console.error('Failed to delete message:', err)
        setStreamError(typeof err === 'string' ? err : (err as Error).message || '删除失败')
      }
    },
    [applyConversation, currentConversation, refreshSidebar],
  )

  const handleRegenerateMessage = useCallback(
    async (messageId: string) => {
      if (!currentConversation) return

      const conversationId = currentConversation.id
      if (isConversationInFlight(inFlightConversationsRef.current, conversationId)) return

      const messageIndex = currentConversation.messages.findIndex(
        (message) => message.id === messageId,
      )
      if (messageIndex < 0) return

      applyConversation({
        ...currentConversation,
        messages: currentConversation.messages.slice(0, messageIndex),
      })
      const removedMessageIds = new Set(
        currentConversation.messages.slice(messageIndex).map((message) => message.id),
      )
      setAssistantStreamStatsByMessageId((prev) => Object.fromEntries(
        Object.entries(prev).filter(([id]) => !removedMessageIds.has(id)),
      ))
      resetLocalCancellation()
      const startedAt = Date.now()
      const snapshot = ensureStreamSnapshot(conversationId)
      snapshot.streaming = true
      snapshot.content = ''
      snapshot.reasoning = ''
      snapshot.reasoningStreaming = false
      snapshot.toolCalls = []
      snapshot.segments = []
      snapshot.startedAt = startedAt
      snapshot.reasoningStartedAt = null
      snapshot.reasoningDurationMs = null
      snapshot.reasoningStartedAtBySegmentId = {}
      snapshot.reasoningDurationMsBySegmentId = {}
      snapshot.runId = null
      syncGeneratingConversationIds()

      if (currentConversationIdRef.current === conversationId) {
        setStreaming(true)
        setStreamFrozen(false)
        setCancellingStream(false)
        setStreamingContent('')
        setStreamingReasoning('')
        setStreamingReasoningDurationMs(null)
        setStreamingReasoningDurationMsBySegmentId({})
        setReasoningStreaming(false)
        setStreamingToolCalls([])
        setStreamingSegments([])
        setStreamErrorForConversation(conversationId, '')
        activeRunIdRef.current = null
        streamStartedAtRef.current = startedAt
        streamingContentRef.current = ''
        streamingReasoningRef.current = ''
      }

      markConversationInFlight(conversationId)
      let persistedConversation: Conversation | null = null
      try {
        const updated = await chatApi.regenerateMessage(conversationId, messageId)
        persistedConversation = updated
        if (currentConversationIdRef.current === conversationId) {
          applyAssistantStreamStats(updated)
          applyConversation(updated)
          refreshSidebar()
        } else {
          refreshSidebar()
        }
      } catch (err) {
        console.error('Failed to regenerate message:', err)
        setStreamErrorForConversation(
          conversationId,
          typeof err === 'string' ? err : (err as Error).message || '重新生成失败',
        )
        clearStreamSnapshot(conversationId)
        if (currentConversationIdRef.current === conversationId) {
          void reloadConversation(conversationId)
        }
      } finally {
        clearConversationInFlight(conversationId)
        if (persistedConversation) {
          // 同 handleSend:已有持久化对话,丢弃延后的全量重拉,直接套用。
          delete pendingStreamDoneRef.current[conversationId]
          finishStreamingRunWithConversation(conversationId, persistedConversation)
        } else if (!(await flushPendingStreamDone(conversationId))) {
          clearStreamSnapshot(conversationId)
        }
      }
    },
    [applyAssistantStreamStats, applyConversation, clearConversationInFlight, clearStreamSnapshot, currentConversation, ensureStreamSnapshot, finishStreamingRunWithConversation, flushPendingStreamDone, markConversationInFlight, refreshSidebar, reloadConversation, resetLocalCancellation, setStreamErrorForConversation, syncGeneratingConversationIds],
  )

  const handleRuntimeChange = async (runtime: AgentRuntimeConfig) => {
    setDraftAgentRuntime(runtime)
    if (!currentConversation) return
    try {
      const updated = await chatApi.setAgentRuntime(currentConversation.id, runtime)
      applyConversation(updated)
    } catch (err) {
      console.error('Failed to change agent runtime:', err)
      setStreamError(typeof err === 'string' ? err : (err as Error).message || 'Agent 切换失败')
    }
  }

  const handleExternalModelChange = async (model: string, reasoning?: string | null) => {
    // Route through handleRuntimeChange so the draft updates even before a conversation exists
    // (the draft is applied when the conversation is created on first send).
    const next: AgentRuntimeConfig = {
      ...activeAgentRuntime,
      kind: 'external',
      externalModel: model,
      externalReasoning: reasoning ?? activeAgentRuntime.externalReasoning ?? null,
    }
    await handleRuntimeChange(next)
  }

  const handleExternalSandboxChange = async (sandbox: string) => {
    const next: AgentRuntimeConfig = {
      ...activeAgentRuntime,
      kind: 'external',
      externalSandbox: sandbox,
    }
    await handleRuntimeChange(next)
  }

  const handleModelChange = async (providerId: string, model: string) => {
    setDraftProviderId(providerId)
    setDraftModel(model)

    if (!currentConversation) return

    try {
      const updatedConv = await chatApi.updateConversation(currentConversation.id, {
        providerId,
        model,
      })
      applyConversation(updatedConv)
    } catch (err) {
      console.error('Failed to change model:', err)
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '模型切换失败')
    }
  }

  const handleCancelStream = useCallback(async () => {
    const conversationId = currentConversationIdRef.current
    if (
      !conversationId
      || cancellingStream
      || !isConversationBusy(
        conversationId,
        inFlightConversationsRef.current,
        streamSnapshotsRef.current,
      )
    ) {
      return
    }

    setCancellingStream(true)
    cancelCurrentRunLocally()
    try {
      await chatApi.cancelStream(conversationId)
    } catch (err) {
      console.error('Failed to cancel chat stream:', err)
      setStreamErrorForConversation(
        conversationId,
        typeof err === 'string' ? err : (err as Error).message || '停止生成失败',
      )
    } finally {
      setCancellingStream(false)
    }
  }, [cancelCurrentRunLocally, cancellingStream, setStreamErrorForConversation])

  const displayMessages = useMemo(() => {
    const stored = currentConversation?.messages ?? []
    if (!pendingUserMessage || pendingUserMessageConversationId !== currentConversation?.id) return stored
    const alreadyStored = stored.some(
      (message) =>
        message.id === pendingUserMessage.id ||
        (message.role === 'user' &&
          message.content === pendingUserMessage.content &&
          message.timestamp >= pendingUserMessage.timestamp - 2),
    )
    return alreadyStored ? stored : [...stored, pendingUserMessage]
  }, [currentConversation?.id, currentConversation?.messages, pendingUserMessage, pendingUserMessageConversationId])

  const hasMessages = displayMessages.length > 0
  const showEmptyHero = chatView === 'conversation' && !hasMessages && !streaming && !streamError
  const emptyHeroGreetingKey = showEmptyHero ? currentConversation?.id : null

  const emptyHeroGreeting = useMemo(
    () => ({
      key: emptyHeroGreetingKey,
      text: pickRandomChatEmptyGreeting(),
    }),
    [emptyHeroGreetingKey],
  )

  const setSidebarCollapsedPersisted = useCallback((collapsed: boolean) => {
    setSidebarCollapsed(collapsed)
    rememberChatSidebarCollapsed(collapsed)
  }, [])

  const handleCollapseSidebar = useCallback(() => {
    setSidebarCollapsedPersisted(true)
  }, [setSidebarCollapsedPersisted])

  useEffect(() => {
    if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) return
    let cancelled = false

    void (async () => {
      const { getCurrentWindow } = await import('@tauri-apps/api/window')
      const { LogicalSize } = await import('@tauri-apps/api/dpi')
      const baseMin = sidebarCollapsed ? CHAT_MIN_SIZE_COLLAPSED : CHAT_MIN_SIZE_EXPANDED
      const min = getChatPlatformWindowSize(baseMin)
      const win = getCurrentWindow()
      await win.setMinSize(new LogicalSize(min.width, min.height))
      if (cancelled) return

      if (!sidebarCollapsed) {
        const scaleFactor = await win.scaleFactor()
        const size = await win.innerSize()
        const logical = size.toLogical(scaleFactor)
        if (logical.width < min.width) {
          const nextHeight = Math.max(logical.height, min.height)
          await win.setSize(new LogicalSize(min.width, nextHeight))
          rememberChatSize(min.width, nextHeight)
        }
      }
    })().catch((err) => {
      console.error('[Chat] Failed to update window min size:', err)
    })

    return () => {
      cancelled = true
    }
  }, [sidebarCollapsed])

  const handleSidebarSelectProject = useCallback((project: ChatProject | null) => {
    runAfterLeavingSettings(() => handleSelectProject(project))
  }, [handleSelectProject, runAfterLeavingSettings])

  const handleSidebarSelectConversation = useCallback((id: string) => {
    runAfterLeavingSettings(() => void handleSelectConversation(id))
  }, [handleSelectConversation, runAfterLeavingSettings])

  const handleSidebarNewConversation = useCallback(() => {
    runAfterLeavingSettings(() => void handleNewConversation())
  }, [handleNewConversation, runAfterLeavingSettings])

  const handleSidebarConversationDeleted = useCallback(() => {
    forgetRememberedChatRoute()
    applyConversation(null)
    syncConversationRoute(null)
    refreshSidebar()
  }, [applyConversation, refreshSidebar, syncConversationRoute])

  const handleSidebarForceDropConversation = useCallback((id: string) => {
    // B3：侧栏删除时强制清掉该会话的 in-flight/快照/乐观项，
    // 使乐观合并不再保留它（删"generating"会话也能立即从侧栏消失）。
    dropConversationLocally(id)
  }, [dropConversationLocally])

  const handleSidebarOpenSettings = useCallback(() => {
    const settingsPanelOpen = chatView === 'settings' && extensionsNavItem === null
    if (settingsPanelOpen) {
      if (settingsRef.current) {
        settingsRef.current.requestClose()
      } else {
        handleSettingsClose()
      }
      return
    }
    setExtensionsNavItem(null)
    openEmbeddedSettings('chat')
  }, [chatView, extensionsNavItem, handleSettingsClose, openEmbeddedSettings])

  const handleSidebarSearchOpenChange = useCallback((open: boolean) => {
    if (open) {
      runAfterLeavingSettings(() => setSearchOpen(true))
      return
    }
    setSearchOpen(false)
  }, [runAfterLeavingSettings])

  return (
    <div
      className={`chat-window-shell${usesNativeTitlebar ? ' chat-window-shell--native-titlebar' : ''}`}
    >
      {!usesNativeTitlebar && <WindowControls />}
      <div className="flex h-full min-h-0 w-full">
        <Sidebar
          currentConversationId={currentConversation?.id}
          generatingConversationIds={generatingConversationIds}
          optimisticConversations={optimisticSidebarConversations}
          selectedProject={selectedProject}
          onSelectProject={handleSidebarSelectProject}
          onSelectConversation={handleSidebarSelectConversation}
          onNewConversation={handleSidebarNewConversation}
          onConversationDeleted={handleSidebarConversationDeleted}
          onForceDropConversation={handleSidebarForceDropConversation}
          onOpenExtensionsItem={openExtensionsItem}
          onOpenSettings={handleSidebarOpenSettings}
          settingsActive={chatView === 'settings' && extensionsNavItem === null}
          extensionsActive={extensionsActive}
          collapsed={sidebarCollapsed}
          onToggleCollapsed={handleCollapseSidebar}
          refreshKey={sidebarRefreshKey}
          profileRefreshKey={sidebarProfileRefreshKey}
          searchOpen={searchOpen}
          onSearchOpenChange={handleSidebarSearchOpenChange}
        />

        {chatView === 'settings' ? (
          <div className="chat-win-titlebar-safe flex min-h-0 min-w-0 flex-1 flex-col">
            <Suspense fallback={<ChatPaneLoading />}>
              <SettingsShell
                ref={settingsRef}
                variant="embedded"
                initialTab={settingsInitialTab}
                reserveTrafficLightSpace={sidebarCollapsed && usesNativeTitlebar}
                onClose={handleSettingsClose}
                onSettingsChange={handleSettingsChange}
              />
            </Suspense>
          </div>
        ) : chatView === 'assistants' ? (
          <div className="chat-win-titlebar-safe flex min-h-0 min-w-0 flex-1 flex-col">
            <Suspense fallback={<ChatPaneLoading />}>
              <AssistantCenter
                skills={enabledSkills}
                currentAssistantId={currentAssistantId}
                onStartAssistantChat={(assistant) => void handleStartAssistantChat(assistant)}
                onStartBuilder={() => void handleStartBuilderChat()}
                onApplyAssistant={currentConversation ? (assistantId) => void handleApplyAssistant(assistantId) : undefined}
                onClose={handleAssistantCenterClose}
              />
            </Suspense>
          </div>
        ) : chatView === 'skill' ? (
          <div className="chat-win-titlebar-safe flex min-h-0 min-w-0 flex-1 flex-col">
            <Suspense fallback={<ChatPaneLoading />}>
              <SkillCenter
                onClose={handleSkillCenterClose}
                onSkillsChanged={() => void loadSkills()}
              />
            </Suspense>
          </div>
        ) : (
          <div className="chat-main-pane relative flex min-w-0 flex-1 flex-col">
            {imageViewerItem ? (
              <ChatImageViewer
                item={imageViewerItem}
                onClose={() => setImageViewerItem(null)}
              />
            ) : (
              <>
                <header
              className={`chat-titlebar-row ${chatTitlebarRowClass} min-w-0 gap-2 ${
                sidebarCollapsed && usesNativeTitlebar
                  ? `${chatTitlebarMacInsetClass} chat-titlebar-row--collapsed-mac`
                  : 'px-6'
              } ${sidebarCollapsed ? 'pr-3' : ''} ${!usesNativeTitlebar ? 'chat-win-titlebar-safe' : ''}`}
              data-tauri-drag-region
            >
              {sidebarCollapsed && (
                <ChatTitlebarActions
                  sidebarExpanded={false}
                  onToggleSidebar={() => setSidebarCollapsedPersisted(false)}
                  onNewConversation={() => {
                    runAfterLeavingSettings(() => void handleNewConversation())
                  }}
                />
              )}
              <div className="flex min-w-0 items-center gap-1">
                <div className="shrink-0" data-tauri-drag-region="false">
                  <RuntimePicker
                    agentRuntime={activeAgentRuntime}
                    onRuntimeChange={(runtime) => void handleRuntimeChange(runtime)}
                  />
                </div>
                <div className="min-w-0 max-w-full shrink" data-tauri-drag-region="false">
                  {usesExternalRuntime ? (
                    <ExternalModelSelector
                      agentRuntime={activeAgentRuntime}
                      onModelChange={(model, reasoning) => void handleExternalModelChange(model, reasoning)}
                    />
                  ) : (
                    <ModelSelector
                      currentProviderId={activeProviderId}
                      currentModel={activeModel}
                      onModelChange={(providerId, model) => void handleModelChange(providerId, model)}
                    />
                  )}
                </div>
                <div className="shrink-0" data-tauri-drag-region="false">
                  <PermissionPicker
                    agentRuntime={activeAgentRuntime}
                    onSandboxChange={(s) => void handleExternalSandboxChange(s)}
                    approvalPolicy={approvalPolicy}
                    onApprovalPolicyChange={handleApprovalPolicyChange}
                  />
                </div>
              </div>
              <div className="min-w-5 flex-1" data-tauri-drag-region />
              <div className="flex min-w-0 shrink items-center justify-end gap-1">
                <div className="shrink-0" data-tauri-drag-region="false">
                  <ContextIndicator
                    contextState={contextState}
                    messageCount={displayMessages.length}
                    loading={contextLoading}
                    compressing={contextCompressing}
                    error={contextError}
                    usesExternalRuntime={usesExternalRuntime}
                    onRefresh={handleRefreshContext}
                    onCompress={() => void handleCompressContext()}
                  />
                </div>
                <AgentTodoIndicator todoState={currentConversation?.agent_todo_state ?? currentConversation?.agentTodoState ?? null} />
              </div>
                </header>

                <div className="flex min-h-0 flex-1 flex-col">
                  {showEmptyHero ? (
                    <div className="chat-empty-hero flex flex-1 flex-col items-center justify-center px-6 pb-16">
                  <ChatDotGridBackground />
                  <div className="chat-empty-hero-stack chat-motion-fade-up relative z-10 w-full max-w-3xl space-y-8">
                    <h2
                      className="chat-empty-hero-title text-center text-[1.75rem] leading-snug tracking-tight text-neutral-900 dark:text-neutral-50 sm:text-[2rem]"
                      aria-label={
                        currentAssistantSnapshot
                          ? currentAssistantSnapshot.name
                          : selectedProject
                            ? `Start in “${selectedProject.name}”`
                            : emptyHeroGreeting.text
                      }
                    >
                      {currentAssistantSnapshot ? (
                        currentAssistantSnapshot.name
                      ) : selectedProject ? (
                        `Start in “${selectedProject.name}”`
                      ) : (
                        <TypewriterText
                          text={emptyHeroGreeting.text}
                          resetKey={emptyHeroGreeting.key}
                          active={showEmptyHero}
                        />
                      )}
                    </h2>
                    <InputBar
                      layout="inline"
                      onSend={(content, attachments) => void handleSendMessage(content, attachments)}
                      disabled={isCurrentConversationBusy()}
                      onCancel={() => void handleCancelStream()}
                      cancelVisible={streaming}
                      cancelling={cancellingStream}
                      onOpenSettings={() => openEmbeddedSettings('chat')}
                      onOpenTools={() => openEmbeddedSettings('skill')}
                      onNewChat={() => void handleNewConversation()}
                      onCompactContext={() => void handleCompressContext()}
                      onClearChat={() => void handleClearChat()}
                      enabledTools={enabledTools}
                      toolsDisabledReason={toolsDisabledReason}
                      toolStatusHint={toolStatusHint}
                      sendDisabledReason={sendDisabledReason}
                      agentPlanState={currentConversation?.agent_plan_state ?? currentConversation?.agentPlanState ?? null}
                      onAgentPlanModeChange={handleAgentPlanModeChange}
                      onExecuteAgentPlan={handleExecuteAgentPlan}
                      enabledSkills={slashSkills}
                      onOpenSkillSettings={openSkillCenter}
                      selectedProject={selectedProject}
                      onSelectProject={handleSidebarSelectProject}
                      showProjectEntry
                      currentAssistant={currentAssistantSnapshot ? { id: currentAssistantSnapshot.id, name: currentAssistantSnapshot.name } : null}
                      onOpenAssistantCenter={openAssistantCenter}
                      onClearAssistant={() => void handleApplyAssistant(null)}
                      autoFocus
                      usesExternalRuntime={usesExternalRuntime}
                      externalAgentName={activeAgentRuntime.externalAgentId ?? null}
                      conversationId={currentConversation?.id ?? null}
                    />
                  </div>
                </div>
                  ) : (
                    <>
                  <Suspense fallback={<MessageListLoading />}>
                    <MessageList
                      key={currentConversation?.id ?? 'empty'}
                      conversationId={currentConversation?.id}
                      messages={displayMessages}
                      agentPlanState={currentConversation?.agent_plan_state ?? currentConversation?.agentPlanState ?? null}
                      streaming={streaming}
                      streamFrozen={streamFrozen}
                      streamingContent={streamingContent}
                      streamingReasoning={streamingReasoning}
                      streamingReasoningDurationMs={streamingReasoningDurationMs}
                      streamingReasoningDurationMsBySegmentId={streamingReasoningDurationMsBySegmentId}
                      reasoningStreaming={reasoningStreaming}
                      streamingToolCalls={streamingToolCalls}
                      streamingSegments={streamingSegments}
                      error={streamError}
                      assistantStreamStatsByMessageId={assistantStreamStatsByMessageId}
                      onUpdateMessage={handleUpdateMessage}
                      onRegenerateMessage={handleRegenerateMessage}
                      onDeleteMessage={handleDeleteMessage}
                    />
                  </Suspense>
                  <InputBar
                    onSend={(content, attachments) => void handleSendMessage(content, attachments)}
                    disabled={isCurrentConversationBusy()}
                    onCancel={() => void handleCancelStream()}
                    cancelVisible={streaming}
                    cancelling={cancellingStream}
                    onOpenSettings={() => openEmbeddedSettings('chat')}
                    onOpenTools={() => openEmbeddedSettings('skill')}
                    onNewChat={() => void handleNewConversation()}
                    onCompactContext={() => void handleCompressContext()}
                    onClearChat={() => void handleClearChat()}
                    enabledTools={enabledTools}
                    toolsDisabledReason={toolsDisabledReason}
                    toolStatusHint={toolStatusHint}
                    sendDisabledReason={sendDisabledReason}
                    agentPlanState={currentConversation?.agent_plan_state ?? currentConversation?.agentPlanState ?? null}
                    onAgentPlanModeChange={handleAgentPlanModeChange}
                    onExecuteAgentPlan={handleExecuteAgentPlan}
                    enabledSkills={slashSkills}
                    onOpenSkillSettings={openSkillCenter}
                    currentAssistant={currentAssistantSnapshot ? { id: currentAssistantSnapshot.id, name: currentAssistantSnapshot.name } : null}
                    onOpenAssistantCenter={openAssistantCenter}
                    onClearAssistant={() => void handleApplyAssistant(null)}
                    autoFocus
                    usesExternalRuntime={usesExternalRuntime}
                    externalAgentName={activeAgentRuntime.externalAgentId ?? null}
                    conversationId={currentConversation?.id ?? null}
                  />
                    </>
                  )}
                </div>
              </>
            )}
          </div>
        )}
      </div>
      {pendingToolConfirm && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/20 px-4" data-tauri-drag-region="false">
          <div className="w-full max-w-md rounded-lg border border-neutral-200 bg-white p-4 shadow-xl dark:border-neutral-700 dark:bg-neutral-900">
            <div className="mb-3 flex items-start gap-2">
              <Wrench size={17} className="mt-0.5 shrink-0 text-[#C56646] dark:text-[#E39A78]" />
              <div className="min-w-0 flex-1">
                <div className="text-[14px] font-semibold text-neutral-900 dark:text-neutral-100">
                  允许调用工具 {pendingToolConfirm.name}？
                </div>
                <div className="mt-1 text-[12px] text-neutral-500 dark:text-neutral-400">
                  {pendingToolConfirm.source}
                  {pendingToolConfirm.serverId ? ` · ${pendingToolConfirm.serverId}` : ''}
                  {pendingToolConfirm.sensitivity ? ` · ${pendingToolConfirm.sensitivity}` : ''}
                </div>
              </div>
              <button
                type="button"
                className="rounded-md p-1 text-neutral-400 hover:bg-neutral-100 hover:text-neutral-700 dark:hover:bg-neutral-800 dark:hover:text-neutral-200"
                aria-label="拒绝"
                onClick={() => resolvePendingToolConfirm(false)}
              >
                <X size={14} />
              </button>
            </div>
            {pendingToolConfirm.argumentsPreview && (
              <pre className="mb-3 max-h-40 overflow-auto rounded-md bg-neutral-100 p-3 text-[11px] leading-relaxed text-neutral-700 dark:bg-neutral-800 dark:text-neutral-200">
                {pendingToolConfirm.argumentsPreview}
              </pre>
            )}
            <div className="flex justify-end gap-2">
              <button
                type="button"
                className="rounded-md px-3 py-1.5 text-[12px] font-medium text-neutral-600 hover:bg-neutral-100 dark:text-neutral-300 dark:hover:bg-neutral-800"
                onClick={() => resolvePendingToolConfirm(false)}
              >
                拒绝
              </button>
              <button
                type="button"
                className="rounded-md bg-neutral-900 px-3 py-1.5 text-[12px] font-medium text-white hover:bg-neutral-700 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200"
                onClick={() => resolvePendingToolConfirm(true)}
              >
                允许
              </button>
            </div>
          </div>
        </div>
      )}
      {pendingSessionConsent && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/20 px-4" data-tauri-drag-region="false">
          <div className="w-full max-w-md rounded-lg border border-neutral-200 bg-white p-4 shadow-xl dark:border-neutral-700 dark:bg-neutral-900">
            <div className="mb-3 flex items-start gap-2">
              <Wrench size={17} className="mt-0.5 shrink-0 text-[#C56646] dark:text-[#E39A78]" />
              <div className="min-w-0 flex-1">
                <div className="text-[14px] font-semibold text-neutral-900 dark:text-neutral-100">
                  允许本次会话使用文件和命令工具？
                </div>
                <div className="mt-1 text-[12px] text-neutral-500 dark:text-neutral-400">
                  授权后，本会话内 Kivio 可读取、写入、删除磁盘上的任意文件，并执行任意终端命令（包括项目目录之外的位置）。仅本次会话有效，应用重启后需重新授权。
                </div>
              </div>
              <button
                type="button"
                className="rounded-md p-1 text-neutral-400 hover:bg-neutral-100 hover:text-neutral-700 dark:hover:bg-neutral-800 dark:hover:text-neutral-200"
                aria-label="拒绝"
                onClick={() => resolvePendingSessionConsent(false)}
              >
                <X size={14} />
              </button>
            </div>
            <div className="flex justify-end gap-2">
              <button
                type="button"
                className="rounded-md px-3 py-1.5 text-[12px] font-medium text-neutral-600 hover:bg-neutral-100 dark:text-neutral-300 dark:hover:bg-neutral-800"
                onClick={() => resolvePendingSessionConsent(false)}
              >
                拒绝
              </button>
              <button
                type="button"
                className="rounded-md bg-neutral-900 px-3 py-1.5 text-[12px] font-medium text-white hover:bg-neutral-700 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200"
                onClick={() => resolvePendingSessionConsent(true)}
              >
                允许本次会话
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
