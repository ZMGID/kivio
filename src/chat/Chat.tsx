import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Wrench, X } from 'lucide-react'
import { Sidebar } from './Sidebar'
import { ChatTitlebarActions } from './ChatTitlebarActions'
import { MessageList, type AssistantStreamStats } from './MessageList'
import { InputBar } from './InputBar'
import { ModelSelector } from './ModelSelector'
import { WindowControls } from './WindowControls'
import { chatApi } from './api'
import { chatTitlebarMacInsetClass, chatTitlebarRowClass, usesNativeTitlebar } from './platform'
import type { ChatMessage, Conversation, PendingAttachment, SkillMeta, ToolCallRecord } from './types'
import { api, type ChatToolConfirmPayload, type ChatToolDefinition, type ChatToolProgressPayload } from '../api/tauri'
import { SettingsShell, type SettingsShellHandle, type SettingsTab } from '../settings/SettingsShell'
import { useWindowInteractionFocus } from '../utils/windowFocus'
import { estimateTokens } from '../lens/markdown'
import { forgetRememberedChatRoute } from './persistence'

type ChatView = 'conversation' | 'settings'

interface ChatProps {
  onSettingsChange: () => void
}

function hashPath(): string {
  return window.location.hash.replace('#', '').split('?')[0]
}

function isChatSettingsPath(path: string): boolean {
  return path === 'chat/settings' || path.startsWith('chat/settings/')
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

export default function Chat({ onSettingsChange }: ChatProps) {
  const [chatView, setChatView] = useState<ChatView>(() =>
    isChatSettingsPath(hashPath()) ? 'settings' : 'conversation',
  )
  const [currentConversation, setCurrentConversation] = useState<Awaited<
    ReturnType<typeof chatApi.getConversation>
  > | null>(null)
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false)
  const [searchOpen, setSearchOpen] = useState(false)
  const [streaming, setStreaming] = useState(false)
  const [cancellingStream, setCancellingStream] = useState(false)
  const [streamingContent, setStreamingContent] = useState('')
  const [streamingReasoning, setStreamingReasoning] = useState('')
  const [reasoningStreaming, setReasoningStreaming] = useState(false)
  const [streamError, setStreamError] = useState('')
  /** 发送中待显示的用户消息（与 conversation 分离，避免 route reload 冲掉） */
  const [pendingUserMessage, setPendingUserMessage] = useState<ChatMessage | null>(null)
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
  const currentConversationIdRef = useRef<string | null>(null)
  const activeRunIdRef = useRef<string | null>(null)
  const sendInFlightRef = useRef(false)
  const pendingStreamDoneRef = useRef<(() => void) | null>(null)
  const streamStartedAtRef = useRef<number | null>(null)
  const streamingContentRef = useRef('')
  const streamingReasoningRef = useRef('')
  const settingsRef = useRef<SettingsShellHandle>(null)
  const pendingAfterSettingsCloseRef = useRef<(() => void) | null>(null)
  const requestWindowFocus = useWindowInteractionFocus()

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
    if (enabledSkills.length === 1) return enabledSkills[0].id
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

  const refreshSidebar = useCallback(() => {
    setSidebarRefreshKey((key) => key + 1)
  }, [])

  const loadDefaultModel = useCallback(async () => {
    try {
      const settings = await api.getSettings()
      setDraftProviderId(settings.chatProviderId || settings.translatorProviderId || '')
      setDraftModel(settings.chatModel || settings.translatorModel || '')
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

  const handleSettingsClose = useCallback(() => {
    setChatView('conversation')
    syncConversationRoute(currentConversationIdRef.current)
    void loadSkills()
    void refreshToolIndicator()
    const pending = pendingAfterSettingsCloseRef.current
    pendingAfterSettingsCloseRef.current = null
    pending?.()
  }, [loadSkills, refreshToolIndicator, syncConversationRoute])

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

  const reloadConversation = useCallback(async (conversationId: string) => {
    if (sendInFlightRef.current) return
    try {
      const conv = await chatApi.getConversation(conversationId)
      setCurrentConversation(conv)
      setStreamingToolCalls([])
      setCancellingStream(false)
      activeRunIdRef.current = null
    } catch (err) {
      console.error('Failed to reload conversation:', err)
      forgetRememberedChatRoute()
      currentConversationIdRef.current = null
      setCurrentConversation(null)
      syncConversationRoute(null)
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '对话加载失败')
    }
  }, [syncConversationRoute])

  const finishStreamingRun = useCallback(
    (payload: { reason?: string; conversationId?: string }) => {
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
      if (payload.reason === 'error') {
        setStreamError('回复生成失败，请稍后重试。')
      }
      const conversationId = currentConversationIdRef.current
      if (conversationId && payload.reason !== 'cancelled') {
        void reloadConversation(conversationId)
        refreshSidebar()
      }
    },
    [refreshSidebar, reloadConversation],
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
        const currentConversationId = currentConversationIdRef.current
        if (!currentConversationId || payload.conversationId !== currentConversationId) {
          return
        }
        if (payload.runId) {
          if (activeRunIdRef.current && activeRunIdRef.current !== payload.runId) return
          activeRunIdRef.current = payload.runId
        }
        if (payload.reasoningDelta) {
          setReasoningStreaming(true)
          streamingReasoningRef.current += payload.reasoningDelta
          setStreamingReasoning((prev) => prev + payload.reasoningDelta)
        }
        if (payload.delta) {
          setReasoningStreaming(false)
          streamingContentRef.current += payload.delta
          setStreamingContent((prev) => prev + payload.delta)
        }
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
  }, [finishStreamingRun, refreshSidebar, reloadConversation])

  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined

    const setupListener = async () => {
      unlisten = await api.onChatTool((payload) => {
        if (cancelled) return
        const currentConversationId = currentConversationIdRef.current
        if (!currentConversationId || payload.conversationId !== currentConversationId) {
          return
        }
        // 忽略 invoke 结束后的迟到 tool 事件，否则会重新 setStreaming(true) 卡死输入栏。
        if (!sendInFlightRef.current) return
        if (payload.runId) {
          if (activeRunIdRef.current && activeRunIdRef.current !== payload.runId) return
          activeRunIdRef.current = payload.runId
        }
        setStreaming(true)
        setReasoningStreaming(false)
        const record = toolEventToRecord(payload)
        setStreamingToolCalls((prev) => {
          const index = prev.findIndex((item) => item.id === record.id)
          if (index < 0) return [...prev, record]
          return prev.map((item, i) => (i === index ? { ...item, ...record } : item))
        })
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
      unlisten = await api.onChatToolConfirm((payload) => {
        if (cancelled) return
        const currentConversationId = currentConversationIdRef.current
        if (!currentConversationId || payload.conversationId !== currentConversationId) {
          void api.chatConfirmToolCall(payload.toolCallId, false)
          return
        }
        setPendingToolConfirm(payload)
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
      setChatView('conversation')
      if (sendInFlightRef.current) return
      const conversationId = getRouteConversationId()
      if (!conversationId) {
        setCurrentConversation(null)
        return
      }
      void reloadConversation(conversationId)
    }
    loadFromRoute()
    window.addEventListener('hashchange', loadFromRoute)
    return () => window.removeEventListener('hashchange', loadFromRoute)
  }, [getRouteConversationId, reloadConversation])

  const handleSelectConversation = async (conversationId: string) => {
    setLastAssistantStreamStats(null)
    try {
      const conv = await chatApi.getConversation(conversationId)
      setCurrentConversation(conv)
      setStreamingToolCalls([])
      activeRunIdRef.current = null
      syncConversationRoute(conversationId)
      setStreamError('')
    } catch (err) {
      console.error('Failed to load conversation:', err)
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '对话加载失败')
    }
  }

  const handleNewConversation = useCallback(async () => {
    setLastAssistantStreamStats(null)
    try {
      const conv = await chatApi.createConversation(
        activeProviderId || undefined,
        activeModel || undefined
      )
      setCurrentConversation(conv)
      setStreamingToolCalls([])
      activeRunIdRef.current = null
      syncConversationRoute(conv.id)
      refreshSidebar()
      setStreamError('')
    } catch (err) {
      console.error('Failed to create conversation:', err)
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '创建对话失败')
    }
  }, [activeModel, activeProviderId, refreshSidebar, syncConversationRoute])

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

    setPendingUserMessage(optimisticUserMessage)
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
    try {
      let conversation = currentConversation
      if (!conversation) {
        conversation = await chatApi.createConversation(
          activeProviderId || undefined,
          activeModel || undefined
        )
        currentConversationIdRef.current = conversation.id
        setCurrentConversation(conversation)
        syncConversationRoute(conversation.id)
      }

      const selectedSkillId = effectiveSkillId || null
      const updatedConv = await chatApi.sendMessage(
        conversation!.id,
        trimmed,
        attachments,
        selectedSkillId,
      )
      applyAssistantStreamStats(updatedConv)
      setCurrentConversation(updatedConv)
      setPendingUserMessage(null)
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
      refreshSidebar()
    } catch (err) {
      console.error('Failed to send message:', err)
      setPendingUserMessage(null)
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
        setCurrentConversation(updated)
        refreshSidebar()
      } catch (err) {
        console.error('Failed to update message:', err)
        setStreamError(typeof err === 'string' ? err : (err as Error).message || '保存失败')
      }
    },
    [currentConversation, refreshSidebar],
  )

  const handleDeleteMessage = useCallback(
    async (messageId: string) => {
      if (!currentConversation) return
      if (!window.confirm('确定删除这条助手回复吗？')) return
      try {
        const updated = await chatApi.deleteMessage(currentConversation.id, messageId)
        setCurrentConversation(updated)
        setLastAssistantStreamStats((prev) =>
          prev?.messageId === messageId ? null : prev,
        )
        refreshSidebar()
      } catch (err) {
        console.error('Failed to delete message:', err)
        setStreamError(typeof err === 'string' ? err : (err as Error).message || '删除失败')
      }
    },
    [currentConversation, refreshSidebar],
  )

  const handleRegenerateMessage = useCallback(
    async (messageId: string) => {
      if (!currentConversation || streaming || sendInFlightRef.current) return

      const conversationId = currentConversation.id
      const messageIndex = currentConversation.messages.findIndex(
        (message) => message.id === messageId,
      )
      if (messageIndex < 0) return

      setCurrentConversation({
        ...currentConversation,
        messages: currentConversation.messages.slice(0, messageIndex),
      })
      setLastAssistantStreamStats(null)
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

      try {
        const updated = await chatApi.regenerateMessage(conversationId, messageId)
        applyAssistantStreamStats(updated)
        setCurrentConversation(updated)
        refreshSidebar()
      } catch (err) {
        console.error('Failed to regenerate message:', err)
        setStreamError(typeof err === 'string' ? err : (err as Error).message || '重新生成失败')
        void reloadConversation(conversationId)
      } finally {
        flushPendingStreamDone()
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
        sendInFlightRef.current = false
      }
    },
    [applyAssistantStreamStats, currentConversation, flushPendingStreamDone, refreshSidebar, reloadConversation, streaming],
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
      setCurrentConversation(updatedConv)
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
    try {
      await chatApi.cancelStream(conversationId)
    } catch (err) {
      console.error('Failed to cancel chat stream:', err)
      setStreamError(typeof err === 'string' ? err : (err as Error).message || '停止生成失败')
    } finally {
      setCancellingStream(false)
    }
  }, [cancellingStream, streaming])

  const displayMessages = useMemo(() => {
    const stored = currentConversation?.messages ?? []
    if (!pendingUserMessage) return stored
    const alreadyStored = stored.some(
      (message) =>
        message.id === pendingUserMessage.id ||
        (message.role === 'user' &&
          message.content === pendingUserMessage.content &&
          message.timestamp >= pendingUserMessage.timestamp - 2),
    )
    return alreadyStored ? stored : [...stored, pendingUserMessage]
  }, [currentConversation?.messages, pendingUserMessage])

  const hasMessages = displayMessages.length > 0
  const showEmptyHero = chatView === 'conversation' && !hasMessages && !streaming && !streamError

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
          onSelectConversation={(id) => {
            runAfterLeavingSettings(() => void handleSelectConversation(id))
          }}
          onNewConversation={() => {
            runAfterLeavingSettings(() => void handleNewConversation())
          }}
          onConversationDeleted={() => {
            forgetRememberedChatRoute()
            setCurrentConversation(null)
            syncConversationRoute(null)
            refreshSidebar()
          }}
          onOpenSettings={() => openEmbeddedSettings('chat')}
          settingsActive={chatView === 'settings'}
          collapsed={sidebarCollapsed}
          onToggleCollapsed={() => setSidebarCollapsed(true)}
          refreshKey={sidebarRefreshKey}
          searchOpen={searchOpen}
          onSearchOpenChange={(open) => {
            if (open) {
              runAfterLeavingSettings(() => setSearchOpen(true))
              return
            }
            setSearchOpen(false)
          }}
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
        ) : (
          <div className="relative flex min-w-0 flex-1 flex-col bg-white dark:bg-[#212121]">
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
              <div className="min-w-0 flex-1" data-tauri-drag-region />
            </header>

            <div className="flex min-h-0 flex-1 flex-col">
              {showEmptyHero ? (
                <div className="flex flex-1 flex-col items-center justify-center px-6 pb-16">
                  <div className="w-full max-w-3xl space-y-8">
                    <h2 className="text-center text-[1.75rem] font-semibold leading-snug tracking-tight text-neutral-900 dark:text-neutral-50 sm:text-[2rem]">
                      今天我能为您做些什么？
                    </h2>
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
                onClick={() => {
                  void api.chatConfirmToolCall(pendingToolConfirm.toolCallId, false)
                  setPendingToolConfirm(null)
                }}
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
                onClick={() => {
                  void api.chatConfirmToolCall(pendingToolConfirm.toolCallId, false)
                  setPendingToolConfirm(null)
                }}
              >
                拒绝
              </button>
              <button
                type="button"
                className="rounded-md bg-neutral-900 px-3 py-1.5 text-[12px] font-medium text-white hover:bg-neutral-700 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-200"
                onClick={() => {
                  void api.chatConfirmToolCall(pendingToolConfirm.toolCallId, true)
                  setPendingToolConfirm(null)
                }}
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
