import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Bot, Wrench, X } from 'lucide-react'
import { Sidebar } from './Sidebar'
import { AssistantCenter } from './AssistantCenter'
import { ChatTitlebarActions } from './ChatTitlebarActions'
import { MessageList, type AssistantStreamStats } from './MessageList'
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
  PendingAttachment,
  SkillMeta,
  ToolCallRecord,
} from './types'
import { api, type ChatToolConfirmPayload, type ChatToolDefinition, type ChatToolProgressPayload } from '../api/tauri'
import { SettingsShell, type SettingsShellHandle, type SettingsTab } from '../settings/SettingsShell'
import { useWindowInteractionFocus } from '../utils/windowFocus'
import { estimateTokens } from '../lens/markdown'
import { forgetRememberedChatRoute } from './persistence'
import { runPythonInSandbox } from './pyodideRunner'
import { pickRandomChatEmptyGreeting } from './utils'

type ChatView = 'conversation' | 'settings' | 'assistants'

interface ChatProps {
  onSettingsChange: () => void
}

interface ConversationStreamSnapshot {
  runId: string | null
  streaming: boolean
  content: string
  reasoning: string
  reasoningStreaming: boolean
  toolCalls: ToolCallRecord[]
  startedAt: number | null
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

function isLocallyCancelledPayload(
  payload: { conversationId: string; runId?: string },
  cancelledConversationId: string | null,
  cancelledRunId: string | null,
): boolean {
  if (cancelledConversationId !== payload.conversationId) return false
  return !cancelledRunId || !payload.runId || payload.runId === cancelledRunId
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
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false)
  const [searchOpen, setSearchOpen] = useState(false)
  const [selectedProject, setSelectedProject] = useState<ChatProject | null>(null)
  const [streaming, setStreaming] = useState(false)
  const [cancellingStream, setCancellingStream] = useState(false)
  const [streamingContent, setStreamingContent] = useState('')
  const [streamingReasoning, setStreamingReasoning] = useState('')
  const [reasoningStreaming, setReasoningStreaming] = useState(false)
  const [streamError, setStreamError] = useState('')
  /** 发送中待显示的用户消息（与 conversation 分离，避免 route reload 冲掉） */
  const [pendingUserMessage, setPendingUserMessage] = useState<ChatMessage | null>(null)
  const [pendingUserMessageConversationId, setPendingUserMessageConversationId] = useState<string | null>(null)
  const [lastAssistantStreamStats, setLastAssistantStreamStats] =
    useState<AssistantStreamStats | null>(null)
  const [sidebarRefreshKey, setSidebarRefreshKey] = useState(0)
  const [draftProviderId, setDraftProviderId] = useState('')
  const [draftModel, setDraftModel] = useState('')
  const [skills, setSkills] = useState<SkillMeta[]>([])
  const [disabledSkillIds, setDisabledSkillIds] = useState<string[]>([])
  const [settingsInitialTab, setSettingsInitialTab] = useState<SettingsTab>('chat')
  const [streamingToolCalls, setStreamingToolCalls] = useState<ToolCallRecord[]>([])
  const [enabledTools, setEnabledTools] = useState<ChatToolDefinition[]>([])
  const [enabledToolCount, setEnabledToolCount] = useState<number | null>(null)
  const [toolsDisabledReason, setToolsDisabledReason] = useState('')
  const [toolsRequested, setToolsRequested] = useState(false)
  const [pendingToolConfirm, setPendingToolConfirm] = useState<ChatToolConfirmPayload | null>(null)
  const [contextState, setContextState] = useState<ConversationContextState | null>(null)
  const [contextLoading, setContextLoading] = useState(false)
  const [contextCompressing, setContextCompressing] = useState(false)
  const [contextError, setContextError] = useState('')
  const currentConversationIdRef = useRef<string | null>(null)
  const activeRunIdRef = useRef<string | null>(null)
  const locallyCancelledConversationIdRef = useRef<string | null>(null)
  const locallyCancelledRunIdRef = useRef<string | null>(null)
  const sendInFlightRef = useRef(false)
  const pendingStreamDoneRef = useRef<(() => void) | null>(null)
  const streamSnapshotsRef = useRef<Record<string, ConversationStreamSnapshot>>({})
  const pendingToolConfirmsRef = useRef<Record<string, ChatToolConfirmPayload>>({})
  const streamStartedAtRef = useRef<number | null>(null)
  const streamingContentRef = useRef('')
  const streamingReasoningRef = useRef('')
  const newConversationRequestRef = useRef<Promise<Conversation> | null>(null)
  const reusableBlankConversationRef = useRef<Conversation | null>(null)
  const settingsRef = useRef<SettingsShellHandle>(null)
  const pendingAfterSettingsCloseRef = useRef<(() => void) | null>(null)
  const requestWindowFocus = useWindowInteractionFocus()

  const applyConversation = useCallback((conversation: Conversation | null) => {
    setCurrentConversation(conversation)
    setContextState(conversation?.context_state ?? conversation?.contextState ?? null)
    reusableBlankConversationRef.current =
      conversation
      && conversation.messages.length === 0
      && !(conversation.assistant_id ?? conversation.assistantId)
        ? conversation
        : null
  }, [])

  const patchContextState = useCallback((nextState: ConversationContextState) => {
    setContextState(nextState)
    setCurrentConversation((prev) => prev
      ? { ...prev, context_state: nextState, contextState: nextState }
      : prev)
  }, [])

  const clearStreamingPreview = useCallback(() => {
    setStreaming(false)
    setCancellingStream(false)
    setStreamingContent('')
    setStreamingReasoning('')
    setReasoningStreaming(false)
    setStreamingToolCalls([])
    activeRunIdRef.current = null
    streamStartedAtRef.current = null
    streamingContentRef.current = ''
    streamingReasoningRef.current = ''
  }, [])

  const ensureStreamSnapshot = useCallback((conversationId: string) => {
    const existing = streamSnapshotsRef.current[conversationId]
    if (existing) return existing
    const snapshot: ConversationStreamSnapshot = {
      runId: null,
      streaming: true,
      content: '',
      reasoning: '',
      reasoningStreaming: false,
      toolCalls: [],
      startedAt: Date.now(),
    }
    streamSnapshotsRef.current[conversationId] = snapshot
    return snapshot
  }, [])

  const restoreStreamingPreview = useCallback((conversationId: string | null) => {
    if (!conversationId) {
      clearStreamingPreview()
      setPendingToolConfirm(null)
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
      activeRunIdRef.current = snapshot.runId
      streamStartedAtRef.current = snapshot.startedAt
      streamingContentRef.current = snapshot.content
      streamingReasoningRef.current = snapshot.reasoning
    }
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
    activeRunIdRef.current = snapshot.runId
    streamStartedAtRef.current = snapshot.startedAt
    streamingContentRef.current = snapshot.content
    streamingReasoningRef.current = snapshot.reasoning
  }, [])

  const clearStreamSnapshot = useCallback((conversationId: string | null) => {
    if (!conversationId) return
    delete streamSnapshotsRef.current[conversationId]
    delete pendingToolConfirmsRef.current[conversationId]
    if (currentConversationIdRef.current === conversationId) {
      setPendingToolConfirm(null)
      clearStreamingPreview()
    }
  }, [clearStreamingPreview])

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

  const activeProviderId = currentConversation?.provider_id || draftProviderId
  const activeModel = currentConversation?.model || draftModel
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

  const canReuseCurrentBlankConversation = useCallback(() => {
    const candidate = currentConversation ?? reusableBlankConversationRef.current
    if (!candidate) return false
    if (candidate.messages.length > 0) return false
    if (candidate.assistant_id ?? candidate.assistantId) return false
    if ((candidate.folder ?? '') !== (selectedProject?.name ?? '')) return false
    return true
  }, [currentConversation, selectedProject?.name])
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
      return
    }
    try {
      const settings = await api.getSettings()
      const chatTools = settings.chatTools
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
        return
      }
      const provider = settings.providers.find((item) => item.id === activeProviderId)
      const anyMcpEnabled = chatTools.enabled && chatTools.servers.some((server) => server.enabled)
      const anyNativeEnabled = Boolean(chatTools.nativeTools?.webSearch)
      const skillRuntimeEnabled = Boolean(chatTools.nativeTools?.skillRuntime)
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
      setToolsDisabledReason(err instanceof Error ? err.message : String(err))
    }
  }, [activeProviderId, effectiveSkillId])

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
    void loadSkills()
  }, [loadDefaultModel, loadSkills])

  useEffect(() => {
    void refreshToolIndicator()
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
    pendingAfterSettingsCloseRef.current = action
    settingsRef.current?.requestClose()
  }, [chatView])

  const handleSettingsChange = useCallback(() => {
    onSettingsChange()
    void loadDefaultModel()
    void loadSkills()
    void refreshToolIndicator()
  }, [loadDefaultModel, loadSkills, onSettingsChange, refreshToolIndicator])

  const reloadConversation = useCallback(async (conversationId: string, options?: { force?: boolean }) => {
    if (sendInFlightRef.current && !options?.force) return
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
    (payload: { reason?: string; conversationId?: string }) => {
      const conversationId = payload.conversationId ?? currentConversationIdRef.current
      if (conversationId) {
        delete streamSnapshotsRef.current[conversationId]
      }
      if (conversationId && currentConversationIdRef.current === conversationId) {
        clearStreamingPreview()
      }
      if (payload.reason !== 'cancelled') {
        resetLocalCancellation()
      }
      if (payload.reason === 'error') {
        setStreamError((prev) => prev || '回复生成失败，请稍后重试。')
      }
      if (conversationId && payload.reason !== 'cancelled') {
        if (currentConversationIdRef.current === conversationId) {
          void reloadConversation(conversationId)
        }
        refreshSidebar()
      }
    },
    [clearStreamingPreview, refreshSidebar, reloadConversation, resetLocalCancellation],
  )

  const flushPendingStreamDone = useCallback(() => {
    const pending = pendingStreamDoneRef.current
    pendingStreamDoneRef.current = null
    pending?.()
  }, [])

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
        const snapshot = ensureStreamSnapshot(payload.conversationId)
        if (payload.runId) {
          if (snapshot.runId && snapshot.runId !== payload.runId) return
          snapshot.runId = payload.runId
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
        showStreamSnapshotIfCurrent(payload.conversationId, snapshot)
        if (payload.done) {
          // invoke 未完成前不要 reload；延后到 flushPendingStreamDone，避免与 send 写盘竞态。
          if (sendInFlightRef.current) {
            pendingStreamDoneRef.current = () => finishStreamingRun(payload)
            return
          }
          finishStreamingRun(payload)
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
  }, [ensureStreamSnapshot, finishStreamingRun, showStreamSnapshotIfCurrent])

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
        if (!sendInFlightRef.current) return
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
  }, [ensureStreamSnapshot, showStreamSnapshotIfCurrent])

  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined

    const setupListener = async () => {
      unlisten = await api.onChatToolConfirm((payload) => {
        if (cancelled) return
        pendingToolConfirmsRef.current[payload.conversationId] = payload
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
  }, [])

  const resolvePendingToolConfirm = useCallback((approved: boolean) => {
    if (!pendingToolConfirm) return
    delete pendingToolConfirmsRef.current[pendingToolConfirm.conversationId]
    void api.chatConfirmToolCall(pendingToolConfirm.toolCallId, approved)
    setPendingToolConfirm(null)
  }, [pendingToolConfirm])

  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined

    const setupListener = async () => {
      unlisten = await api.onChatRunPython((payload) => {
        if (cancelled) return
        void (async () => {
          try {
            const outcome = await runPythonInSandbox(payload.code, payload.timeoutMs)
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
              `Python 沙盒调用失败：${message || 'Unknown error'}。不要重试 run_python，也不要使用 run_command/pip 安装或修改本机 Python 环境来绕过沙盒；请直接基于已有数据回答，除非用户明确要求修改本机环境。`,
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
    if (canReuseCurrentBlankConversation()) {
      const conversation = currentConversation ?? reusableBlankConversationRef.current
      if (!conversation) return
      const conversationId = conversation.id
      currentConversationIdRef.current = conversationId
      applyConversation(conversation)
      restoreStreamingPreview(conversationId)
      syncConversationRoute(conversationId)
      setStreamError('')
      return
    }

    let request = newConversationRequestRef.current
    try {
      if (!request) {
        request = chatApi.createConversation(
          activeProviderId || undefined,
          activeModel || undefined,
          selectedProject?.name,
        )
        newConversationRequestRef.current = request
      }

      const conv = await request
      currentConversationIdRef.current = conv.id
      applyConversation(conv)
      restoreStreamingPreview(conv.id)
      syncConversationRoute(conv.id)
      refreshSidebar()
      setStreamError('')
    } catch (err) {
      console.error('Failed to create conversation:', err)
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '创建对话失败')
    } finally {
      if (newConversationRequestRef.current === request) {
        newConversationRequestRef.current = null
      }
    }
  }, [
    activeModel,
    activeProviderId,
    applyConversation,
    canReuseCurrentBlankConversation,
    currentConversation,
    refreshSidebar,
    restoreStreamingPreview,
    selectedProject?.name,
    syncConversationRoute,
  ])

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

  const handleSendMessage = async (content: string, attachments: PendingAttachment[] = []) => {
    if (streaming || sendInFlightRef.current) return

    const trimmed = content.trim()
    if (!trimmed && attachments.length === 0) return
    if (sendDisabledReason) {
      setStreamError(sendDisabledReason)
      return
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
    setStreaming(true)
    setCancellingStream(false)
    setStreamingContent('')
    setStreamingReasoning('')
    setReasoningStreaming(false)
    setStreamingToolCalls([])
    setStreamError('')
    activeRunIdRef.current = null
    streamStartedAtRef.current = Date.now()
    streamingContentRef.current = ''
    streamingReasoningRef.current = ''

    sendInFlightRef.current = true
    let conversationIdForRun: string | null = null
    try {
      let conversation = currentConversation
      if (!conversation) {
        conversation = await chatApi.createConversation(
          activeProviderId || undefined,
          activeModel || undefined,
          selectedProject?.name,
        )
        currentConversationIdRef.current = conversation.id
        applyConversation(conversation)
        syncConversationRoute(conversation.id)
      }

      const conversationId = conversation!.id
      conversationIdForRun = conversationId
      const snapshot = ensureStreamSnapshot(conversationId)
      snapshot.streaming = true
      snapshot.content = ''
      snapshot.reasoning = ''
      snapshot.reasoningStreaming = false
      snapshot.toolCalls = []
      snapshot.startedAt = streamStartedAtRef.current
      snapshot.runId = null
      setPendingUserMessage(optimisticUserMessage)
      setPendingUserMessageConversationId(conversationId)
      const updatedConv = await chatApi.sendMessage(
        conversationId,
        trimmed,
        attachments,
        null,
      )
      applyAssistantStreamStats(updatedConv)
      setPendingUserMessage(null)
      setPendingUserMessageConversationId(null)
      if (currentConversationIdRef.current === conversationId) {
        applyConversation(updatedConv)
        clearStreamSnapshot(conversationId)
        if (!locallyCancelledConversationIdRef.current) {
          resetLocalCancellation()
        }
        refreshSidebar()
      }
    } catch (err) {
      console.error('Failed to send message:', err)
      setPendingUserMessage(null)
      setPendingUserMessageConversationId(null)
      if (conversationIdForRun) {
        clearStreamSnapshot(conversationIdForRun)
      } else {
        clearStreamingPreview()
      }
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '发送失败')
    } finally {
      flushPendingStreamDone()
      sendInFlightRef.current = false
    }
  }

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
      if (!window.confirm('确定删除这条助手回复吗？')) return
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
      if (!currentConversation || streaming || sendInFlightRef.current) return

      const conversationId = currentConversation.id
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
      setStreaming(true)
      setCancellingStream(false)
      setStreamingContent('')
      setStreamingReasoning('')
      setReasoningStreaming(false)
      setStreamingToolCalls([])
      setStreamError('')
      activeRunIdRef.current = null
      streamStartedAtRef.current = Date.now()
      streamingContentRef.current = ''
      streamingReasoningRef.current = ''
      const snapshot = ensureStreamSnapshot(conversationId)
      snapshot.streaming = true
      snapshot.content = ''
      snapshot.reasoning = ''
      snapshot.reasoningStreaming = false
      snapshot.toolCalls = []
      snapshot.startedAt = streamStartedAtRef.current
      snapshot.runId = null
      sendInFlightRef.current = true

      try {
        const updated = await chatApi.regenerateMessage(conversationId, messageId)
        applyAssistantStreamStats(updated)
        if (currentConversationIdRef.current === conversationId) {
          applyConversation(updated)
          refreshSidebar()
        }
      } catch (err) {
        console.error('Failed to regenerate message:', err)
        setStreamError(typeof err === 'string' ? err : (err as Error).message || '重新生成失败')
        clearStreamSnapshot(conversationId)
        void reloadConversation(conversationId)
      } finally {
        flushPendingStreamDone()
        if (currentConversationIdRef.current === conversationId) {
          clearStreamingPreview()
        }
        sendInFlightRef.current = false
      }
    },
    [applyAssistantStreamStats, applyConversation, clearStreamSnapshot, clearStreamingPreview, currentConversation, ensureStreamSnapshot, flushPendingStreamDone, refreshSidebar, reloadConversation, resetLocalCancellation, streaming],
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
      refreshSidebar()
    } catch (err) {
      console.error('Failed to change model:', err)
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '模型切换失败')
    }
  }

  const handleCancelStream = useCallback(async () => {
    const conversationId = currentConversationIdRef.current
    if (!conversationId || cancellingStream || (!streaming && !sendInFlightRef.current)) return

    setCancellingStream(true)
    cancelCurrentRunLocally()
    try {
      await chatApi.cancelStream(conversationId)
    } catch (err) {
      console.error('Failed to cancel chat stream:', err)
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '停止生成失败')
    } finally {
      setCancellingStream(false)
    }
  }, [cancelCurrentRunLocally, cancellingStream, streaming])

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
  ).text

  const handleCollapseSidebar = useCallback(() => {
    setSidebarCollapsed(true)
  }, [])

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
    openEmbeddedSettings('chat')
  }, [openEmbeddedSettings])

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
      onPointerEnter={requestWindowFocus}
      onPointerMove={requestWindowFocus}
      onPointerDownCapture={requestWindowFocus}
    >
      <div className="flex h-full min-h-0 w-full">
        <Sidebar
          currentConversationId={currentConversation?.id}
          selectedProject={selectedProject}
          onSelectProject={handleSidebarSelectProject}
          onSelectConversation={handleSidebarSelectConversation}
          onNewConversation={handleSidebarNewConversation}
          onConversationDeleted={handleSidebarConversationDeleted}
          onOpenAssistantCenter={openAssistantCenter}
          onOpenSettings={handleSidebarOpenSettings}
          settingsActive={chatView === 'settings'}
          assistantCenterActive={chatView === 'assistants'}
          collapsed={sidebarCollapsed}
          onToggleCollapsed={handleCollapseSidebar}
          refreshKey={sidebarRefreshKey}
          searchOpen={searchOpen}
          onSearchOpenChange={handleSidebarSearchOpenChange}
        />

        {chatView === 'settings' ? (
          <div className="flex min-h-0 min-w-0 flex-1 flex-col">
            <SettingsShell
              ref={settingsRef}
              variant="embedded"
              initialTab={settingsInitialTab}
              reserveTrafficLightSpace={sidebarCollapsed && usesNativeTitlebar}
              onClose={handleSettingsClose}
              onSettingsChange={handleSettingsChange}
            />
          </div>
        ) : chatView === 'assistants' ? (
          <div className="flex min-h-0 min-w-0 flex-1 flex-col">
            <AssistantCenter
              skills={enabledSkills}
              currentAssistantId={currentAssistantId}
              onStartAssistantChat={(assistant) => void handleStartAssistantChat(assistant)}
              onApplyAssistant={currentConversation ? (assistantId) => void handleApplyAssistant(assistantId) : undefined}
              onClose={handleAssistantCenterClose}
            />
          </div>
        ) : (
          <div className="chat-main-pane relative flex min-w-0 flex-1 flex-col bg-white dark:bg-[#212121]">
            <header
              className={`${chatTitlebarRowClass} gap-2 ${
                sidebarCollapsed && usesNativeTitlebar ? chatTitlebarMacInsetClass : 'px-6'
              } ${sidebarCollapsed ? 'pr-4' : ''}`}
              data-tauri-drag-region
            >
              {!usesNativeTitlebar && <WindowControls />}
              {sidebarCollapsed && (
                <ChatTitlebarActions
                  sidebarExpanded={false}
                  onToggleSidebar={() => setSidebarCollapsed(false)}
                  onNewConversation={() => {
                    runAfterLeavingSettings(() => void handleNewConversation())
                  }}
                />
              )}
              <div data-tauri-drag-region="false">
                <ModelSelector
                  currentProviderId={activeProviderId}
                  currentModel={activeModel}
                  onModelChange={(providerId, model) => void handleModelChange(providerId, model)}
                />
              </div>
              {currentAssistantSnapshot && (
                <div className={chatTitlebarPillButtonClass} data-tauri-drag-region="false">
                  <button
                    type="button"
                    onClick={openAssistantCenter}
                    className="flex min-w-0 items-center gap-1.5"
                    title="打开助手中心"
                  >
                    <Bot size={15} className="shrink-0 text-neutral-500 dark:text-neutral-400" />
                    <span className="max-w-[150px] truncate text-[13px] font-medium text-neutral-800 dark:text-neutral-200">
                      {currentAssistantSnapshot.name}
                    </span>
                  </button>
                  <span
                    aria-hidden
                    className="h-4 w-px shrink-0 bg-neutral-200/90 dark:bg-neutral-700"
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
              <ContextIndicator
                contextState={contextState}
                messageCount={displayMessages.length}
                loading={contextLoading}
                compressing={contextCompressing}
                error={contextError}
                onRefresh={handleRefreshContext}
                onCompress={() => void handleCompressContext()}
              />
              <div className="min-w-0 flex-1" data-tauri-drag-region />
            </header>

            <div className="flex min-h-0 flex-1 flex-col">
              {showEmptyHero ? (
                <div className="flex flex-1 flex-col items-center justify-center px-6 pb-16">
                  <div className="w-full max-w-3xl space-y-8">
                    <h2 className="chat-empty-hero-title text-center text-[1.75rem] leading-snug tracking-tight text-neutral-900 dark:text-neutral-50 sm:text-[2rem]">
                      {currentAssistantSnapshot
                        ? currentAssistantSnapshot.name
                        : selectedProject
                          ? `开始「${selectedProject.name}」`
                          : emptyHeroGreeting}
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
                                disabled={streaming || sendInFlightRef.current}
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
                      disabled={streaming || sendInFlightRef.current}
                      onCancel={() => void handleCancelStream()}
                      cancelVisible={streaming || sendInFlightRef.current}
                      cancelling={cancellingStream}
                      onOpenSettings={() => openEmbeddedSettings('chat')}
                      enabledTools={enabledTools}
                      toolsDisabledReason={toolsDisabledReason}
                      toolStatusHint={toolStatusHint}
                      sendDisabledReason={sendDisabledReason}
                      enabledSkills={enabledSkills.map((skill) => ({ id: skill.id, name: skill.name }))}
                      onOpenSkillSettings={() => openEmbeddedSettings('skill')}
                      autoFocus
                    />
                  </div>
                </div>
              ) : (
                <>
                  <MessageList
                    conversationId={currentConversation?.id}
                    messages={displayMessages}
                    streaming={streaming}
                    streamingContent={streamingContent}
                    streamingReasoning={streamingReasoning}
                    reasoningStreaming={reasoningStreaming}
                    streamingToolCalls={streamingToolCalls}
                    error={streamError}
                    lastAssistantStreamStats={lastAssistantStreamStats}
                    onUpdateMessage={handleUpdateMessage}
                    onRegenerateMessage={handleRegenerateMessage}
                    onDeleteMessage={handleDeleteMessage}
                  />
                  <InputBar
                    onSend={(content, attachments) => void handleSendMessage(content, attachments)}
                    disabled={streaming || sendInFlightRef.current}
                    onCancel={() => void handleCancelStream()}
                    cancelVisible={streaming || sendInFlightRef.current}
                    cancelling={cancellingStream}
                    onOpenSettings={() => openEmbeddedSettings('chat')}
                    enabledTools={enabledTools}
                    toolsDisabledReason={toolsDisabledReason}
                    toolStatusHint={toolStatusHint}
                    sendDisabledReason={sendDisabledReason}
                    enabledSkills={enabledSkills.map((skill) => ({ id: skill.id, name: skill.name }))}
                    onOpenSkillSettings={() => openEmbeddedSettings('skill')}
                    autoFocus
                  />
                </>
              )}
            </div>
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
