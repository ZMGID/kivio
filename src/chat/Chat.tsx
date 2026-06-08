import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Bot, Wrench, X } from 'lucide-react'
import { Sidebar, type ExtensionsNavItem } from './Sidebar'
import { ChatImageViewer } from './ChatImageViewer'
import { ChatTitlebarActions } from './ChatTitlebarActions'
import type { AssistantStreamStats } from './MessageList'
import { InputBar } from './InputBar'
import { ModelSelector } from './ModelSelector'
import { WindowControls } from './WindowControls'
import { ContextIndicator } from './ContextIndicator'
import { chatApi } from './api'
import {
  chatTitlebarMacInsetClass,
  chatTitlebarPillButtonClass,
  chatTitlebarPillIconClass,
  chatTitlebarRowClass,
  usesNativeTitlebar,
} from './platform'
import type {
  ChatProject,
  ChatMessage,
  ChatAssistant,
  Conversation,
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
  type ChatStreamPayload,
  type ChatToolConfirmPayload,
  type ChatToolDefinition,
  type ChatToolProgressPayload,
} from '../api/tauri'
import type { SettingsShellHandle, SettingsTab } from '../settings/SettingsShell'
import { estimateTokens } from '../utils/tokens'
import {
  CHAT_MIN_SIZE_COLLAPSED,
  CHAT_MIN_SIZE_EXPANDED,
  forgetRememberedChatRoute,
  getRememberedChatSidebarCollapsed,
  rememberChatSidebarCollapsed,
  rememberChatSize,
} from './persistence'
import { ChatDotGridBackground } from './ChatDotGridBackground'
import { TypewriterText } from './TypewriterText'
import { pickRandomChatEmptyGreeting } from './utils'
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

const MessageList = lazy(() => import('./MessageList').then((module) => ({
  default: module.MessageList,
})))

function ChatPaneLoading() {
  return (
    <div className="flex h-full w-full items-center justify-center bg-white dark:bg-[#212121]">
      <div className="h-5 w-5 animate-spin rounded-full border-2 border-neutral-300 border-t-neutral-800 dark:border-neutral-700 dark:border-t-neutral-200" />
    </div>
  )
}

function MessageListLoading() {
  return (
    <div className="flex flex-1 items-center justify-center bg-white dark:bg-[#212121]">
      <div className="h-5 w-5 animate-spin rounded-full border-2 border-neutral-300 border-t-neutral-800 dark:border-neutral-700 dark:border-t-neutral-200" />
    </div>
  )
}

type ChatView = 'conversation' | 'settings' | 'assistants'

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

function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
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
    status: payload.status === 'success' ? 'completed' : payload.status,
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

type SendMessageOptions = {
  forceNewConversation?: boolean
  conversationOverride?: Conversation | null
}

export default function Chat({ onSettingsChange }: ChatProps) {
  const [chatView, setChatView] = useState<ChatView>(() => {
    const path = hashPath()
    if (isChatSettingsPath(path)) return 'settings'
    if (isChatAssistantCenterPath(path)) return 'assistants'
    return 'conversation'
  })
  const [currentConversation, setCurrentConversation] = useState<Awaited<
    ReturnType<typeof chatApi.getConversation>
  > | null>(null)
  const [sidebarCollapsed, setSidebarCollapsed] = useState(() => getRememberedChatSidebarCollapsed())
  const [searchOpen, setSearchOpen] = useState(false)
  const [selectedProject, setSelectedProject] = useState<ChatProject | null>(null)
  const [streaming, setStreaming] = useState(false)
  const [cancellingStream, setCancellingStream] = useState(false)
  const [streamingContent, setStreamingContent] = useState('')
  const [streamingReasoning, setStreamingReasoning] = useState('')
  const [reasoningStreaming, setReasoningStreaming] = useState(false)
  const [streamError, setStreamError] = useState('')
  const [streamingSegments, setStreamingSegments] = useState<ChatMessageSegment[]>([])
  /** 发送中待显示的用户消息（与 conversation 分离，避免 route reload 冲掉） */
  const [pendingUserMessage, setPendingUserMessage] = useState<ChatMessage | null>(null)
  const [pendingUserMessageConversationId, setPendingUserMessageConversationId] = useState<string | null>(null)
  const [lastAssistantStreamStats, setLastAssistantStreamStats] =
    useState<AssistantStreamStats | null>(null)
  const [sidebarRefreshKey, setSidebarRefreshKey] = useState(0)
  const [generatingConversationIds, setGeneratingConversationIds] = useState<ReadonlySet<string>>(
    () => new Set(),
  )
  const [sidebarProfileRefreshKey, setSidebarProfileRefreshKey] = useState(0)
  const [draftProviderId, setDraftProviderId] = useState('')
  const [draftModel, setDraftModel] = useState('')
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
  const streamStartedAtRef = useRef<number | null>(null)
  const streamingContentRef = useRef('')
  const streamingReasoningRef = useRef('')
  const settingsRef = useRef<SettingsShellHandle>(null)
  const pendingAfterSettingsCloseRef = useRef<(() => void) | null>(null)

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
    setStreaming(false)
    setCancellingStream(false)
    setStreamingContent('')
    setStreamingReasoning('')
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
    if (!conversationId) {
      clearStreamingPreview()
      setPendingToolConfirm(null)
      setStreamError('')
      return
    }
    const snapshot = streamSnapshotsRef.current[conversationId]
    if (!snapshot) {
      clearStreamingPreview()
    } else {
      setStreaming(snapshot.streaming)
      setCancellingStream(false)
      setStreamingContent(snapshot.content)
      setStreamingReasoning(snapshot.reasoning)
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
  }, [clearStreamingPreview])

  const showStreamSnapshotIfCurrent = useCallback((
    conversationId: string,
    snapshot: ConversationStreamSnapshot,
  ) => {
    if (currentConversationIdRef.current !== conversationId) return
    setStreaming(snapshot.streaming)
    setCancellingStream(false)
    setStreamingContent(snapshot.content)
    setStreamingReasoning(snapshot.reasoning)
    setReasoningStreaming(snapshot.reasoningStreaming)
    setStreamingToolCalls(snapshot.toolCalls)
    setStreamingSegments(snapshot.segments)
    activeRunIdRef.current = snapshot.runId
    streamStartedAtRef.current = snapshot.startedAt
    streamingContentRef.current = snapshot.content
    streamingReasoningRef.current = snapshot.reasoning
  }, [])

  const clearStreamSnapshot = useCallback((conversationId: string | null) => {
    if (!conversationId) return
    delete streamSnapshotsRef.current[conversationId]
    delete pendingToolConfirmsRef.current[conversationId]
    syncGeneratingConversationIds()
    if (currentConversationIdRef.current === conversationId) {
      setPendingToolConfirm(null)
      clearStreamingPreview()
    }
  }, [clearStreamingPreview, syncGeneratingConversationIds])

  const cancelCurrentRunLocally = useCallback(() => {
    locallyCancelledConversationIdRef.current = currentConversationIdRef.current
    locallyCancelledRunIdRef.current = activeRunIdRef.current
    clearStreamSnapshot(currentConversationIdRef.current)
    clearStreamingPreview()
  }, [clearStreamSnapshot, clearStreamingPreview])

  const resetLocalCancellation = useCallback(() => {
    locallyCancelledConversationIdRef.current = null
    locallyCancelledRunIdRef.current = null
  }, [])

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

  const openExtensionsItem = useCallback((item: ExtensionsNavItem) => {
    setExtensionsNavItem(item)
    if (item === 'assistants') {
      openAssistantCenter()
      return
    }
    openEmbeddedSettings(item)
  }, [openAssistantCenter, openEmbeddedSettings])

  const extensionsActive = useMemo<ExtensionsNavItem | null>(() => {
    if (chatView === 'assistants') return 'assistants'
    if (chatView === 'settings' && extensionsNavItem === 'skill') return 'skill'
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
      forgetRememberedChatRoute()
      currentConversationIdRef.current = null
      applyConversation(null)
      syncConversationRoute(null)
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '对话加载失败')
    }
  }, [applyConversation, restoreStreamingPreview, syncConversationRoute])

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
        applyConversation(result.conversation)
        setContextState(result.contextState)
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
  }, [applyConversation])

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
        syncGeneratingConversationIds()
      }
      if (conversationId && currentConversationIdRef.current === conversationId) {
        setPendingToolConfirm(null)
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
    }
    delete streamSnapshotsRef.current[conversationId]
    delete pendingToolConfirmsRef.current[conversationId]
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
          snapshot.streaming = true
          snapshot.reasoningStreaming = true
          snapshot.reasoning += payload.reasoningDelta
        }
        if (payload.delta) {
          snapshot.streaming = true
          snapshot.reasoningStreaming = false
          snapshot.content += payload.delta
        }
        syncGeneratingConversationIds()
        showStreamSnapshotIfCurrent(payload.conversationId, snapshot)
        if (payload.done) {
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
    setLastAssistantStreamStats(null)
    try {
      const conv = await chatApi.getConversation(conversationId)
      currentConversationIdRef.current = conversationId
      applyConversation(conv)
      restoreStreamingPreview(conversationId)
      syncConversationRoute(conversationId)
      setStreamError('')
    } catch (err) {
      console.error('Failed to load conversation:', err)
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '对话加载失败')
    }
  }, [applyConversation, restoreStreamingPreview, syncConversationRoute])

  const handleNewConversation = useCallback(async () => {
    setLastAssistantStreamStats(null)
    setDraftProviderId(activeProviderId)
    setDraftModel(activeModel)
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
      setLastAssistantStreamStats(null)
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
      delete streamErrorsRef.current[conversationId]
      clearConversationInFlight(conversationId)
      forgetRememberedChatRoute()
      currentConversationIdRef.current = null
      setLastAssistantStreamStats(null)
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
    setLastAssistantStreamStats(null)
    try {
      const assistantProviderId = assistant.provider_id ?? assistant.providerId ?? ''
      const assistantModel = assistant.model ?? ''
      const conv = await chatApi.createConversation(
        assistantProviderId || activeProviderId || undefined,
        assistantModel || activeModel || undefined,
        selectedProject?.name,
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
  }, [activeModel, activeProviderId, applyConversation, refreshSidebar, restoreStreamingPreview, selectedProject?.name, syncConversationRoute])

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
    const conversation = await chatApi.createConversation(
      activeProviderId || undefined,
      activeModel || undefined,
      selectedProject?.name,
    )
    currentConversationIdRef.current = conversation.id
    applyConversation(conversation)
    syncConversationRoute(conversation.id)
    refreshSidebar()
    return conversation
  }, [activeModel, activeProviderId, applyConversation, currentConversation, refreshSidebar, selectedProject?.name, syncConversationRoute])

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
    setLastAssistantStreamStats(null)
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
    setLastAssistantStreamStats({
      messageId: lastAssistant.id,
      tokensPerSec: tokenEstimate / elapsedSec,
    })
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

    const conversationId = conversation.id
    if (isConversationInFlight(inFlightConversationsRef.current, conversationId)) {
      setStreamErrorForConversation(conversationId, '该对话正在生成中，请稍后再试')
      return false
    }

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
    snapshot.runId = null
    syncGeneratingConversationIds()

    if (currentConversationIdRef.current === conversationId) {
      setStreaming(true)
      setCancellingStream(false)
      setStreamingContent('')
      setStreamingReasoning('')
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
      clearStreamSnapshot(conversationId)
      const message = typeof err === 'string' ? err : (err as Error).message || '发送失败'
      setStreamErrorForConversation(conversationId, message)
    } finally {
      clearConversationInFlight(conversationId)
      if (!(await flushPendingStreamDone(conversationId))) {
        if (persistedConversation) {
          finishStreamingRunWithConversation(conversationId, persistedConversation)
        } else {
          clearStreamSnapshot(conversationId)
        }
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
    effectiveSkillId,
    enabledSkills,
    ensureStreamSnapshot,
    finishStreamingRunWithConversation,
    flushPendingStreamDone,
    markConversationInFlight,
    refreshSidebar,
    resetLocalCancellation,
    selectedProject?.name,
    sendDisabledReason,
    setStreamErrorForConversation,
    syncConversationRoute,
    syncGeneratingConversationIds,
  ])

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
        const attachments = (request.attachments ?? [])
          .filter((attachment) => attachment.path)
          .map<PendingAttachment>((attachment, index) => ({
            id: attachment.id || `external-${request.id}-${index}`,
            type: attachment.type === 'file' ? 'file' : 'image',
            name: attachment.name || (attachment.type === 'file' ? 'Attachment' : 'Image'),
            path: attachment.path,
          }))
        const accepted = await handleSendMessage(
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
  }, [handleSendMessage])

  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined

    void drainExternalSends()
    api.onChatExternalSendReady(() => {
      if (cancelled) return
      void drainExternalSends()
    }).then((dispose) => {
      if (cancelled) dispose()
      else unlisten = dispose
    }).catch(err => console.error(err))

    return () => {
      cancelled = true
      unlisten?.()
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
        setLastAssistantStreamStats((prev) =>
          prev?.messageId === messageId ? null : prev,
        )
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
      setLastAssistantStreamStats(null)
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
      snapshot.runId = null
      syncGeneratingConversationIds()

      if (currentConversationIdRef.current === conversationId) {
        setStreaming(true)
        setCancellingStream(false)
        setStreamingContent('')
        setStreamingReasoning('')
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
        if (!(await flushPendingStreamDone(conversationId))) {
          if (persistedConversation) {
            finishStreamingRunWithConversation(conversationId, persistedConversation)
          } else {
            clearStreamSnapshot(conversationId)
          }
        }
      }
    },
    [applyAssistantStreamStats, applyConversation, clearConversationInFlight, clearStreamSnapshot, currentConversation, ensureStreamSnapshot, finishStreamingRunWithConversation, flushPendingStreamDone, markConversationInFlight, refreshSidebar, reloadConversation, resetLocalCancellation, setStreamErrorForConversation, syncGeneratingConversationIds],
  )

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
      const min = sidebarCollapsed ? CHAT_MIN_SIZE_COLLAPSED : CHAT_MIN_SIZE_EXPANDED
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
          selectedProject={selectedProject}
          onSelectProject={handleSidebarSelectProject}
          onSelectConversation={handleSidebarSelectConversation}
          onNewConversation={handleSidebarNewConversation}
          onConversationDeleted={handleSidebarConversationDeleted}
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
                onApplyAssistant={currentConversation ? (assistantId) => void handleApplyAssistant(assistantId) : undefined}
                onClose={handleAssistantCenterClose}
              />
            </Suspense>
          </div>
        ) : (
          <div className="chat-main-pane relative flex min-w-0 flex-1 flex-col bg-white dark:bg-[#212121]">
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
              <div className="flex min-w-0 items-center gap-1.5">
                <div className="min-w-0 max-w-full shrink" data-tauri-drag-region="false">
                  <ModelSelector
                    currentProviderId={activeProviderId}
                    currentModel={activeModel}
                    onModelChange={(providerId, model) => void handleModelChange(providerId, model)}
                  />
                </div>
                {currentAssistantSnapshot && (
                  <div className={`${chatTitlebarPillButtonClass} chat-titlebar-assistant-pill min-w-0 shrink`} data-tauri-drag-region="false">
                    <button
                      type="button"
                      onClick={openAssistantCenter}
                      className="flex min-w-0 items-center gap-1.5"
                      title={currentAssistantSnapshot.name}
                    >
                      <Bot size={15} className="shrink-0 text-neutral-500 dark:text-neutral-400" />
                      <span className="chat-titlebar-assistant-label max-w-[150px] truncate text-[13px] font-medium text-neutral-800 dark:text-neutral-200">
                        {currentAssistantSnapshot.name}
                      </span>
                    </button>
                    <span
                      aria-hidden
                      className="chat-titlebar-assistant-divider h-4 w-px shrink-0 bg-neutral-200/90 dark:bg-neutral-700"
                    />
                    <button
                      type="button"
                      onClick={() => void handleApplyAssistant(null)}
                      className={chatTitlebarPillIconClass}
                      title="清除助手"
                      aria-label="清除助手"
                    >
                      <X size={13} strokeWidth={1.9} />
                    </button>
                  </div>
                )}
                <div className="shrink-0" data-tauri-drag-region="false">
                  <ContextIndicator
                    contextState={contextState}
                    messageCount={displayMessages.length}
                    loading={contextLoading}
                    compressing={contextCompressing}
                    error={contextError}
                    onRefresh={handleRefreshContext}
                    onCompress={() => void handleCompressContext()}
                  />
                </div>
              </div>
              <div className="min-w-5 flex-1" data-tauri-drag-region />
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
                    {(currentAssistantSnapshot?.greeting || currentAssistantSnapshot?.conversation_starters?.length) && (
                      <div className="space-y-3 text-center">
                        {currentAssistantSnapshot.greeting && (
                          <p className="mx-auto max-w-xl text-[13px] leading-relaxed text-neutral-500 dark:text-neutral-400">
                            {currentAssistantSnapshot.greeting}
                          </p>
                        )}
                        {currentAssistantSnapshot.conversation_starters && currentAssistantSnapshot.conversation_starters.length > 0 && (
                          <div className="mx-auto flex max-w-2xl flex-wrap justify-center gap-2">
                            {currentAssistantSnapshot.conversation_starters.slice(0, 4).map((starter) => (
                              <button
                                key={starter}
                                type="button"
                                onClick={() => void handleSendMessage(starter)}
                                disabled={isCurrentConversationBusy()}
                                className="max-w-full truncate rounded-full border border-neutral-200 bg-white px-3 py-1.5 text-[12px] text-neutral-700 shadow-sm transition hover:border-neutral-300 hover:bg-neutral-50 disabled:cursor-not-allowed disabled:opacity-50 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-300 dark:hover:bg-neutral-800"
                              >
                                {starter}
                              </button>
                            ))}
                          </div>
                        )}
                      </div>
                    )}
                    <InputBar
                      layout="inline"
                      onSend={(content, attachments) => void handleSendMessage(content, attachments)}
                      disabled={isCurrentConversationBusy()}
                      onCancel={() => void handleCancelStream()}
                      cancelVisible={isCurrentConversationBusy()}
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
                      approvalPolicy={approvalPolicy}
                      onApprovalPolicyChange={handleApprovalPolicyChange}
                      agentPlanState={currentConversation?.agent_plan_state ?? currentConversation?.agentPlanState ?? null}
                      onAgentPlanModeChange={handleAgentPlanModeChange}
                      onExecuteAgentPlan={handleExecuteAgentPlan}
                      enabledSkills={enabledSkills.map((skill) => ({ id: skill.id, name: skill.name }))}
                      onOpenSkillSettings={() => openEmbeddedSettings('skill')}
                      autoFocus
                    />
                  </div>
                </div>
                  ) : (
                    <>
                  <Suspense fallback={<MessageListLoading />}>
                    <MessageList
                      conversationId={currentConversation?.id}
                      messages={displayMessages}
                      agentPlanState={currentConversation?.agent_plan_state ?? currentConversation?.agentPlanState ?? null}
                      agentTodoState={currentConversation?.agent_todo_state ?? currentConversation?.agentTodoState ?? null}
                      streaming={streaming}
                      streamingContent={streamingContent}
                      streamingReasoning={streamingReasoning}
                      reasoningStreaming={reasoningStreaming}
                      streamingToolCalls={streamingToolCalls}
                      streamingSegments={streamingSegments}
                      error={streamError}
                      lastAssistantStreamStats={lastAssistantStreamStats}
                      onUpdateMessage={handleUpdateMessage}
                      onRegenerateMessage={handleRegenerateMessage}
                      onDeleteMessage={handleDeleteMessage}
                    />
                  </Suspense>
                  <InputBar
                    onSend={(content, attachments) => void handleSendMessage(content, attachments)}
                    disabled={isCurrentConversationBusy()}
                    onCancel={() => void handleCancelStream()}
                    cancelVisible={isCurrentConversationBusy()}
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
                    approvalPolicy={approvalPolicy}
                    onApprovalPolicyChange={handleApprovalPolicyChange}
                    agentPlanState={currentConversation?.agent_plan_state ?? currentConversation?.agentPlanState ?? null}
                    onAgentPlanModeChange={handleAgentPlanModeChange}
                    onExecuteAgentPlan={handleExecuteAgentPlan}
                    enabledSkills={enabledSkills.map((skill) => ({ id: skill.id, name: skill.name }))}
                    onOpenSkillSettings={() => openEmbeddedSettings('skill')}
                    autoFocus
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
    </div>
  )
}
