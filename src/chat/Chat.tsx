import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Wrench, X } from 'lucide-react'
import { Sidebar, type ExtensionsNavItem } from './Sidebar'
import { ChatImageViewer } from './ChatImageViewer'
import { ChatTitlebarActions } from './ChatTitlebarActions'
import type { AssistantStreamStats } from './MessageList'
import { InputBar } from './InputBar'
import { ModelSelector } from './ModelSelector'
import { ThinkingLevelSelector } from './ThinkingLevelSelector'
import { ExternalModelSelector, RuntimePicker } from './RuntimePicker'
import { PermissionPicker } from './PermissionPicker'
import { BackgroundJobsIndicator } from './BackgroundJobsIndicator'
import { WindowControls } from './WindowControls'
import { ContextIndicator } from './ContextIndicator'
import { AgentTodoIndicator } from './AgentTodoIndicator'
import { isExecutableAgentPlanText } from './agentPlan'
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
  ChatSet,
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
  ThinkingLevel,
  ModelRef,
} from './types'
import {
  api,
  type ChatExternalSendRequest,
  type ChatSessionConsentPayload,
  type ChatStreamPayload,
  type ChatToolConfirmPayload,
  type ChatToolDefinition,
  type ChatMcpServer,
  type ChatToolProgressPayload,
  type ChatUserPromptPayload,
} from '../api/tauri'
import { OnboardingShell } from '../onboarding/OnboardingShell'
import type { SettingsShellHandle, SettingsTab } from '../settings/SettingsShell'
import { normalizeLang, type Lang } from '../settings/i18n'
import { estimateTokens } from '../utils/tokens'
import {
  CHAT_MIN_SIZE_COLLAPSED,
  CHAT_MIN_SIZE_EXPANDED,
  forgetRememberedChatRoute,
  getChatPlatformWindowSize,
  getRememberedChatSidebarCollapsed,
  getRememberedChatWebviewZoom,
  isChatOnboardingPath,
  nextChatWebviewZoom,
  rememberChatSidebarCollapsed,
  rememberChatSize,
  rememberChatWebviewZoom,
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
import {
  getCoarse as getStreamCoarse,
  patchSnapshot as patchStreamSnapshot,
  reset as resetStreamStore,
  setCoarse as setStreamCoarse,
  setSnapshot as setStreamSnapshot,
  useStreamCoarse,
} from './streamingStore'
import {
  beginGroup,
  endGroup,
  ensureGroupColumn,
  flushGroups,
  hasActiveGroup,
  resetGroups,
  touchGroup,
} from './groupStreamingStore'
import { compareTimelineSegments, segmentStepNumber, segmentToolCallId } from './segments'
import { latestCompactionBoundaryId, mergeCompactionContextState } from './compactionBoundary'

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

type ChatView = 'conversation' | 'settings' | 'assistants' | 'skill' | 'onboarding'

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

function isChatOnboardingRoute(path: string): boolean {
  return isChatOnboardingPath(path)
}

function chatWebviewZoomShortcut(e: KeyboardEvent): 'in' | 'out' | 'reset' | null {
  const mod = e.metaKey || e.ctrlKey
  if (!mod || e.altKey) return null
  if (e.key === '+' || e.key === '=' || e.code === 'Equal' || e.code === 'NumpadAdd') return 'in'
  if (e.key === '-' || e.code === 'Minus' || e.code === 'NumpadSubtract') return 'out'
  if (e.key === '0' || e.code === 'Digit0' || e.code === 'Numpad0') return 'reset'
  return null
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

// 設定當前檢視的流式錯誤（寫 streamingStore 的 coarse 片）。模組級函式，呼叫點無需進
// useCallback 依賴。注意：與 setStreamErrorForConversation 不同，這裡只改當前檢視、不寫
// streamErrorsRef（保持原 setStreamError(useState) 的語義）。
function setStreamError(error: string): void {
  setStreamCoarse({ streamError: error })
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

// 把一條 chat-stream delta 累積進給定快照（會話單流 or 多答組某列共用）。
// 原地 mutate snapshot；segment 已由呼叫方算好。返回 void。
function applyStreamDeltaToSnapshot(
  snapshot: ConversationStreamSnapshot,
  payload: ChatStreamPayload,
  segment: ChatMessageSegment | null,
) {
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
}

// done 幀收尾：補齊最後一段 reasoning 的時長。原地 mutate。
function finalizeReasoningDurationOnDone(snapshot: ConversationStreamSnapshot) {
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
}

// 把一條 chat-tool record 累積進給定快照（會話單流 or 多答組某列共用）。原地 mutate。
function applyToolRecordToSnapshot(
  snapshot: ConversationStreamSnapshot,
  record: ToolCallRecord,
) {
  snapshot.streaming = true
  snapshot.reasoningStreaming = false
  const index = snapshot.toolCalls.findIndex((item) => item.id === record.id)
  snapshot.toolCalls = index < 0
    ? [...snapshot.toolCalls, record]
    : snapshot.toolCalls.map((item, i) => (i === index ? { ...item, ...record } : item))
  snapshot.segments = upsertToolStreamSegment(snapshot.segments, record)
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
  if (!compact) return '新對話'
  return compact.length > 30 ? `${compact.slice(0, 30)}...` : compact
}

function optimisticConversationListItem(
  conversation: Conversation,
  content: string,
): ConversationListItem {
  const preview = content.replace(/\s+/g, ' ').trim()
  const title = conversation.title === '新對話'
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
    set_id: conversation.set_id ?? conversation.setId ?? null,
    setId: conversation.set_id ?? conversation.setId ?? null,
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
    if (isChatOnboardingRoute(path)) return 'onboarding'
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
  const [selectedSet, setSelectedSet] = useState<ChatSet | null>(null)
  // 流式高頻狀態已移到 streamingStore（useSyncExternalStore）。Chat 只訂閱 coarse 這一片
  // （streaming/streamFrozen/cancelling/streamError，邊沿才變），用於 showEmptyHero / drain 判定；
  // 內容快照由 MessageList 直接訂閱，避免每幀 token 拖著整個 Chat 重渲。
  const streamCoarse = useStreamCoarse()
  /** 傳送中待顯示的使用者訊息（與 conversation 分離，避免 route reload 沖掉） */
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
  // 歡迎頁（尚無會話）時掛載的知識庫草稿；首次傳送建會話時落到會話上。
  const [draftKnowledgeBaseIds, setDraftKnowledgeBaseIds] = useState<string[]>([])
  // 歡迎頁思考等級草稿；首次傳送建會話時落到會話上。null=跟隨全域。
  const [draftThinkingLevel, setDraftThinkingLevel] = useState<ThinkingLevel | null>(null)
  // 多模型一問多答（任務 06-30）：歡迎頁（尚無會話）時的多答模型草稿；首次傳送建會話時落到會話上。
  const [draftReplyModels, setDraftReplyModels] = useState<ModelRef[]>([])
  const [draftAgentRuntime, setDraftAgentRuntime] = useState<AgentRuntimeConfig>(
    BUILTIN_AGENT_RUNTIME,
  )
  const [skills, setSkills] = useState<SkillMeta[]>([])
  const [disabledSkillIds, setDisabledSkillIds] = useState<string[]>([])
  const [settingsInitialTab, setSettingsInitialTab] = useState<SettingsTab>('chat')
  const [uiLang, setUiLang] = useState<Lang>('zh-TW')
  const [extensionsNavItem, setExtensionsNavItem] = useState<ExtensionsNavItem | null>(null)
  const [enabledTools, setEnabledTools] = useState<ChatToolDefinition[]>([])
  const [mcpServers, setMcpServers] = useState<ChatMcpServer[]>([])
  const [enabledToolCount, setEnabledToolCount] = useState<number | null>(null)
  const [toolsDisabledReason, setToolsDisabledReason] = useState('')
  const [toolsRequested, setToolsRequested] = useState(false)
  const [approvalPolicy, setApprovalPolicy] = useState('readonly_auto_sensitive_confirm')
  const [pendingToolConfirm, setPendingToolConfirm] = useState<ChatToolConfirmPayload | null>(null)
  const [pendingSessionConsent, setPendingSessionConsent] = useState<ChatSessionConsentPayload | null>(null)
  const [contextState, setContextState] = useState<ConversationContextState | null>(null)
  const [contextLoading, setContextLoading] = useState(false)
  const [contextCompressing, setContextCompressing] = useState(false)
  const [agentLoopCompacting, setAgentLoopCompacting] = useState(false)
  const [animateCompactionBoundaryId, setAnimateCompactionBoundaryId] = useState<string | null>(null)
  const [contextError, setContextError] = useState('')
  const [imageViewerItem, setImageViewerItem] = useState<ChatImageViewerItem | null>(null)
  const chatWebviewZoomRef = useRef(getRememberedChatWebviewZoom())
  const currentConversationIdRef = useRef<string | null>(null)
  // 始終指向最新 currentConversation。訊息操作 handler（編輯/刪除/重發）藉此讀取最新會話，
  // 而無需把 currentConversation 列進 useCallback 依賴——否則每次切模型/思考等級（currentConversation
  // 換引用）這些 handler 都換身份，打穿 MessageBubble 的 memo 導致全列表重渲（公式 remount 閃爍）。
  const currentConversationRef = useRef(currentConversation)
  currentConversationRef.current = currentConversation
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
  // A 合幀（render coalescing）：高頻 stream/tool/subagent/userprompt 事件不再每條都同步
  // setState 重渲，而是把"待顯示的快照"記到 ref，用 requestAnimationFrame 每幀最多 flush 一次。
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

  // B：徹底把一個會話從所有本機樂觀/in-flight/快照狀態中剔除（ghost 清理）。
  // 不觸碰 currentConversation/route，由呼叫方按場景決定。
  const dropConversationLocally = useCallback((conversationId: string) => {
    inFlightConversationsRef.current.delete(conversationId)
    delete streamSnapshotsRef.current[conversationId]
    delete pendingToolConfirmsRef.current[conversationId]
    delete pendingSessionConsentsRef.current[conversationId]
    delete pendingStreamDoneRef.current[conversationId]
    delete streamErrorsRef.current[conversationId]
    // 若該會話還掛著待重新整理的合幀，連帶取消，避免被剔除的 ghost 還閃一幀。
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
      setStreamCoarse({ streamError: error })
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
    // 兜底網：後端已在所有返回 Conversation 的命令出口剝離 model_messages/api_messages
    // （strip_transcripts_for_frontend），所以正常路徑到這裡已是輕量副本。這裡再剝一次，確保
    // 任何遺漏/未來新增的後端出口都不會把這兩份前端永不讀的轉錄留進 React state。後端回放讀盤
    // 上完整副本，不受影響。
    if (conversation?.messages) {
      for (const m of conversation.messages) {
        if (m.role !== 'assistant') continue
        m.model_messages = undefined
        m.modelMessages = undefined
        m.api_messages = undefined
        m.apiMessages = undefined
      }
    }
    setCurrentConversation(conversation)
    setContextState(conversation?.context_state ?? conversation?.contextState ?? null)
  }, [])

  // 純後設資料更新（模型 / 思考等級 / 知識庫掛載等）：合併後端返回的新後設資料，但**保留現有
  // messages 陣列引用**。否則每條訊息都變成新物件，擊穿 MessageBubble/ChatMarkdown 的 memo，
  // 歷史訊息裡的 LaTeX 會整屏重渲閃一下。這類更新後端不會改 messages，沿用舊引用安全。
  const applyConversationMeta = useCallback((updated: Conversation) => {
    setCurrentConversation((prev) =>
      prev && prev.id === updated.id ? { ...updated, messages: prev.messages } : updated,
    )
  }, [])

  const patchContextState = useCallback((nextState: ConversationContextState) => {
    setContextState((prev) => {
      const merged = mergeCompactionContextState(prev, nextState)
      setCurrentConversation((conversation) => conversation
        ? { ...conversation, context_state: merged, contextState: merged }
        : conversation)
      return merged
    })
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
    // 取消掛起的合幀，避免舊快照在清空後又被刷回來產生空幀/串幀。
    if (streamRenderRafRef.current != null) {
      cancelAnimationFrame(streamRenderRafRef.current)
      streamRenderRafRef.current = null
    }
    pendingStreamRenderRef.current = null
    // 內容回空閒 + streaming/frozen/cancelling 歸位；streamError 不動（與原語義一致）。
    resetStreamStore()
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
    // 切換會話/恢復預覽前取消任何掛起的合幀，避免上一個會話的快照被刷到當前檢視。
    if (streamRenderRafRef.current != null) {
      cancelAnimationFrame(streamRenderRafRef.current)
      streamRenderRafRef.current = null
    }
    pendingStreamRenderRef.current = null
    if (!conversationId) {
      clearStreamingPreview()
      setPendingToolConfirm(null)
      setPendingSessionConsent(null)
      setStreamCoarse({ streamError: '' })
      return
    }
    const snapshot = streamSnapshotsRef.current[conversationId]
    if (!snapshot) {
      clearStreamingPreview()
    } else {
      setStreamSnapshot(snapshot)
      setStreamCoarse({ streaming: snapshot.streaming, streamFrozen: false, cancelling: false })
      activeRunIdRef.current = snapshot.runId
      streamStartedAtRef.current = snapshot.startedAt
      streamingContentRef.current = snapshot.content
      streamingReasoningRef.current = snapshot.reasoning
    }
    setStreamCoarse({ streamError: streamErrorsRef.current[conversationId] ?? '' })
    setPendingToolConfirm(pendingToolConfirmsRef.current[conversationId] ?? null)
    setPendingSessionConsent(pendingSessionConsentsRef.current[conversationId] ?? null)
  }, [clearStreamingPreview])

  const applyStreamSnapshotToState = useCallback((snapshot: ConversationStreamSnapshot) => {
    setStreamSnapshot(snapshot)
    setStreamCoarse({ streaming: snapshot.streaming, cancelling: false })
    activeRunIdRef.current = snapshot.runId
    streamStartedAtRef.current = snapshot.startedAt
    streamingContentRef.current = snapshot.content
    streamingReasoningRef.current = snapshot.reasoning
  }, [])

  // 立即把掛起幀刷出去（done/結束、解除安裝、切換會話前呼叫），保證不丟最後一幀。
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

  // 取消掛起幀而不應用（切換會話/解除安裝時呼叫，避免把舊會話快照刷到新會話）。
  // 注：clearStreamingPreview / restoreStreamingPreview 已內聯同樣的取消邏輯。

  // A 合幀：事件本身仍即時累積到 snapshot 物件，這裡只把"渲染"節流到每幀一次。
  // immediate=true（done 等終止幀）立即 flush，不再等下一幀。
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
    // 解除安裝時清掉所有活躍多答組，避免遺留列快照。
    resetGroups()
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
    // 立即停掉"生成中"視覺（撤掉取消按鈕 + 停 shimmer），但保留已生成文字：
    // 切到 frozen 態凍結展示，等 send invoke 返回持久化訊息時由
    // finishStreamingRunWithConversation 無縫替換（clearStreamingPreview 會清除 frozen）。
    // 後續遲到的流事件已被 isLocallyCancelledPayload 過濾，預覽不會再變動。
    setStreamCoarse({ streaming: false, streamFrozen: true })
    patchStreamSnapshot({ reasoningStreaming: false })
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

  const activeAgentRuntime = useMemo(
    () => (currentConversation ? normalizeAgentRuntime(currentConversation) : draftAgentRuntime),
    [currentConversation, draftAgentRuntime],
  )
  const usesExternalRuntime = activeAgentRuntime.kind === 'external' && !!activeAgentRuntime.externalAgentId
  const currentConversationIsBlank = isPlainBlankConversation(currentConversation)
  const activeProviderId = currentConversation && !currentConversationIsBlank
    ? currentConversation.provider_id
    : draftProviderId
  const activeModel = currentConversation && !currentConversationIsBlank
    ? currentConversation.model
    : draftModel
  // 多模型一問多答（任務 06-30）：當前生效的多答模型集（會話級持久 reply_models，歡迎頁用草稿）。
  const activeReplyModels = useMemo<ModelRef[]>(
    () => (currentConversation && !currentConversationIsBlank
      ? currentConversation.reply_models ?? currentConversation.replyModels ?? []
      : draftReplyModels),
    [currentConversation, currentConversationIsBlank, draftReplyModels],
  )
  const storedActiveSkillId = currentConversation
    ? currentConversation.active_skill_id ?? currentConversation.activeSkillId ?? null
    : null
  // 当前会话自身所属项目（id + 名 folder）。传给输入栏，使从「最近」打开的项目内对话
  // 也能在项目按钮上显示其项目，即便导航态 selectedProject 已被清空。
  const conversationProject = useMemo<{ id: string; name: string } | null>(() => {
    const id = currentConversation?.project_id ?? currentConversation?.projectId ?? null
    if (!id) return null
    return { id, name: currentConversation?.folder ?? '' }
  }, [currentConversation?.project_id, currentConversation?.projectId, currentConversation?.folder])
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
      setMcpServers([])
      return
    }
    try {
      const settings = await api.getSettings()
      const chatTools = settings.chatTools
      setMcpServers(chatTools?.servers ?? [])
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
          setToolsDisabledReason('當前模型不支援 tools；已選 Skill 時將注入 SKILL.md')
        } else if (skillRuntimeEnabled) {
          setToolsDisabledReason('Skill 漸進式載入需要 tools 支援；已選 Skill 時將注入 SKILL.md')
        } else {
          setToolsDisabledReason('當前模型不支援 tools')
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

  const handleToggleMcpServer = useCallback(async (serverId: string) => {
    try {
      const settings = await api.getSettings()
      const servers = (settings.chatTools?.servers ?? []).map((server) =>
        server.id === serverId ? { ...server, enabled: !server.enabled } : server,
      )
      // 樂觀更新本機列表（開關即時反饋），儲存後由 refreshToolIndicator 校正。
      setMcpServers(servers)
      await api.saveSettings({
        ...settings,
        chatTools: { ...settings.chatTools, servers },
      })
      onSettingsChange()
      await refreshToolIndicator()
    } catch (err) {
      console.error('Failed to toggle MCP server:', err)
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
      if (toolsDisabledReason.includes('不支援 tools') && effectiveSkillId) {
        return toolsDisabledReason
      }
      return effectiveSkillRecommendedTools.length > 0
        ? `當前 Skill 需要工具，但${toolsDisabledReason}`
        : toolsDisabledReason
    }
    if (toolsDisabledReason && (enabledToolCount ?? 0) === 0) {
      return ''
    }
    if (unavailableRecommendedTools.length > 0) {
      return `當前 Skill 推薦的工具不可用：${unavailableRecommendedTools.slice(0, 3).join(', ')}`
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
    if (rest === 'skill' || rest.startsWith('skill/')) return null
    if (rest === 'onboarding' || rest.startsWith('onboarding/')) return null
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

  const syncOnboardingRoute = useCallback(() => {
    if (window.location.hash !== '#chat/onboarding') {
      window.location.hash = '#chat/onboarding'
    }
  }, [])

  const handleOnboardingExit = useCallback(() => {
    setChatView('conversation')
    syncConversationRoute(null)
  }, [syncConversationRoute])

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
      setUiLang(normalizeLang(settings.settingsLanguage))
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
      setStreamCoarse({ cancelling: false })
    } catch (err) {
      console.error('Failed to reload conversation:', err)
      // B2：reload 失敗（尤其"對話不存在"）——把 ghost 從樂觀列表/in-flight/快照剔除並重新整理側欄。
      dropConversationLocally(conversationId)
      forgetRememberedChatRoute()
      if (currentConversationIdRef.current === conversationId || currentConversationIdRef.current === null) {
        currentConversationIdRef.current = null
        applyConversation(null)
        syncConversationRoute(null)
      }
      refreshSidebar()
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '對話載入失敗，已從列表移除')
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
        setContextError(typeof err === 'string' ? err : (err as Error).message || '上下文統計失敗')
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
        const latestId = latestCompactionBoundaryId(result.contextState)
        if (latestId) {
          setAnimateCompactionBoundaryId(latestId)
          window.setTimeout(() => {
            setAnimateCompactionBoundaryId((current) => (current === latestId ? null : current))
          }, 1800)
        }
        patchContextState(result.contextState)
        refreshSidebar()
        await new Promise<void>((resolve) => {
          window.setTimeout(resolve, 360)
        })
      }
    } catch (err) {
      if (currentConversationIdRef.current === conversationId) {
        setContextError(typeof err === 'string' ? err : (err as Error).message || '上下文壓縮失敗')
      }
    } finally {
      if (currentConversationIdRef.current === conversationId) {
        setContextCompressing(false)
      }
    }
  }, [contextCompressing, patchContextState, refreshSidebar])

  const finishStreamingRun = useCallback(
    async (payload: { reason?: string; conversationId?: string }) => {
      const conversationId = payload.conversationId ?? currentConversationIdRef.current
      // 兜底：run 結束時壓縮必然已終止；防禦後端遺漏終止事件把"壓縮中"狀態卡死。
      setAgentLoopCompacting(false)
      if (payload.reason !== 'cancelled') {
        resetLocalCancellation()
      }
      if (payload.reason === 'error' && conversationId) {
        setStreamErrorForConversation(
          conversationId,
          streamErrorsRef.current[conversationId] || '回覆生成失敗，請稍後重試。',
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
        // 多答組分支（任務 06-30）：該會話處於多模型併發流時，按 messageId 路由到對應列，
        // 不動會話級單流快照（單模型路徑零迴歸）。
        if (hasActiveGroup(payload.conversationId) && payload.messageId) {
          const column = ensureGroupColumn(
            payload.conversationId,
            payload.messageId,
          )
          if (!column) return
          const segment = streamPayloadToSegment(payload)
          applyStreamDeltaToSnapshot(column, payload, segment)
          if (payload.done) {
            finalizeReasoningDurationOnDone(column)
            column.streaming = false
            // 列結束是終止幀：立即 flush（不等下一幀），讓該列完成態儘快可見。
            flushGroups()
          } else {
            // 內容 delta 經 rAF 合幀（N 列高頻 delta 不各自打爆 setState）。
            touchGroup()
          }
          // 組的整體「done / 持久化」交給 sendMessage 返回後的統一收尾；這裡不觸發 finishStreamingRun。
          return
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
          // done：立即 flush 最後一幀，別讓合幀吞掉收尾內容。
          showStreamSnapshotIfCurrent(payload.conversationId, snapshot, true)
          // invoke 未完成前不要 reload；延後到 flushPendingStreamDone，避免與 send 寫盤競態。
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
      unlisten = await api.onChatCompaction((payload) => {
        if (cancelled) return
        const currentConversationId = currentConversationIdRef.current
        if (!currentConversationId || payload.conversationId !== currentConversationId) {
          return
        }
        if (payload.phase === 'started') {
          if (payload.trigger !== 'manual') {
            setAgentLoopCompacting(true)
          }
          return
        }
        if (payload.trigger !== 'manual') {
          setAgentLoopCompacting(false)
        }
        const boundary = payload.boundary
        if (boundary?.id) {
          setAnimateCompactionBoundaryId(boundary.id)
          window.setTimeout(() => {
            setAnimateCompactionBoundaryId((current) => (current === boundary.id ? null : current))
          }, 1800)
        }
        if (boundary && payload.phase === 'completed') {
          setCurrentConversation((conversation) => {
            if (!conversation) return conversation
            const prevState = conversation.context_state ?? conversation.contextState
            const existing = prevState?.compaction_boundaries ?? prevState?.compactionBoundaries ?? []
            if (existing.some((item) => item.id === boundary.id)) return conversation
            const nextBoundaries = [...existing, boundary]
            const nextState = {
              ...(prevState ?? {}),
              compaction_boundaries: nextBoundaries,
              compactionBoundaries: nextBoundaries,
            }
            setContextState(nextState)
            return { ...conversation, context_state: nextState, contextState: nextState }
          })
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
        // 忽略 invoke 結束後的遲到 tool 事件，否則會重新 setStreaming(true) 卡死輸入欄。
        if (!isConversationInFlight(inFlightConversationsRef.current, payload.conversationId)) return
        // 多答組分支：按 messageId 路由到對應列。
        if (hasActiveGroup(payload.conversationId) && payload.messageId) {
          const column = ensureGroupColumn(payload.conversationId, payload.messageId)
          if (!column) return
          const record = toolEventToRecord(payload)
          applyToolRecordToSnapshot(column, record)
          touchGroup()
          return
        }
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
        // A `chat-subagent` progress event must address an existing snapshot for
        // the parent conversation (do NOT create one — that would resurrect a
        // finalized conversation). Accept whenever the conversation is in-flight
        // or a snapshot already exists.
        const existingSnapshot = streamSnapshotsRef.current[payload.parentConversationId]
        const inFlight = isConversationInFlight(
          inFlightConversationsRef.current,
          payload.parentConversationId,
        )
        if (!inFlight && !existingSnapshot) return
        const snapshot = ensureStreamSnapshot(payload.parentConversationId)
        // Match the active run when known; only drop when both ids are set and differ.
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
        // Sub-agents run blocking + single-result: the parent tool card transitions
        // running→done via the `chat-tool` flow (the inline result), while these
        // `chat-subagent` events drive the live nested progress (steps/preview).
        snapshot.toolCalls = snapshot.toolCalls.map((item, i) => {
          if (i !== index) return item
          const existing =
            item.structuredContent && typeof item.structuredContent === 'object'
              ? (item.structuredContent as Record<string, unknown>)
              : {}
          const nextStructured: Record<string, unknown> = {
            ...existing,
            subagentProgress: progress,
          }
          return {
            ...item,
            structuredContent: nextStructured,
          }
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
            const { runPythonInSandbox } = await import('./pyodideClient')
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
              `Python 沙盒呼叫失敗：${message || 'Unknown error'}。不要使用 run_command/pip 安裝或修改本機 Python 環境來繞過沙盒；請直接基於已有資料回答，除非使用者明確要求修改本機環境。`,
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
      if (isChatOnboardingRoute(path)) {
        setChatView('onboarding')
        return
      }
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
    if (!isTauriRuntime()) return
    let cancelled = false
    void api.getSettings().then((settings) => {
      if (cancelled) return
      if (settings.onboardingStatus === 'pending' && !isChatOnboardingRoute(hashPath())) {
        syncOnboardingRoute()
      }
    }).catch((err) => {
      console.error('Failed to check onboarding status:', err)
    })
    return () => {
      cancelled = true
    }
  }, [syncOnboardingRoute])

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
      // B2：點開一個不存在/載入失敗的 ghost——從樂觀列表 + in-flight + 快照剔除，
      // 清空當前會話並重新整理側欄，讓 ghost 自動消失而不是卡住。
      dropConversationLocally(conversationId)
      if (currentConversationIdRef.current === conversationId) {
        currentConversationIdRef.current = null
        applyConversation(null)
      }
      forgetRememberedChatRoute()
      syncConversationRoute(null)
      refreshSidebar()
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '對話載入失敗，已從列表移除')
    }
  }, [applyConversation, dropConversationLocally, refreshSidebar, restoreStreamingPreview, syncConversationRoute])

  const handleNewConversation = useCallback(async () => {
    setSelectedProject(null)
    setSelectedSet(null)
    setAssistantStreamStatsByMessageId({})
    setDraftProviderId(activeProviderId)
    setDraftModel(activeModel)
    setDraftAgentRuntime(activeAgentRuntime)
    setDraftKnowledgeBaseIds([])
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
      setStreamErrorForConversation(conversationId, '請先停止當前回覆，再清空對話。')
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
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '清空對話失敗')
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
        selectedSet?.id ?? null,
      )
      currentConversationIdRef.current = conv.id
      applyConversation(conv)
      restoreStreamingPreview(conv.id)
      syncConversationRoute(conv.id)
      refreshSidebar()
      setStreamError('')
    } catch (err) {
      console.error('Failed to start assistant conversation:', err)
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '建立助手對話失敗')
    }
  }, [activeModel, activeProviderId, applyConversation, refreshSidebar, restoreStreamingPreview, selectedProject?.id, selectedProject?.name, selectedSet?.id, syncConversationRoute])

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
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '建立搭建對話失敗')
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
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '助手切換失敗')
    }
  }, [applyConversation, currentConversation, refreshContextStats, refreshSidebar])

  const ensureConversationForAgentPlan = useCallback(async () => {
    if (currentConversation) return currentConversation
    let conversation = await chatApi.createConversation(
      activeProviderId || undefined,
      activeModel || undefined,
      selectedProject?.name,
      selectedProject?.id ?? null,
      undefined,
      selectedSet?.id ?? null,
    )
    if (!agentRuntimesEqual(normalizeAgentRuntime(conversation), draftAgentRuntime)) {
      conversation = await chatApi.setAgentRuntime(conversation.id, draftAgentRuntime)
    }
    currentConversationIdRef.current = conversation.id
    applyConversation(conversation)
    syncConversationRoute(conversation.id)
    refreshSidebar()
    return conversation
  }, [activeModel, activeProviderId, applyConversation, currentConversation, draftAgentRuntime, refreshSidebar, selectedProject?.id, selectedProject?.name, selectedSet?.id, syncConversationRoute])

  const handleAgentPlanModeChange = useCallback(async (mode: AgentPlanMode) => {
    try {
      const conversation = await ensureConversationForAgentPlan()
      const updated = await chatApi.setAgentPlanMode(conversation.id, mode)
      applyConversation(updated)
      void refreshContextStats(updated.id)
      refreshSidebar()
    } catch (err) {
      console.error('Failed to update agent plan mode:', err)
      setStreamError(typeof err === 'string' ? err : (err as Error).message || 'Plan 模式切換失敗')
    }
  }, [applyConversation, ensureConversationForAgentPlan, refreshContextStats, refreshSidebar])

  const handleSelectProject = useCallback((project: ChatProject | null) => {
    setSelectedProject(project)
    setSelectedSet(null)
    setAssistantStreamStatsByMessageId({})
    setPendingUserMessage(null)
    setPendingUserMessageConversationId(null)
    currentConversationIdRef.current = null
    applyConversation(null)
    restoreStreamingPreview(null)
    syncConversationRoute(null)
    setStreamError('')
  }, [applyConversation, restoreStreamingPreview, syncConversationRoute])

  const handleSelectSet = useCallback((set: ChatSet | null) => {
    setSelectedSet(set)
    setSelectedProject(null)
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
      if (e.key === ',') {
        e.preventDefault()
        openEmbeddedSettings()
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [chatView, handleNewConversation, openEmbeddedSettings])

  const applyChatWebviewZoom = useCallback(async (zoom: number) => {
    if (!isTauriRuntime()) return
    const { getCurrentWebview } = await import('@tauri-apps/api/webview')
    await getCurrentWebview().setZoom(zoom)
  }, [])

  useEffect(() => {
    const zoom = getRememberedChatWebviewZoom()
    chatWebviewZoomRef.current = zoom
    void applyChatWebviewZoom(zoom).catch((err) => {
      console.error('[Chat] Failed to restore webview zoom:', err)
    })
  }, [applyChatWebviewZoom])

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const direction = chatWebviewZoomShortcut(e)
      if (!direction) return

      e.preventDefault()
      e.stopPropagation()
      const nextZoom = nextChatWebviewZoom(chatWebviewZoomRef.current, direction)
      chatWebviewZoomRef.current = nextZoom
      rememberChatWebviewZoom(nextZoom)
      void applyChatWebviewZoom(nextZoom).catch((err) => {
        console.error('[Chat] Failed to apply webview zoom:', err)
      })
    }

    window.addEventListener('keydown', onKeyDown, true)
    return () => window.removeEventListener('keydown', onKeyDown, true)
  }, [applyChatWebviewZoom])

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
          undefined,
          selectedSet?.id ?? null,
        )
        currentConversationIdRef.current = conversation.id
        applyConversation(conversation)
        syncConversationRoute(conversation.id)
      } catch (err) {
        console.error('Failed to create conversation before send:', err)
        setStreamError(typeof err === 'string' ? err : (err as Error).message || '建立對話失敗')
        return false
      }
    }

    if (!agentRuntimesEqual(normalizeAgentRuntime(conversation), draftAgentRuntime)) {
      try {
        conversation = await chatApi.setAgentRuntime(conversation.id, draftAgentRuntime)
        applyConversation(conversation)
      } catch (err) {
        console.error('Failed to apply agent runtime before send:', err)
        setStreamError(typeof err === 'string' ? err : (err as Error).message || 'Agent 切換失敗')
        return false
      }
    }

    // Apply the welcome-page knowledge-base draft to the freshly-created
    // conversation (mounting on the welcome screen had no conversation yet).
    {
      const convKb = conversation.knowledge_base_ids ?? conversation.knowledgeBaseIds ?? []
      const sameKb =
        convKb.length === draftKnowledgeBaseIds.length &&
        convKb.every((id) => draftKnowledgeBaseIds.includes(id))
      if (draftKnowledgeBaseIds.length > 0 && !sameKb) {
        try {
          conversation = await chatApi.updateConversation(conversation.id, {
            knowledgeBaseIds: draftKnowledgeBaseIds,
          })
          applyConversation(conversation)
        } catch (err) {
          console.error('Failed to apply knowledge base draft before send:', err)
        }
      }
    }

    // 同理：把歡迎頁選好的思考等級草稿落到新會話上。
    if (draftThinkingLevel) {
      const convLevel = conversation.thinking_level ?? conversation.thinkingLevel ?? null
      if (convLevel !== draftThinkingLevel) {
        try {
          conversation = await chatApi.updateConversation(conversation.id, {
            thinkingLevel: draftThinkingLevel,
          })
          applyConversation(conversation)
        } catch (err) {
          console.error('Failed to apply thinking level draft before send:', err)
        }
      }
    }

    // 多模型一問多答（任務 06-30）：把歡迎頁選好的多答模型草稿落到新會話上。
    {
      const convReplyModels = conversation.reply_models ?? conversation.replyModels ?? []
      const sameReply =
        convReplyModels.length === draftReplyModels.length &&
        convReplyModels.every((ref, i) =>
          ref.provider_id === draftReplyModels[i]?.provider_id
          && ref.model === draftReplyModels[i]?.model)
      if (draftReplyModels.length > 0 && !sameReply) {
        try {
          conversation = await chatApi.updateConversation(conversation.id, {
            replyModels: draftReplyModels,
          })
          applyConversation(conversation)
        } catch (err) {
          console.error('Failed to apply reply models draft before send:', err)
        }
      }
    }

    const conversationId = conversation.id
    if (isConversationInFlight(inFlightConversationsRef.current, conversationId)) {
      setStreamErrorForConversation(conversationId, '該對話正在生成中，請稍後再試')
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
      // 起新一輪：內容回空閒，coarse 置 streaming。
      resetStreamStore()
      setStreamCoarse({ streaming: true })
      setStreamErrorForConversation(conversationId, '')
      activeRunIdRef.current = null
      streamStartedAtRef.current = startedAt
      streamingContentRef.current = ''
      streamingReasoningRef.current = ''
      setPendingUserMessage(optimisticUserMessage)
      setPendingUserMessageConversationId(conversationId)
    }

    markConversationInFlight(conversationId)
    // 多模型一問多答（任務 06-30）：reply_models ≥2 且非 plan/orchestrate 模式時，後端會 fan-out
    // 出 N 條併發流。前端據此建多答組（佔位 N 列），流事件按 messageId 路由到對應列。
    // 與後端 resolve_reply_arms 的判定保持一致（≤1 個臂 = 單模型路徑，零迴歸）。
    const replyArms = conversation.reply_models ?? conversation.replyModels ?? []
    const convPlanMode =
      conversation.agent_plan_state?.mode ?? conversation.agentPlanState?.mode ?? 'act'
    const willFanOut = replyArms.length >= 2 && convPlanMode === 'act'
    if (willFanOut) {
      const groupId = `grp-local-${Date.now()}`
      beginGroup(
        conversationId,
        groupId,
        replyArms.map((ref) => ({ providerId: ref.provider_id, model: ref.model })),
      )
      // 多答組不走單流預覽：清掉剛才置的會話級 streaming 佔位，避免頂部多出一條空預覽氣泡。
      if (currentConversationIdRef.current === conversationId) {
        resetStreamStore()
        setStreamCoarse({ streaming: true })
      }
    }
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
      // 後端生成失敗時保留了使用者訊息並隨錯誤帶回對話——套用它，讓問題留線上程裡可重試，
      // 而不是連問題一起消失（舊行為）。
      const keptConversation = (err as { conversation?: Conversation })?.conversation
      if (currentConversationIdRef.current === conversationId) {
        setPendingUserMessage(null)
        setPendingUserMessageConversationId(null)
        if (keptConversation) {
          applyConversation(keptConversation)
        }
      }
      setOptimisticSidebarConversations((items) => items.filter((item) => item.id !== conversationId))
      clearStreamSnapshot(conversationId)
      if (keptConversation) refreshSidebar()
      const message = typeof err === 'string' ? err : (err as Error).message || '傳送失敗'
      setStreamErrorForConversation(conversationId, message)
    } finally {
      clearConversationInFlight(conversationId)
      // 多答組收尾：sendMessage 返回時所有臂已結束，持久化後的會話已 applyConversation（含 N 條
      // 帶 group_id 的 assistant 訊息），即時流列已可丟棄，由 MessageGroup 渲染落庫後的列。
      endGroup(conversationId)
      if (persistedConversation) {
        // invoke 已返回持久化後的完整對話且上面已 applyConversation。
        // 丟棄被延後的 finishStreamingRun(它會再次全量 reloadConversation),避免每輪隨歷史線性變慢。
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
    draftKnowledgeBaseIds,
    draftThinkingLevel,
    draftReplyModels,
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
    selectedSet?.id,
    sendDisabledReason,
    setStreamErrorForConversation,
    syncConversationRoute,
    syncGeneratingConversationIds,
  ])

  // 用 ref 持有最新 handleSendMessage，使下方的 drainExternalSends 保持穩定身份，
  // 避免其依賴抖動導致訂閱 effect 反覆 cleanup/重訂閱（重訂閱縫隙會丟掉外部傳送事件）。
  const handleSendMessageRef = useRef(handleSendMessage)
  handleSendMessageRef.current = handleSendMessage

  // 歷史預置（Lens「在 AI 客戶端繼續」交接）：用最新 reactive 值（provider/model/project）建立帶歷史的新會話。
  // 同 handleSendMessageRef 思路用 ref 持有，保持 drainExternalSends 穩定身份。
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
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '匯入對話失敗')
      return false
    }
  }, [activeModel, activeProviderId, applyConversation, refreshSidebar, selectedProject?.id, syncConversationRoute])
  const importExternalConversationRef = useRef(importExternalConversation)
  importExternalConversationRef.current = importExternalConversation

  const handleExecuteAgentPlan = useCallback(async (messageId: string) => {
    const conversation = currentConversation
    if (!conversation) return
    const planMessage = conversation.messages.find((message) => message.id === messageId)
    const messagePlan = planMessage?.agent_plan ?? planMessage?.agentPlan ?? null
    const messagePlanText = messagePlan?.plan?.trim() ?? ''
    const legacyPlan = conversation.agent_plan_state ?? conversation.agentPlanState ?? null
    const legacyPlanText = legacyPlan?.plan?.trim() ?? ''
    const isLegacyPlanMessage = Boolean(
      planMessage
      && !isExecutableAgentPlanText(messagePlanText)
      && isExecutableAgentPlanText(legacyPlanText)
      && planMessage.role === 'assistant'
      && planMessage.content.trim() === legacyPlanText,
    )
    const planText = isExecutableAgentPlanText(messagePlanText)
      ? messagePlanText
      : (isLegacyPlanMessage ? legacyPlanText : '')
    if (!isExecutableAgentPlanText(planText)) return
    if (isConversationInFlight(inFlightConversationsRef.current, conversation.id)) {
      setStreamErrorForConversation(conversation.id, '該對話正在生成中，請稍後再試')
      return
    }

    try {
      const updated = await chatApi.executeAgentPlan(
        conversation.id,
        isExecutableAgentPlanText(messagePlanText) ? messageId : undefined,
      )
      applyConversation(updated)
      refreshSidebar()
      void refreshContextStats(updated.id)
      void handleSendMessage('按這條計畫開始執行。', [], { conversationOverride: updated })
    } catch (err) {
      console.error('Failed to execute agent plan:', err)
      setStreamErrorForConversation(
        conversation.id,
        typeof err === 'string' ? err : (err as Error).message || '執行計畫失敗',
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

        // 歷史預置分支：把 Lens 完整多輪歷史 + 截圖搬成一個新會話（不發訊息、不觸發回覆），落地末尾可續聊。
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
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '外部訊息傳送失敗')
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

    // 外部傳送（如 Lens 交接）的投遞不依賴某個一次性事件的時序：
    // 任意可靠時機都主動從後端取走 pending（chat_take_external_sends 冪等，取空即 no-op）。
    void drainExternalSends()
    // 1) 後端就緒事件
    register(api.onChatExternalSendReady(() => {
      if (!cancelled) void drainExternalSends()
    }))
    // 2) 視窗獲得焦點 —— 覆蓋複用視窗被重新喚起、以及冷啟動時就緒事件丟失的情況
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
    if (!streamCoarse.streaming && externalSendDrainRequestedRef.current) {
      void drainExternalSends()
    }
  }, [drainExternalSends, streamCoarse.streaming])

  const handleUpdateMessage = useCallback(
    async (messageId: string, content: string) => {
      const conv = currentConversationRef.current
      if (!conv) return
      try {
        const updated = await chatApi.updateMessage(conv.id, messageId, content)
        applyConversation(updated)
        refreshSidebar()
      } catch (err) {
        console.error('Failed to update message:', err)
        setStreamError(typeof err === 'string' ? err : (err as Error).message || '儲存失敗')
      }
    },
    [applyConversation, refreshSidebar],
  )

  const handleDeleteMessage = useCallback(
    async (messageId: string) => {
      const conv = currentConversationRef.current
      if (!conv) return
      if (!window.confirm('確定刪除這條訊息嗎？')) return
      try {
        const updated = await chatApi.deleteMessage(conv.id, messageId)
        applyConversation(updated)
        setAssistantStreamStatsByMessageId((prev) => {
          const next = { ...prev }
          delete next[messageId]
          return next
        })
        refreshSidebar()
      } catch (err) {
        console.error('Failed to delete message:', err)
        setStreamError(typeof err === 'string' ? err : (err as Error).message || '刪除失敗')
      }
    },
    [applyConversation, refreshSidebar],
  )

  // 多答組「選中條」（任務 06-30 / D5）：標記某組進下一輪歷史的列。預設第一列；使用者點選改。
  const handleSetGroupSelection = useCallback(
    async (groupId: string, messageId: string) => {
      const conv = currentConversationRef.current
      if (!conv) return
      try {
        const updated = await chatApi.setGroupSelection(conv.id, groupId, messageId)
        applyConversationMeta(updated)
      } catch (err) {
        console.error('Failed to set group selection:', err)
        setStreamError(typeof err === 'string' ? err : (err as Error).message || '選中失敗')
      }
    },
    [applyConversationMeta],
  )

  const handleRegenerateMessage = useCallback(
    async (messageId: string, newContent?: string) => {
      const conv = currentConversationRef.current
      if (!conv) return

      const conversationId = conv.id
      // Busy 拒絕（AC3）：入口已在 MessageList 按 streaming/frozen 收起，這裡是兜底。
      // 帶編輯內容時靜默 return 會無聲丟掉使用者改的文字，必須給出提示（與 handleSend 同文案）。
      if (isConversationInFlight(inFlightConversationsRef.current, conversationId)) {
        setStreamErrorForConversation(conversationId, '該對話正在生成中，請稍後再試')
        return
      }

      const messageIndex = conv.messages.findIndex(
        (message) => message.id === messageId,
      )
      if (messageIndex < 0) return

      // 助手訊息：截到它之前重生成。使用者訊息：保留它（編輯時先替換內容）、只丟其後內容再重試。
      const keepTarget = conv.messages[messageIndex].role === 'user'
      const cutFrom = keepTarget ? messageIndex + 1 : messageIndex
      // 空白-only 的編輯內容按「未編輯」處理（純重生成）：絕不能把 Some("") 發給後端——
      // 樂觀截斷已經執行，後端再報「訊息內容不能為空」會留下截斷了卻沒重生成的執行緒。
      const trimmedNewContent = newContent?.trim() || undefined
      const keptMessages = conv.messages.slice(0, cutFrom)
      if (keepTarget && trimmedNewContent) {
        keptMessages[messageIndex] = {
          ...keptMessages[messageIndex],
          content: trimmedNewContent,
        }
      }
      applyConversation({
        ...conv,
        messages: keptMessages,
      })
      const removedMessageIds = new Set(
        conv.messages.slice(cutFrom).map((message) => message.id),
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
        // 起新一輪：內容回空閒，coarse 置 streaming。
        resetStreamStore()
        setStreamCoarse({ streaming: true })
        setStreamErrorForConversation(conversationId, '')
        activeRunIdRef.current = null
        streamStartedAtRef.current = startedAt
        streamingContentRef.current = ''
        streamingReasoningRef.current = ''
      }

      markConversationInFlight(conversationId)
      let persistedConversation: Conversation | null = null
      try {
        const updated = await chatApi.regenerateMessage(conversationId, messageId, trimmedNewContent)
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
          typeof err === 'string' ? err : (err as Error).message || '重新生成失敗',
        )
        clearStreamSnapshot(conversationId)
        if (currentConversationIdRef.current === conversationId) {
          void reloadConversation(conversationId)
        }
      } finally {
        clearConversationInFlight(conversationId)
        if (persistedConversation) {
          // 同 handleSend:已有持久化對話,丟棄延後的全量重拉,直接套用。
          delete pendingStreamDoneRef.current[conversationId]
          finishStreamingRunWithConversation(conversationId, persistedConversation)
        } else if (!(await flushPendingStreamDone(conversationId))) {
          clearStreamSnapshot(conversationId)
        }
      }
    },
    [applyAssistantStreamStats, applyConversation, clearConversationInFlight, clearStreamSnapshot, ensureStreamSnapshot, finishStreamingRunWithConversation, flushPendingStreamDone, markConversationInFlight, refreshSidebar, reloadConversation, resetLocalCancellation, setStreamErrorForConversation, syncGeneratingConversationIds],
  )

  const handleRuntimeChange = useCallback(async (runtime: AgentRuntimeConfig) => {
    setDraftAgentRuntime(runtime)
    if (!currentConversation) return
    try {
      const updated = await chatApi.setAgentRuntime(currentConversation.id, runtime)
      applyConversation(updated)
    } catch (err) {
      console.error('Failed to change agent runtime:', err)
      setStreamError(typeof err === 'string' ? err : (err as Error).message || 'Agent 切換失敗')
    }
  }, [applyConversation, currentConversation])

  const handleExternalModelChange = useCallback(async (model: string, reasoning?: string | null) => {
    // Route through handleRuntimeChange so the draft updates even before a conversation exists
    // (the draft is applied when the conversation is created on first send).
    const next: AgentRuntimeConfig = {
      ...activeAgentRuntime,
      kind: 'external',
      externalModel: model,
      externalReasoning: reasoning ?? activeAgentRuntime.externalReasoning ?? null,
    }
    await handleRuntimeChange(next)
  }, [activeAgentRuntime, handleRuntimeChange])

  const handleExternalSandboxChange = useCallback(async (sandbox: string) => {
    const next: AgentRuntimeConfig = {
      ...activeAgentRuntime,
      kind: 'external',
      externalSandbox: sandbox,
    }
    await handleRuntimeChange(next)
  }, [activeAgentRuntime, handleRuntimeChange])

  const handleModelChange = useCallback(async (providerId: string, model: string) => {
    setDraftProviderId(providerId)
    setDraftModel(model)

    if (!currentConversation) return

    try {
      const updatedConv = await chatApi.updateConversation(currentConversation.id, {
        providerId,
        model,
      })
      applyConversationMeta(updatedConv)
    } catch (err) {
      console.error('Failed to change model:', err)
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '模型切換失敗')
    }
  }, [applyConversationMeta, currentConversation])

  const handleThinkingLevelChange = useCallback(async (level: ThinkingLevel | null) => {
    setDraftThinkingLevel(level)
    if (!currentConversation) return
    try {
      const updatedConv = await chatApi.updateConversation(currentConversation.id, {
        thinkingLevel: level,
      })
      applyConversationMeta(updatedConv)
    } catch (err) {
      console.error('Failed to change thinking level:', err)
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '思考等級切換失敗')
    }
  }, [applyConversationMeta, currentConversation])

  // 多模型一問多答（任務 06-30 / D2）：變更多答模型集，持久化到會話（歡迎頁先存草稿）。
  // 上限 4 由 UI 側約束；這裡直落 chatApi.updateConversation({ replyModels })。
  const handleChangeReplyModels = useCallback(async (models: ModelRef[]) => {
    setDraftReplyModels(models)
    if (!currentConversation) return
    try {
      const updatedConv = await chatApi.updateConversation(currentConversation.id, {
        replyModels: models,
      })
      applyConversationMeta(updatedConv)
    } catch (err) {
      console.error('Failed to update reply models:', err)
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '多答模型更新失敗')
    }
  }, [applyConversationMeta, currentConversation])

  const handleChangeKnowledgeBaseIds = useCallback(async (ids: string[]) => {
    // the draft is applied when the conversation is created on first send.
    setDraftKnowledgeBaseIds(ids)
    if (!currentConversation) return
    try {
      const updatedConv = await chatApi.updateConversation(currentConversation.id, {
        knowledgeBaseIds: ids,
      })
      applyConversationMeta(updatedConv)
    } catch (err) {
      console.error('Failed to update knowledge bases:', err)
    }
  }, [applyConversationMeta, currentConversation])

  const handleCancelStream = useCallback(async () => {
    const conversationId = currentConversationIdRef.current
    if (
      !conversationId
      || getStreamCoarse().cancelling
      || !isConversationBusy(
        conversationId,
        inFlightConversationsRef.current,
        streamSnapshotsRef.current,
      )
    ) {
      return
    }

    setStreamCoarse({ cancelling: true })
    cancelCurrentRunLocally()
    try {
      await chatApi.cancelStream(conversationId)
    } catch (err) {
      console.error('Failed to cancel chat stream:', err)
      setStreamErrorForConversation(
        conversationId,
        typeof err === 'string' ? err : (err as Error).message || '停止生成失敗',
      )
    } finally {
      setStreamCoarse({ cancelling: false })
    }
  }, [cancelCurrentRunLocally, setStreamErrorForConversation])

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
  const showEmptyHero = chatView === 'conversation' && !hasMessages && !streamCoarse.streaming && !streamCoarse.streamError
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

  const handleSidebarSelectSet = useCallback((set: ChatSet | null) => {
    runAfterLeavingSettings(() => handleSelectSet(set))
  }, [handleSelectSet, runAfterLeavingSettings])

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
    // B3：側欄刪除時強制清掉該會話的 in-flight/快照/樂觀項，
    // 使樂觀合併不再保留它（刪"generating"會話也能立即從側欄消失）。
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
        {chatView !== 'onboarding' ? (
        <Sidebar
          currentConversationId={currentConversation?.id}
          generatingConversationIds={generatingConversationIds}
          optimisticConversations={optimisticSidebarConversations}
          selectedProject={selectedProject}
          onSelectProject={handleSidebarSelectProject}
          selectedSet={selectedSet}
          onSelectSet={handleSidebarSelectSet}
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
        ) : null}

        {chatView === 'onboarding' ? (
          <div className="chat-win-titlebar-safe flex min-h-0 min-w-0 flex-1 flex-col">
            <OnboardingShell
              onComplete={handleOnboardingExit}
              onSkip={handleOnboardingExit}
              onSettingsChange={onSettingsChange}
            />
          </div>
        ) : chatView === 'settings' ? (
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
                    onRuntimeChange={handleRuntimeChange}
                  />
                </div>
                <div className="min-w-0 max-w-full shrink" data-tauri-drag-region="false">
                  {usesExternalRuntime ? (
                    <ExternalModelSelector
                      agentRuntime={activeAgentRuntime}
                      onModelChange={handleExternalModelChange}
                    />
                  ) : (
                    <ModelSelector
                      currentProviderId={activeProviderId}
                      currentModel={activeModel}
                      onModelChange={handleModelChange}
                    />
                  )}
                </div>
                {!usesExternalRuntime && (
                  <div className="shrink-0" data-tauri-drag-region="false">
                    <ThinkingLevelSelector
                      currentProviderId={activeProviderId}
                      currentModel={activeModel}
                      value={
                        currentConversation
                          ? (currentConversation.thinking_level
                              ?? currentConversation.thinkingLevel
                              ?? null)
                          : draftThinkingLevel
                      }
                      onChange={handleThinkingLevelChange}
                    />
                  </div>
                )}
                <div className="shrink-0" data-tauri-drag-region="false">
                  <PermissionPicker
                    agentRuntime={activeAgentRuntime}
                    onSandboxChange={handleExternalSandboxChange}
                    approvalPolicy={approvalPolicy}
                    onApprovalPolicyChange={handleApprovalPolicyChange}
                  />
                </div>
                <div className="shrink-0" data-tauri-drag-region="false">
                  <BackgroundJobsIndicator />
                </div>
              </div>
              <div className="min-w-5 flex-1" data-tauri-drag-region />
              <div className="flex min-w-0 shrink items-center justify-end gap-1">
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
                      cancelVisible={streamCoarse.streaming}
                      cancelling={streamCoarse.cancelling}
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
                      enabledSkills={slashSkills}
                      onOpenSkillSettings={openSkillCenter}
                      selectedProject={selectedProject}
                      conversationProject={conversationProject}
                      onSelectProject={handleSidebarSelectProject}
                      showProjectEntry
                      currentAssistant={currentAssistantSnapshot ? { id: currentAssistantSnapshot.id, name: currentAssistantSnapshot.name } : null}
                      onOpenAssistantCenter={openAssistantCenter}
                      onClearAssistant={() => void handleApplyAssistant(null)}
                      autoFocus
                      usesExternalRuntime={usesExternalRuntime}
                      externalAgentName={activeAgentRuntime.externalAgentId ?? null}
                      conversationId={currentConversation?.id ?? null}
                      knowledgeBaseIds={currentConversation ? (currentConversation.knowledge_base_ids ?? currentConversation.knowledgeBaseIds ?? []) : draftKnowledgeBaseIds}
                      onChangeKnowledgeBaseIds={handleChangeKnowledgeBaseIds}
                      mcpServers={mcpServers}
                      onToggleMcpServer={handleToggleMcpServer}
                      replyModels={activeReplyModels}
                      onChangeReplyModels={handleChangeReplyModels}
                      contextSlot={
                        <ContextIndicator
                          contextState={contextState}
                          messageCount={displayMessages.length}
                          loading={contextLoading}
                          compressing={contextCompressing}
                          error={contextError}
                          usesExternalRuntime={usesExternalRuntime}
                          onRefresh={handleRefreshContext}
                          onCompress={() => void handleCompressContext()}
                          lang={uiLang}
                        />
                      }
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
                      assistantStreamStatsByMessageId={assistantStreamStatsByMessageId}
                      onUpdateMessage={handleUpdateMessage}
                      onRegenerateMessage={handleRegenerateMessage}
                      onDeleteMessage={handleDeleteMessage}
                      onRetryLastUser={handleRegenerateMessage}
                      onExecuteAgentPlan={handleExecuteAgentPlan}
                      groupSelections={currentConversation?.group_selections ?? currentConversation?.groupSelections ?? {}}
                      onSetGroupSelection={handleSetGroupSelection}
                      contextState={contextState}
                      compactionInProgress={contextCompressing || agentLoopCompacting}
                      animateCompactionBoundaryId={animateCompactionBoundaryId}
                      lang={uiLang}
                    />
                  </Suspense>
                  <InputBar
                    onSend={(content, attachments) => void handleSendMessage(content, attachments)}
                    disabled={isCurrentConversationBusy()}
                    onCancel={() => void handleCancelStream()}
                    cancelVisible={streamCoarse.streaming}
                    cancelling={streamCoarse.cancelling}
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
                    enabledSkills={slashSkills}
                    onOpenSkillSettings={openSkillCenter}
                    selectedProject={selectedProject}
                    conversationProject={conversationProject}
                    onSelectProject={handleSidebarSelectProject}
                    showProjectEntry
                    currentAssistant={currentAssistantSnapshot ? { id: currentAssistantSnapshot.id, name: currentAssistantSnapshot.name } : null}
                    onOpenAssistantCenter={openAssistantCenter}
                    onClearAssistant={() => void handleApplyAssistant(null)}
                    autoFocus
                    usesExternalRuntime={usesExternalRuntime}
                    externalAgentName={activeAgentRuntime.externalAgentId ?? null}
                    conversationId={currentConversation?.id ?? null}
                    knowledgeBaseIds={currentConversation ? (currentConversation.knowledge_base_ids ?? currentConversation.knowledgeBaseIds ?? []) : draftKnowledgeBaseIds}
                    onChangeKnowledgeBaseIds={handleChangeKnowledgeBaseIds}
                    mcpServers={mcpServers}
                    onToggleMcpServer={handleToggleMcpServer}
                    replyModels={activeReplyModels}
                    onChangeReplyModels={handleChangeReplyModels}
                    contextSlot={
                      <ContextIndicator
                        contextState={contextState}
                        messageCount={displayMessages.length}
                        loading={contextLoading}
                        compressing={contextCompressing}
                        error={contextError}
                        usesExternalRuntime={usesExternalRuntime}
                        onRefresh={handleRefreshContext}
                        onCompress={() => void handleCompressContext()}
                        lang={uiLang}
                      />
                    }
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
                  允許呼叫工具 {pendingToolConfirm.name}？
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
                aria-label="拒絕"
                onClick={() => resolvePendingToolConfirm(false)}
              >
                <X size={14} />
              </button>
            </div>
            {pendingToolConfirm.argumentsPreview && (
              <pre className="custom-scrollbar mb-3 max-h-40 overflow-auto rounded-md bg-neutral-100 p-3 text-[11px] leading-relaxed text-neutral-700 dark:bg-neutral-800 dark:text-neutral-200">
                {pendingToolConfirm.argumentsPreview}
              </pre>
            )}
            <div className="flex justify-end gap-2">
              <button
                type="button"
                className="rounded-md px-3 py-1.5 text-[12px] font-medium text-neutral-600 hover:bg-neutral-100 dark:text-neutral-300 dark:hover:bg-neutral-800"
                onClick={() => resolvePendingToolConfirm(false)}
              >
                拒絕
              </button>
              <button
                type="button"
                className="rounded-md bg-neutral-900 px-3 py-1.5 text-[12px] font-medium text-white hover:bg-neutral-700 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200"
                onClick={() => resolvePendingToolConfirm(true)}
              >
                允許
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
                  允許本次會話使用檔案和命令工具？
                </div>
                <div className="mt-1 text-[12px] text-neutral-500 dark:text-neutral-400">
                  授權後，本會話內 Kivio 可讀取、寫入、刪除磁碟上的任意檔案，並執行任意終端命令（包括專案目錄之外的位置）。僅本次會話有效，應用重啟後需重新授權。
                </div>
              </div>
              <button
                type="button"
                className="rounded-md p-1 text-neutral-400 hover:bg-neutral-100 hover:text-neutral-700 dark:hover:bg-neutral-800 dark:hover:text-neutral-200"
                aria-label="拒絕"
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
                拒絕
              </button>
              <button
                type="button"
                className="rounded-md bg-neutral-900 px-3 py-1.5 text-[12px] font-medium text-white hover:bg-neutral-700 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200"
                onClick={() => resolvePendingSessionConsent(true)}
              >
                允許本次會話
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
