import { refreshSubAgents } from './useSubAgents'
import { SubAgentIndicator } from './SubAgentPanel'
import { lazy, memo, Profiler, startTransition, Suspense, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore, type ProfilerOnRenderCallback, type ReactNode, type Ref } from 'react'
import { type ConversationSelectionScope, type ExtensionsNavItem } from './Sidebar'
import { ChatSidebarPane } from './ChatSidebarPane'
import { useChatRouting } from './hooks/useChatRouting'
import { useSettingsExit } from './hooks/useSettingsExit'
import { useSidebarLayout } from './hooks/useSidebarLayout'
import { useAssistantActions } from './hooks/useAssistantActions'
import { createChatNavigationController } from './chatNavigationController'
import { createChatExecutionOwner } from './chatExecutionOwner'
import { createChatStreamLifecycleOwner, type StreamLifecycleResult } from './chatStreamLifecycleOwner'
import { createChatPopoutOwnershipOwner } from './chatPopoutOwnershipOwner'
import { createRunInteractionInbox } from './runInteractionInbox'
import { createChatSendController, type SendPresentationEvent } from './chatSendController'
import { createChatRunCommands, type RunCommandPresentationEvent } from './chatRunCommands'
import { createStreamPreviewOwner } from './streamPreviewOwner'
import { useExternalSendQueue } from './hooks/useExternalSendQueue'
import { useMessageQueue } from './hooks/useMessageQueue'
import type { QueuedMessage } from './hooks/useMessageQueue'
import { useComposerDraft } from './hooks/useComposerDraft'
import { useTauriEvent } from './hooks/useTauriEvent'
import {
  getRouteConversationId,
  hashPath,
  isChatAssistantCenterPath,
  isChatKnowledgeCenterPath,
  isChatMcpCenterPath,
  isChatNotesPath,
  isChatArtifactsPath,
  isChatOnboardingRoute,
  isChatPluginCenterPath,
  isChatSessionCenterPath,
  isChatAutomationsPath,
  isChatSettingsPath,
  isChatSkillCenterPath,
  conversationHash,
  extensionsNavItemForView,
  setHash,
} from './chatRoutes'
import { AsyncQuestionsContext } from './asyncQuestionsContext'
import { ChatTitlebar } from './ChatTitlebar'
import { deriveToolStatusHint, findUnavailableRecommendedTools } from './toolAvailability'
import { useChatToolIndicator } from './hooks/useChatToolIndicator'
import { useConversationContext } from './hooks/useConversationContext'
import { useConversationMetaMutations } from './hooks/useConversationMetaMutations'
import { useMessageActions } from './hooks/useMessageActions'
import { ChatTitlebarActions } from './ChatTitlebarActions'
import { ConversationTitlebarControls } from './ConversationTitlebarControls'
import {
  captureConversationNavigation,
  completeConversationTransition,
  invalidateConversationTransition,
  isCurrentConversationNavigation,
} from './conversationTransitionStore'
import type { AssistantStreamStats, MessageListProps } from './MessageList'
import type { InputBarProps } from './InputBar'
import { SessionUsageStrip } from './SessionUsageStrip'
import { deriveDshPresetModes, derivePermissionModes, useDetectedExternalAgents, useDshCustomPresets } from './permissionModes'
import { ContextIndicator } from './ContextIndicator'
import {
  agentRuntimesEqual,
  BUILTIN_AGENT_RUNTIME,
  chatApi,
  normalizeAgentRuntime,
} from './api'
import { loadLastAgentRuntime, saveLastAgentRuntime } from './lastAgentRuntime'
import { loadLastModel, resolvePreferredChatModel } from '../data/chatModelPreference'
import {
  loadLastThinkingLevel,
  loadLastWebSearchMode,
  persistLastChatModelToSettings,
} from './composerPreferences'
import {
  resolveSendSkillId,
  normalizeSkill,
  skillRecommendedTools,
} from './skillSelection'
import {
  conversationLastMessageContent,
  optimisticConversationListItem,
  pruneSettledOptimisticItems,
  settleOptimisticConversationListItems,
} from './optimisticSidebar'
import { additionalDirectoriesOf, isPlainBlankConversation } from './conversationFields'
import { scheduleIdleTask } from './idleTask'
import {
  chatTitlebarMacInsetClass,
  chatTitlebarRowClass,
  usesNativeTitlebar,
} from './platform'
import type {
  ChatProject,
  ChatSet,
  Conversation,
  ConversationListItem,
  ConversationSearchHit,
  AgentPlanMode,
  AgentPlanState,
  AgentTodoState,
  GoalState,
  PendingAttachment,
  SkillMeta,
  ModelRef,
  WebSearchMode,
} from './types'
import {
  api,
  builtinWebSearchSupported,
  resolveProviderWebSearchMode,
  type ChatHookPayload,
} from '../api/tauri'
import { getSettingsCached, subscribeSettings, updateSettingsCached } from '../api/settingsCache'
import { setExclusiveConversationIds } from '../api/chatProtocol'
import { OnboardingShell } from '../onboarding/public/shell'
import type { SettingsShellHandle, SettingsTab } from '../settings/public/shell'
import { i18n, LangContext, type Lang } from '../components/i18n'
import { estimateTokens } from '../utils/tokens'
import {
  forgetRememberedChatRoute,
} from './persistence'
import { RightDock } from './dock/RightDock'
import { useRightDock } from './hooks/useRightDock'
import { insertTextIntoComposer } from './composerInsert'
import { requestDockMarkdownPreview } from './dock/dockPreview'
import { isTauriRuntime } from './utils'
import { onChatImageViewerOpen, type ChatImageViewerItem } from './imageViewer'
import {
  getCoarse as getStreamCoarse,
  setCoarse as setStreamCoarse,
  useStreamCoarse,
} from './streamingStore'
import {
  resetGroups,
} from './groupStreamingStore'
import { onChatPerfProfiler, useChatPerfLongTaskProbe, useChatPerfRenderProbe } from './chatPerformanceProbe'
import { ChatRouteKeepAlive } from './ChatRouteKeepAlive'
import { ChatConversationPane } from './ChatConversationPane'
import { GoalCard } from './GoalCard'
import { composerGoal } from './goalPresentation'
import { PopoutOccupiedPlaceholder } from './popout/PopoutOccupiedPlaceholder'
import { emptyPopoutConversation, stripConversationMessages } from './popout/conversationStub'
import {
  findSubagentToolIndex,
  isStreamTerminal,
  mergeSubagentProgress,
  messageToolCalls,
} from './streamApply'
import { isPlanApproval } from './toolApproval'
import { PendingInteractionSlot } from './PendingInteractionSlot'

const AssistantCenter = lazy(() => import('./AssistantCenter').then((module) => ({
  default: module.AssistantCenter,
})))

// 共享 import thunk：lazy 与空闲预取复用同一次动态 import（模块缓存保证只加载一次）。
// SettingsShell 依赖图很大（Markdown/KaTeX、各设置面板），dev 下首次点开设置要现场编译
// 数百个模块而转圈数秒；挂载后空闲预取把这段成本移到用户点击之前。
const importSettingsShell = () => import('../settings/public/shell')

const SettingsShell = lazy(() => importSettingsShell().then((module) => ({
  default: module.SettingsShell,
})))
const SessionCenter = lazy(() => import('./public/sessionCenter').then((module) => ({
  default: module.SessionCenter,
})))
const PluginCenter = lazy(() => import('./public/pluginCenter').then((module) => ({
  default: module.PluginCenter,
})))
const ChatMarkdown = lazy(() => import('./public/markdown').then((module) => ({
  default: module.ChatMarkdown,
})))

const SkillCenter = lazy(() => import('./SkillCenter').then((module) => ({
  default: module.SkillCenter,
})))

const McpCenter = lazy(() => import('./McpCenter').then((module) => ({
  default: module.McpCenter,
})))

const KnowledgeCenter = lazy(() => import('./KnowledgeCenter').then((module) => ({
  default: module.KnowledgeCenter,
})))

const NotesCenter = lazy(() => import('./NotesCenter').then((module) => ({
  default: module.NotesCenter,
})))
const ArtifactsCenter = lazy(() => import('./ArtifactsCenter').then((module) => ({ default: module.ArtifactsCenter })))

const AutomationCenter = lazy(() => import('./automation/AutomationCenter').then((module) => ({
  default: module.AutomationCenter,
})))

type ChatView = import('./routeCodec').ChatView

interface ChatProps {
  onSettingsChange: () => void
  /**
   * 首屏内容就绪回调（一次性）。宿主（App）据此把窗口 show 从“App 挂载即弹出”推迟到
   * “Chat 首屏可渲染”，避免窗口弹出后仍在转圈。初始视图为设置页时，就绪信号来自
   * SettingsShell 的 onReady；其余视图挂载后即视为骨架就绪。
   */
  onContentReady?: () => void
}

/**
 * 设置页入场容器：先静态铺好起始态（下移 + 半透明），首帧绘制之后再加 --entered 触发过渡。
 *
 * 为什么不能直接用 CSS animation：animation 走墙钟时间，而 SettingsShell 那棵树很大，
 * 挂载帧的布局/绘制常吃掉上百毫秒 —— 等首帧真正画出来，动画已经跑完大半，体感就是"没有动画"
 * （退场没这问题，它作用于已绘制的元素）。双 rAF 把起点钉在首帧之后，整段位移必定可见。
 * 同款模式见 CompactionDivider。
 */
function SettingsEnterPane({ exiting, className, children }: {
  exiting: boolean
  className: string
  children: ReactNode
}) {
  const [entered, setEntered] = useState(false)

  useLayoutEffect(() => {
    let cancelled = false
    const frame = requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        if (!cancelled) setEntered(true)
      })
    })
    return () => {
      cancelled = true
      cancelAnimationFrame(frame)
    }
  }, [])

  const motion = exiting
    ? 'chat-motion-settings-out'
    : `chat-motion-settings-in${entered ? ' chat-motion-settings-in--entered' : ''}`

  return <div className={`${motion} ${className}`}>{children}</div>
}

/** 设置区的独立渲染边界。侧栏折叠、聊天流式状态变化不应重新执行设置页大树。 */
const ChatSettingsPane = memo(function ChatSettingsPane({
  settingsRef,
  exiting,
  className,
  initialTab,
  reserveTrafficLightSpace,
  onClose,
  onSettingsChange,
  onReady,
  renderSessionCenter,
  onRender,
}: {
  settingsRef: Ref<SettingsShellHandle>
  exiting: boolean
  className: string
  initialTab: SettingsTab
  reserveTrafficLightSpace: boolean
  onClose: () => void
  onSettingsChange: () => void
  onReady: () => void
  renderSessionCenter: (lang: Lang) => ReactNode
  onRender: ProfilerOnRenderCallback
}) {
  return (
    <Suspense fallback={null}>
      <SettingsEnterPane
        key="settings"
        exiting={exiting}
        className={className}
      >
        <Profiler id="SettingsShell" onRender={onRender}>
          <SettingsShell
            ref={settingsRef}
            variant="embedded"
            initialTab={initialTab}
            reserveTrafficLightSpace={reserveTrafficLightSpace}
            onClose={onClose}
            onSettingsChange={onSettingsChange}
            onReady={onReady}
            renderSessionCenter={renderSessionCenter}
            renderPluginCenter={({ section, onSectionChange, lang, connectors }) => (
              <Suspense fallback={null}>
                <PluginCenter
                  section={section}
                  onSectionChange={onSectionChange}
                  lang={lang}
                  connectors={connectors}
                />
              </Suspense>
            )}
            renderReleaseNotes={(markdown) => (
              <Suspense fallback={<p>{markdown}</p>}>
                <ChatMarkdown content={markdown} />
              </Suspense>
            )}
          />
        </Profiler>
      </SettingsEnterPane>
    </Suspense>
  )
})

// 设置当前视图的流式错误（写 streamingStore 的 coarse 片）。模块级函数，调用点无需进
// useCallback 依赖。注意：与 setStreamErrorForConversation 不同，这里只改当前视图、不写
// streamErrorsRef（保持原 setStreamError(useState) 的语义）。
function setStreamError(error: string): void {
  setStreamCoarse({ streamError: error })
}

type SendMessageOptions = {
  planMessageId?: string
  forceNewConversation?: boolean
  conversationOverride?: Conversation | null
  /** 外部队列可记录已创建/已 patch 的会话，失败重试继续该会话。 */
  onPartialConversation?: (conversation: Conversation) => void
  /** 前置校验完成、消息正式进入本地发送流程；输入框可立即清空。 */
  onAccepted?: () => void
}

/** 稳定空数组：没有排队消息时不要每次渲染都造一个新引用。 */
const NO_QUEUED_MESSAGES: QueuedMessage[] = []

export default function Chat({ onSettingsChange, onContentReady }: ChatProps) {
  useChatPerfRenderProbe('Chat', { view: hashPath() })
  useChatPerfLongTaskProbe()
  const [chatView, setChatView] = useState<ChatView>(() => {
    const path = hashPath()
    if (isChatOnboardingRoute(path)) return 'onboarding'
    if (isChatSettingsPath(path)) return 'settings'
    if (isChatAssistantCenterPath(path)) return 'assistants'
    if (isChatSkillCenterPath(path)) return 'skill'
    if (isChatMcpCenterPath(path)) return 'mcp'
    if (isChatKnowledgeCenterPath(path)) return 'knowledge'
    if (isChatNotesPath(path)) return 'notes'
    if (isChatArtifactsPath(path)) return 'artifacts'
    if (isChatAutomationsPath(path)) return 'automations'
    // 旧 `#chat/sessions`：对话库已迁设置
    if (isChatSessionCenterPath(path)) return 'settings'
    // 旧 `#chat/plugins`：插件已迁设置，首屏落到设置页
    if (isChatPluginCenterPath(path)) return 'settings'
    return 'conversation'
  })
  // 首屏就绪只发一次。初始视图是设置页则等 SettingsShell.onReady；否则挂载后即发。
  const contentReadyEmittedRef = useRef(false)
  const emitContentReady = useCallback(() => {
    if (contentReadyEmittedRef.current) return
    contentReadyEmittedRef.current = true
    onContentReady?.()
  }, [onContentReady])
  const initialViewIsSettingsRef = useRef(chatView === 'settings')
  useLayoutEffect(() => {
    // 初始设置页把就绪信号委托给 SettingsShell.onReady（数据就绪才可渲染）；
    // 其余初始视图（会话/助手/技能/引导）挂载即有骨架，直接发信号。
    if (!initialViewIsSettingsRef.current) emitContentReady()
  }, [emitContentReady])
  const [currentConversation, setCurrentConversation] = useState<Conversation | null>(null)
  const [conversationRenderRequestId, setConversationRenderRequestId] = useState(0)
  /** 全局搜索跳转目标；MessageList 完成滚动后清空。 */
  const [focusMessageId, setFocusMessageId] = useState<string | null>(null)
  const {
    collapsed: sidebarCollapsed,
    width: sidebarWidth,
    setCollapsed: setSidebarCollapsedPersisted,
    collapse: handleCollapseSidebar,
    setWidth: handleSidebarWidthChange,
  } = useSidebarLayout()
  const [searchOpen, setSearchOpen] = useState(false)
  const [selectedProject, setSelectedProject] = useState<ChatProject | null>(null)
  const [selectedSet, setSelectedSet] = useState<ChatSet | null>(null)
  // 流式高频状态已移到 streamingStore（useSyncExternalStore）。Chat 只订阅 coarse 这一片
  // （streaming/streamFrozen/cancelling/streamError，边沿才变），用于 showEmptyHero / drain 判定；
  // 内容快照由 MessageList 直接订阅，避免每帧 token 拖着整个 Chat 重渲。
  const streamCoarse = useStreamCoarse()
  /** 会话执行身份与乐观用户消息跨导航存活；高频正文仍在专用展示 store。 */
  const executionOwner = useRef(createChatExecutionOwner()).current
  const [previewOwner] = useState(createStreamPreviewOwner)
  const [streamLifecycleOwner] = useState(() => createChatStreamLifecycleOwner(executionOwner, previewOwner))
  const [interactionInbox] = useState(() => createRunInteractionInbox({
    confirmTool: api.chatConfirmToolCall,
    respondConsent: api.chatRespondSessionConsent,
  }))
  const interactionSnapshot = useSyncExternalStore(
    interactionInbox.subscribe,
    interactionInbox.getSnapshot,
    interactionInbox.getSnapshot,
  )
  useSyncExternalStore(
    executionOwner.subscribe,
    executionOwner.getRevision,
  )
  const [assistantStreamStatsByMessageId, setAssistantStreamStatsByMessageId] =
    useState<Record<string, AssistantStreamStats>>({})
  const [sidebarRefreshKey, setSidebarRefreshKey] = useState(0)
  const [optimisticSidebarConversations, setOptimisticSidebarConversations] =
    useState<ConversationListItem[]>([])
  const [generatingConversationIds, setGeneratingConversationIds] = useState<ReadonlySet<string>>(
    () => new Set(),
  )
  const [popoutOwner] = useState(createChatPopoutOwnershipOwner)
  const popoutConversationIds = useSyncExternalStore(popoutOwner.subscribe, popoutOwner.membership)
  const [popoutNotice, setPopoutNotice] = useState<string | null>(null)
  useEffect(() => { setExclusiveConversationIds(popoutConversationIds) }, [popoutConversationIds])
  const [sidebarProfileRefreshKey, setSidebarProfileRefreshKey] = useState(0)
  // 欢迎页的输入上下文由一个 owner 管理；首次发送时统一落到新会话。
  const {
    value: {
      providerId: draftProviderId,
      model: draftModel,
      knowledgeBaseIds: draftKnowledgeBaseIds,
      forceKnowledgeSearch: draftForceKnowledgeSearch,
      additionalDirectories: draftAdditionalDirectories,
      thinkingLevel: draftThinkingLevel,
      webSearchMode: draftWebSearchMode,
      replyModels: draftReplyModels,
      agentRuntime: draftAgentRuntime,
    },
    setProviderId: setDraftProviderId,
    setModel: setDraftModel,
    resetConversationContext: resetComposerDraftContext,
    setters: composerDraftSetters,
  } = useComposerDraft({
    providerId: '',
    model: '',
    thinkingLevel: loadLastThinkingLevel(),
    agentRuntime: loadLastAgentRuntime() ?? BUILTIN_AGENT_RUNTIME,
  })
  const [skills, setSkills] = useState<SkillMeta[]>([])
  const [settingsInitialTab, setSettingsInitialTab] = useState<SettingsTab>(() => {
    const path = hashPath()
    if (isChatPluginCenterPath(path)) return 'plugins'
    if (isChatSessionCenterPath(path)) return 'sessions'
    return 'chat'
  })
  const [uiLang, setUiLang] = useState<Lang>('zh')
  const [extensionsNavItem, setExtensionsNavItem] = useState<ExtensionsNavItem | null>(null)
  // 工具目录 / MCP 开关 / 审批策略 / 供应商能力表（apiFormat 用于判断内置搜索能不能选）。
  const {
    enabledTools,
    mcpServers,
    webSearchEnabled,
    providerApiFormats,
    providerOAuthTypes,
    providerBaseUrls,
    enabledToolCount,
    toolDiscoveryPending,
    toolsDisabledReason,
    toolsRequested,
    approvalPolicy,
    disabledSkillIds,
    refresh: refreshToolIndicator,
    setApprovalPolicy: handleApprovalPolicyChange,
    toggleMcpServer: handleToggleMcpServer,
  } = useChatToolIndicator({ onSettingsChange })
  // Hook 执行失败：非阻断警告条。ponytail: 只留最新一条 —— Hook 是旁路观测，
  // 堆一个可滚动的失败列表没有对应的用户动作。
  const [hookWarning, setHookWarning] = useState<ChatHookPayload | null>(null)
  const [protocolVersionMismatch, setProtocolVersionMismatch] = useState(false)
  const [imageViewerItem, setImageViewerItem] = useState<ChatImageViewerItem | null>(null)
  // 导入的对话：CLI 那边是否已经有新内容（ADR-0002）。只提示，不同步。
  const [importedHistoryStale, setImportedHistoryStale] = useState(false)
  const currentConversationIdRef = useRef<string | null>(null)
  // 始终指向最新 currentConversation。消息操作 handler（编辑/删除/重发）借此读取最新会话，
  // 而无需把 currentConversation 列进 useCallback 依赖——否则每次切模型/思考等级（currentConversation
  // 换引用）这些 handler 都换身份，打穿 MessageBubble 的 memo 导致全列表重渲（公式 remount 闪烁）。
  const currentConversationRef = useRef(currentConversation)
  currentConversationRef.current = currentConversation

  const refreshSidebar = useCallback(() => {
    setSidebarRefreshKey((key) => key + 1)
  }, [])

  const {
    contextState,
    setContextLoading,
    contextLoading,
    contextError,
    resetContext,
    compactingConversationIds,
    markConversationCompacting,
    animateCompactionBoundaryId,
    animateClearBoundaryId,
    refreshContextStats,
    refreshCurrent: handleRefreshContext,
    compressCurrent: handleCompressContext,
    clearCurrent: handleClearContext,
  } = useConversationContext({ currentConversation, currentConversationIdRef, setCurrentConversation, refreshSidebar })
  const contextCompressing = currentConversation
    ? compactingConversationIds.has(currentConversation.id)
    : false

  useEffect(() => {
    const id = currentConversation?.id
    if (!id) {
      setImportedHistoryStale(false)
      return
    }
    let cancelled = false
    void chatApi
      .importedHistoryStale(id)
      .then((stale) => {
        if (!cancelled) setImportedHistoryStale(stale)
      })
      // 检查不了就当没过期——这只是个提示，不该因为它报错打断打开对话。
      .catch(() => {
        if (!cancelled) setImportedHistoryStale(false)
      })
    return () => {
      cancelled = true
    }
  }, [currentConversation?.id])
  const streamErrorsRef = useRef<Record<string, string>>({})
  const settingsRef = useRef<SettingsShellHandle>(null)
  // A 合帧（render coalescing）：高频 stream/tool/subagent/userprompt 事件不再每条都同步
  // setState 重渲，而是把"待显示的快照"记到 ref，用 requestAnimationFrame 每帧最多 flush 一次。

  useEffect(() => onChatImageViewerOpen(setImageViewerItem), [])

  const generatingConversationIdsRef = useRef<Set<string>>(new Set())
  const syncGeneratingConversationIds = useCallback(() => {
    const next = new Set([
      ...executionOwner.activeConversationIds(),
      ...previewOwner.streamingConversationIds(),
      ...interactionInbox.getSnapshot().pendingToolConversationIds,
    ])
    const previous = generatingConversationIdsRef.current
    if (previous.size === next.size && [...previous].every((id) => next.has(id))) return
    generatingConversationIdsRef.current = next
    setGeneratingConversationIds(next)
  }, [executionOwner, interactionInbox, previewOwner])

  useEffect(() => interactionInbox.subscribe(syncGeneratingConversationIds), [interactionInbox, syncGeneratingConversationIds])

  // These hooks retain the latest callbacks internally, so their commands can be
  // used by earlier lifecycle handlers without a second Chat-level ref bridge.
  const messageQueue = useMessageQueue({
    onSendMessage: (content, attachments, options) =>
      handleSendMessage(content, attachments, options),
    onRestoreToComposer: (message) => insertTextIntoComposer(message.content),
    onPendingChange: (conversationId, pending) => {
      void chatApi.setGoalUserQueuePending(conversationId, pending)
    },
  })
  const queueCommands = messageQueue.commands
  const { drainExternalSends, wakeAfterRun } = useExternalSendQueue({
    onEnterConversationView: () => setChatView('conversation'),
    onImportConversation: (messages, attachmentPaths) =>
      importExternalConversation(messages, attachmentPaths),
    onSendMessage: (content, attachments, options) =>
      handleSendMessage(content, attachments, options),
    onError: setStreamError,
  })

  // B：彻底把一个会话从所有本地乐观/in-flight/快照状态中剔除（ghost 清理）。
  // 不触碰 currentConversation/route，由调用方按场景决定。
  const dropConversationLocally = useCallback((conversationId: string) => {
    delete streamErrorsRef.current[conversationId]
    interactionInbox.observe({ kind: 'drop', conversationId })
    previewOwner.drop(conversationId)
    executionOwner.observe({ kind: 'drop', conversationId })
    // 排队消息也一起剔除：会话没了，队列里那几条再没有能落到的地方（`drain` 也拿不到会话对象）。
    queueCommands.clearConversation(conversationId)
    setOptimisticSidebarConversations((items) => items.filter((item) => item.id !== conversationId))
    syncGeneratingConversationIds()
  }, [executionOwner, interactionInbox, previewOwner, queueCommands, syncGeneratingConversationIds])

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
    Boolean(currentConversationIdRef.current && (
      executionOwner.snapshot(currentConversationIdRef.current).inFlight
      || previewOwner.isStreaming(currentConversationIdRef.current)
    ))
  ), [executionOwner, previewOwner])

  const applyConversation = useCallback((conversation: Conversation | null) => {
    const current = currentConversationRef.current
    if (
      conversation
      && current
      && conversation.id === current.id
      && conversation.revision < current.revision
    ) {
      return
    }
    // 兜底网：后端已在所有返回 Conversation 的命令出口剥离 model_messages/api_messages
    // （strip_transcripts_for_frontend），所以正常路径到这里已是轻量副本。这里再剥一次，确保
    // 任何遗漏/未来新增的后端出口都不会把这两份前端永不读的转录留进 React state。后端回放读盘
    // 上完整副本，不受影响。
    if (conversation?.messages) {
      for (const m of conversation.messages) {
        if (m.role !== 'assistant') continue
        m.model_messages = undefined
        m.modelMessages = undefined
        m.api_messages = undefined
        m.apiMessages = undefined
      }
    }
    setCurrentConversation((previous) => conversation && previous?.id === conversation.id
      && conversation.revision < previous.revision ? previous : conversation)
  }, [])

  const occupyConversationInMain = useCallback((
    conversationId: string,
    source?: Conversation | ConversationListItem | null,
  ) => {
    invalidateConversationTransition()
    currentConversationIdRef.current = conversationId
    const current = currentConversationRef.current
    const next = current?.id === conversationId
      ? stripConversationMessages(current)
      : emptyPopoutConversation(conversationId, source ?? (current?.id === conversationId ? current : null))
    applyConversation(next)
    previewOwner.drop(conversationId)
    if (getStreamCoarse().streaming) {
      setStreamCoarse({ streaming: false, streamFrozen: false, cancelling: false })
    }
    setHash(conversationHash(conversationId))
  }, [applyConversation, previewOwner])

  /** 后台异步结果只能更新它发起时所属的会话，不能把用户后来打开的会话顶掉。 */
  const applyConversationIfCurrent = useCallback((expectedId: string, conversation: Conversation) => {
    if (currentConversationIdRef.current !== expectedId) return false
    applyConversation(conversation)
    return true
  }, [applyConversation])

  // 纯元数据更新（模型 / 思考等级 / 知识库挂载等）：合并后端返回的新元数据，但**保留现有
  // messages 数组引用**。否则每条消息都变成新对象，击穿 MessageBubble/ChatMarkdown 的 memo，
  // 历史消息里的 LaTeX 会整屏重渲闪一下。这类更新后端不会改 messages，沿用旧引用安全。
  const applyConversationMeta = useCallback((updated: Conversation) => {
    setCurrentConversation((prev) => {
      if (!prev || prev.id !== updated.id || updated.revision < prev.revision) return prev
      return { ...updated, messages: prev.messages }
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

  const patchGoalState = useCallback((nextState: GoalState | null) => {
    setCurrentConversation((prev) => prev
      ? { ...prev, goal_state: nextState ?? undefined, goalState: nextState ?? undefined }
      : prev)
  }, [])

  const freezeStreamSnapshot = useCallback((conversationId: string): boolean => {
    const frozen = previewOwner.freeze(conversationId)
    syncGeneratingConversationIds()
    return frozen
  }, [previewOwner, syncGeneratingConversationIds])

  /** Freeze the visible preview until the committed conversation contains its answer.
   * The external store can flush before the conversation's React state update.
   */
  const settleStreamingPreview = useCallback((conversationId: string) => {
    previewOwner.complete(conversationId, {
      kind: 'persisted',
      committedMessages: currentConversationRef.current?.messages ?? [],
    })
  }, [previewOwner])

  const restoreStreamingPreview = useCallback((conversationId: string | null) => {
    previewOwner.activate(conversationId)
    interactionInbox.activate(conversationId)
    if (!conversationId) {
      setStreamCoarse({ streamError: '' })
      return
    }
    setStreamCoarse({ streamError: streamErrorsRef.current[conversationId] ?? '' })
  }, [interactionInbox, previewOwner])

  // One view reset for every navigation path that actually leaves a conversation.
  // Background execution remains owned by executionOwner and is not cancelled here.
  const clearDisplayedConversation = useCallback(() => {
    currentConversationIdRef.current = null
    applyConversation(null)
    restoreStreamingPreview(null)
  }, [applyConversation, restoreStreamingPreview])

  useEffect(() => {
    previewOwner.attach()
    return () => {
      previewOwner.dispose()
      resetGroups()
    }
  }, [previewOwner])

  const clearStreamSnapshot = useCallback((conversationId: string | null) => {
    if (!conversationId) return
    interactionInbox.observe({ kind: 'drop', conversationId })
    previewOwner.drop(conversationId)
    syncGeneratingConversationIds()
  }, [interactionInbox, previewOwner, syncGeneratingConversationIds])

  const activeAgentRuntime = useMemo(
    () => (currentConversation ? normalizeAgentRuntime(currentConversation) : draftAgentRuntime),
    [currentConversation, draftAgentRuntime],
  )
  const usesExternalRuntime = activeAgentRuntime.kind === 'external' && !!activeAgentRuntime.externalAgentId
  const usesChatRuntime = activeAgentRuntime.kind === 'chat'
  const {
    changeModel: handleModelChange,
    changeThinkingLevel: handleThinkingLevelChange,
    setWebSearchMode: handleSetWebSearchMode,
    changeReplyModels: handleChangeReplyModels,
    changeKnowledgeBaseIds: handleChangeKnowledgeBaseIds,
    changeAdditionalDirectories: handleChangeAdditionalDirectories,
    toggleForceKnowledgeSearch: handleToggleForceKnowledgeSearch,
    changeRuntime: handleRuntimeChange,
    changeExternalModel: handleExternalModelChange,
    changeExternalSandbox: handleExternalSandboxChange,
    changeExternalPreset: handleExternalPresetChange,
    persistApprovedExternalSandbox,
    editGoal: handleEditGoal,
    pauseGoal: handlePauseGoal,
    resumeGoal: handleResumeGoal,
    cancelGoal: handleCancelGoal,
  } = useConversationMetaMutations({
    currentConversationRef,
    activeAgentRuntime,
    draft: composerDraftSetters,
    draftForceKnowledgeSearch,
    applyConversationMeta,
    applyConversationIfCurrent,
    setStreamErrorForConversation,
  })
  // 底栏模式胶囊：内置 Agent = Act/Plan/Orchestrate；Kivio Chat 无此胶囊；本地 CLI = 沙盒档位。
  // CLI 没有档位时返回空表 → 胶囊隐藏。
  const detectedExternalAgents = useDetectedExternalAgents(currentConversation?.id ?? null)
  const activeAgentPlanMode = currentConversation?.agent_plan_state?.mode
    ?? currentConversation?.agentPlanState?.mode
    ?? 'act'
  const currentGoal = currentConversation?.goal_state ?? currentConversation?.goalState
  const visibleGoal = composerGoal(currentGoal, currentConversation?.messages ?? [])
  const goalActive = !!currentGoal && !['completed', 'cancelled'].includes(currentGoal.status)
  const composerModes = useMemo(
    () => derivePermissionModes({
      target: 'composer',
      agentRuntime: activeAgentRuntime,
      agents: detectedExternalAgents,
      agentPlanMode: activeAgentPlanMode,
      goalActive,
    }),
    [activeAgentRuntime, detectedExternalAgents, activeAgentPlanMode, goalActive],
  )
  const dshCustomPresets = useDshCustomPresets(activeAgentRuntime)
  const composerPresets = useMemo(
    () => deriveDshPresetModes(activeAgentRuntime, dshCustomPresets),
    [activeAgentRuntime, dshCustomPresets],
  )
  const currentConversationIsBlank = isPlainBlankConversation(currentConversation)
  const activeProviderId = currentConversation && !currentConversationIsBlank
    ? currentConversation.provider_id
    : draftProviderId
  const activeModel = currentConversation && !currentConversationIsBlank
    ? currentConversation.model
    : draftModel
  // 会话级三态联网搜索（任务 07-23）：会话显式模式优先 → 记住的全局默认（上次选择）
  // → 全局 nativeTools.webSearch 开关。这样选一次内置即成为所有新对话的默认。
  const requestedWebSearchMode = useMemo<WebSearchMode>(() => {
    if (currentConversation && !currentConversationIsBlank) {
      const explicit = currentConversation.webSearchMode ?? currentConversation.web_search_mode
      if (explicit) return explicit
    } else if (draftWebSearchMode) {
      return draftWebSearchMode
    }
    const remembered = loadLastWebSearchMode()
    if (remembered) return remembered
    return webSearchEnabled ? 'third_party' : 'off'
  }, [currentConversation, currentConversationIsBlank, draftWebSearchMode, webSearchEnabled])
  const activeWebSearchMode = resolveProviderWebSearchMode(requestedWebSearchMode, providerOAuthTypes[activeProviderId ?? ''])
  const activeBuiltinWebSearchSupported = useMemo(
    () => builtinWebSearchSupported(
      providerApiFormats[activeProviderId ?? ''],
      providerBaseUrls[activeProviderId ?? ''],
      providerOAuthTypes[activeProviderId ?? ''],
    ),
    [providerApiFormats, providerBaseUrls, providerOAuthTypes, activeProviderId],
  )
  // 多模型一问多答（任务 06-30）：当前生效的多答模型集（会话级持久 reply_models，欢迎页用草稿）。
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

  const toolStatusHint = useMemo(() => deriveToolStatusHint({
    toolsDisabledReason,
    enabledToolCount,
    toolsRequested,
    effectiveSkillId,
    recommendedTools: effectiveSkillRecommendedTools,
    unavailableRecommendedTools: findUnavailableRecommendedTools(
      effectiveSkillRecommendedTools, enabledTools, toolDiscoveryPending,
    ),
  }), [
    effectiveSkillId, effectiveSkillRecommendedTools, enabledToolCount, enabledTools,
    toolDiscoveryPending, toolsDisabledReason, toolsRequested,
  ])

  const sendDisabledReason = effectiveSkillRecommendedTools.length > 0 ? toolStatusHint : ''

  // Navigation owns the route/load/popout commit lease. The page supplies only
  // its display effects and the Tauri read adapter; no hook is wired back via ref.
  const navigation = useMemo(() => createChatNavigationController({
    currentConversation: () => currentConversationRef.current,
    currentConversationId: () => currentConversationIdRef.current,
    listPopouts: popoutOwner.list,
    readConversation: chatApi.getConversation,
    isConversationInFlight: (conversationId) => executionOwner.snapshot(conversationId).inFlight,
    prepareNewConversation: () => {
      setSelectedProject(null)
      setSelectedSet(null)
      setAssistantStreamStatsByMessageId({})
      resetComposerDraftContext({
        providerId: activeProviderId,
        model: activeModel,
        agentRuntime: activeAgentRuntime,
      })
      saveLastAgentRuntime(activeAgentRuntime)
      clearDisplayedConversation()
      resetContext()
    },
    clearEmptyChat: () => {
      setAssistantStreamStatsByMessageId({})
      setStreamError('')
    },
    requestClearChat: (conversationId) => {
      if (executionOwner.snapshot(conversationId).inFlight || previewOwner.isStreaming(conversationId)) return 'busy'
      return window.confirm('Clear this chat? This will delete the current conversation history.')
        ? 'confirmed' : 'cancelled'
    },
    deleteConversation: async (conversationId) => { await chatApi.deleteConversation(conversationId) },
    cancelDeletedRun: async (conversationId) => { await chatApi.cancelStream(conversationId) },
    finalizeDeletedChat: (conversationId, clearCurrentView) => {
      dropConversationLocally(conversationId)
      if (clearCurrentView) {
        setAssistantStreamStatsByMessageId({})
        resetContext()
        clearDisplayedConversation()
      }
      refreshSidebar()
    },
    reportClearError: setStreamErrorForConversation,
    focusPopout: (conversationId) => { void chatApi.focusConversationPopout(conversationId) },
    occupyPopout: (conversationId) => occupyConversationInMain(conversationId, currentConversationRef.current),
    prepareSelection: (focusMessageId, fresh) => {
      if (fresh) {
        setAssistantStreamStatsByMessageId({})
        setHookWarning(null)
      }
      setFocusMessageId(focusMessageId)
    },
    showConversation: (conversation, { renderRequestId, selection }) => {
      currentConversationIdRef.current = conversation.id
      startTransition(() => {
        applyConversation(conversation)
        if (renderRequestId > 0) setConversationRenderRequestId(renderRequestId)
      })
      restoreStreamingPreview(conversation.id)
      if (selection) setStreamError('')
      else setStreamCoarse({ cancelling: false })
    },
    resetConversation: () => {
      clearDisplayedConversation()
    },
    discardConversation: (conversationId, error, selection) => {
      console.error('Failed to load conversation:', error)
      dropConversationLocally(conversationId)
      if (
        currentConversationIdRef.current === conversationId
        || (!selection && currentConversationIdRef.current === null)
      ) {
        clearDisplayedConversation()
      }
      refreshSidebar()
      setStreamError(error.message)
    },
  }), [
    activeAgentRuntime, activeModel, activeProviderId, applyConversation, clearDisplayedConversation,
    dropConversationLocally, executionOwner, occupyConversationInMain, popoutOwner,
    previewOwner, refreshSidebar, resetComposerDraftContext, resetContext, restoreStreamingPreview,
    setStreamErrorForConversation,
  ])

  const openEmbeddedSettingsForPlugins = useCallback(() => {
    setSettingsInitialTab('plugins')
    setChatView('settings')
    setHash('#chat/settings')
  }, [])

  const openEmbeddedSettingsForSessions = useCallback(() => {
    setSettingsInitialTab('sessions')
    setChatView('settings')
    setHash('#chat/settings')
  }, [])

  const {
    syncConversationRoute,
    syncOnboardingRoute,
    openEmbeddedSettings,
    openChatSettings: handleOpenChatSettings,
    openAssistantCenter,
    openSkillCenter,
    openExtensionsItem,
  } = useChatRouting({
    onViewChange: setChatView,
    onLoadConversation: navigation.loadRouteConversation,
    onResetConversation: navigation.resetRouteConversation,
    onLeaveConversation: navigation.leaveConversation,
    currentConversationIdRef,
    onOpenPluginsSettings: openEmbeddedSettingsForPlugins,
    onOpenSessionsSettings: openEmbeddedSettingsForSessions,
    setSettingsInitialTab,
    setExtensionsNavItem,
  })
  const extensionsActive = extensionsNavItemForView(chatView)

  const handleOnboardingExit = useCallback(() => {
    setChatView('conversation')
    syncConversationRoute(null)
  }, [syncConversationRoute])

  const reloadConversation = navigation.reloadConversation
  const handleSelectConversation = navigation.selectConversation

  const loadDefaultModel = useCallback(async () => {
    try {
      const settings = await getSettingsCached()
      setUiLang((settings.settingsLanguage as Lang) || 'zh')
      const last = loadLastModel()
      const preferred = resolvePreferredChatModel({
        providers: settings.providers || [],
        last,
        storedChat: settings.defaultModels?.chat ?? { providerId: '', model: '' },
        legacyChat: {
          providerId: settings.chatProviderId || '',
          model: settings.chatModel || '',
        },
        lens: {
          providerId: settings.lens?.providerId || '',
          model: settings.lens?.model || '',
        },
        translator: {
          providerId: settings.translatorProviderId || '',
          model: settings.translatorModel || '',
        },
      })
      setDraftProviderId(preferred.providerId)
      setDraftModel(preferred.model)
      // 聊天里选过的模型才写回 settings，避免把 Lens/翻译回落误当成 Chat 默认。
      const stored = settings.defaultModels?.chat
      if (
        last
        && last.providerId === preferred.providerId
        && last.model === preferred.model
        && (stored?.providerId !== last.providerId || stored?.model !== last.model)
      ) {
        void persistLastChatModelToSettings(last.providerId, last.model)
      }
    } catch {
      setDraftProviderId('dev-provider')
      setDraftModel('dev-model')
    }
  }, [setDraftModel, setDraftProviderId])

  const skillProjectCwdRef = useRef('')
  const loadSkills = useCallback(async () => {
    if (!isTauriRuntime()) {
      setSkills([])
      return
    }
    try {
      const result = await api.chatSkillsList(undefined, skillProjectCwdRef.current || undefined)
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
    return subscribeSettings((next) => {
      setUiLang((next.settingsLanguage as Lang) || 'zh')
    })
  }, [])

  // 空闲预取各中心页 chunk，避免首次切到设置/专家/技能/MCP 时才触发 lazy import 而转圈；
  // 预取后切换时 Suspense 不再挂起，chat-motion-view-in 动画得以播在真实内容上（而非 spinner）。
  useEffect(() => {
    return scheduleIdleTask(() => {
      void importSettingsShell()
      void import('./AssistantCenter')
      void import('./SkillCenter')
      void import('./McpCenter')
      void import('./KnowledgeCenter')
      void import('./NotesCenter')
      void import('./automation/AutomationCenter')
      void import('./MessageList')
    }, 400)
  }, [])

  const onReturnedToConversation = useCallback(() => {
    void loadSkills()
    void refreshToolIndicator()
  }, [loadSkills, refreshToolIndicator])

  const {
    settingsExiting,
    closeSettings: handleSettingsClose,
    runAfterLeavingSettings,
  } = useSettingsExit({
    chatView,
    setChatView,
    settingsRef,
    currentConversationIdRef,
    syncConversationRoute,
    onReturnedToConversation,
  })

  const handleSettingsChange = useCallback(() => {
    onSettingsChange()
    void loadDefaultModel()
    void loadSkills()
    void refreshToolIndicator()
    setSidebarProfileRefreshKey((key) => key + 1)
  }, [loadDefaultModel, loadSkills, onSettingsChange, refreshToolIndicator])

  useEffect(() => {
    void popoutOwner.list().catch((err) => console.error('Failed to list conversation popouts:', err))
  }, [popoutOwner])

  useTauriEvent(api.onConversationPopoutsChanged, (payload) => {
    const change = popoutOwner.changed(payload.conversationIds)
    setExclusiveConversationIds(change.next)
    void navigation.reconcilePopouts(change.previous, change.next)
  }, [navigation, popoutOwner])

  useEffect(() => {
    if (!popoutNotice) return
    const timer = window.setTimeout(() => setPopoutNotice(null), 4000)
    return () => window.clearTimeout(timer)
  }, [popoutNotice])

  const finishStreamingRun = useCallback(
    async (payload: { reason?: string; conversationId?: string; runId?: string | null; turnEpoch?: number }) => {
      const conversationId = payload.conversationId ?? currentConversationIdRef.current
      if (!conversationId) return
      const terminalEpoch = payload.turnEpoch ?? executionOwner.turnEpoch(conversationId)
      const canCommit = () => executionOwner.turnEpoch(conversationId) === terminalEpoch
      const navigationLease = captureConversationNavigation()
      const canPresent = () => canCommit() && isCurrentConversationNavigation(navigationLease)
      if (!canCommit()) return
      interactionInbox.observe(payload.runId
        ? { kind: 'runTerminal', conversationId, runId: payload.runId }
        : { kind: 'drop', conversationId })
      const preservedPartial = payload.reason === 'error' ? freezeStreamSnapshot(conversationId) : false
      // 兜底：run 结束时压缩必然已终止；防御后端遗漏终止事件把"压缩中"状态卡死。
      markConversationCompacting(conversationId, false)
      if (payload.reason === 'error') {
        setStreamErrorForConversation(
          conversationId,
          streamErrorsRef.current[conversationId] || '回复生成失败，请稍后重试。',
        )
      }
      if (currentConversationIdRef.current === conversationId) {
        try {
          await reloadConversation(conversationId, { force: true, canCommit: canPresent })
        } catch (error) {
          if (canPresent() && currentConversationIdRef.current === conversationId) {
            const message = error instanceof Error ? error.message : String(error)
            setStreamErrorForConversation(conversationId, `回复已结束，但会话回载失败；重新打开此会话重试：${message}`)
          }
        }
      }
      if (!canCommit()) return
      refreshSidebar()
      // The invoke owner already retired this local run before invoking the
      // terminal port. A new run may have started during the read, so never
      // retire execution again by bare conversation ID.
      if (!preservedPartial) settleStreamingPreview(conversationId)
      syncGeneratingConversationIds()
    },
    [executionOwner, freezeStreamSnapshot, interactionInbox, markConversationCompacting, refreshSidebar, reloadConversation, setStreamErrorForConversation, settleStreamingPreview, syncGeneratingConversationIds],
  )

  const finishExternalStreamingRun = useCallback((ready: Extract<StreamLifecycleResult, { kind: 'ready' }>) => {
    const { conversationId, reason } = ready.terminal
    const navigationLease = captureConversationNavigation()
    const canPresent = () => isCurrentConversationNavigation(navigationLease)
      && currentConversationIdRef.current === conversationId
      && !popoutOwner.owns(conversationId)
    void streamLifecycleOwner.settleExternalTerminal(
      ready.permit,
      // Reading is side-effect-free. A stale terminal must not call the
      // navigation helper, which applies its result before the run permit is
      // checked again.
      () => canPresent() ? chatApi.getConversation(conversationId) : Promise.resolve(null),
      (outcome) => {
        const conversation = outcome.kind === 'loaded' ? outcome.value : null
        const loadError = outcome.kind === 'failed' ? outcome.error : null
        if (conversation && canPresent()) {
          applyConversation(conversation)
        }
        markConversationCompacting(conversationId, false)
        const preservePartial = reason === 'error' || Boolean(loadError)
        if (preservePartial) {
          if (!freezeStreamSnapshot(conversationId)) clearStreamSnapshot(conversationId)
        } else {
          // Same settle contract as the built-in path: judge "has the twin landed"
          // against the messages React has *committed* (`currentConversationRef`),
          // never against the list `applyConversation` just handed to setState.
          // The freshly loaded list always contains the twin, so passing it made
          // `complete` clear the live bubble synchronously (SyncLane) one frame
          // before the DefaultLane conversation update painted the twin — the
          // "live unmounted, twin not yet there" blank frame at run end.
          settleStreamingPreview(conversationId)
        }
        if (loadError && canPresent()) {
          setStreamErrorForConversation(
            conversationId,
            `回复已结束，但会话回载失败；重新打开此会话重试：${loadError.message}`,
          )
        } else if (reason === 'error' && canPresent()) {
          setStreamErrorForConversation(
            conversationId,
            streamErrorsRef.current[conversationId] || '回复生成失败，请稍后重试。',
          )
        }
        refreshSidebar()
        syncGeneratingConversationIds()
      },
    )
  }, [
    applyConversation, clearStreamSnapshot, freezeStreamSnapshot, markConversationCompacting,
    popoutOwner, refreshSidebar, setStreamErrorForConversation, settleStreamingPreview,
    streamLifecycleOwner, syncGeneratingConversationIds,
  ])

  // React 的权威消息提交后才清 live 预览；定时兜底和旧轮失效归 previewOwner。
  useEffect(() => {
    if (currentConversation) previewOwner.reconcile(currentConversation.id, currentConversation.messages)
  }, [currentConversation, previewOwner])

  const finishStreamingRunWithConversation = useCallback((
    conversationId: string,
    conversation: Conversation,
  ) => {
    if (currentConversationIdRef.current === conversationId) {
      applyConversation(conversation)
    }
    interactionInbox.observe({ kind: 'drop', conversationId })
    settleStreamingPreview(conversationId)
    syncGeneratingConversationIds()
  }, [applyConversation, interactionInbox, settleStreamingPreview, syncGeneratingConversationIds])

  const settlementPorts = useMemo<Parameters<ReturnType<typeof createChatExecutionOwner>['finish']>[2]>(() => ({
      completeWithConversation: finishStreamingRunWithConversation,
      completeTerminal: finishStreamingRun,
      abandonPreview: (id) => {
        if (!freezeStreamSnapshot(id)) clearStreamSnapshot(id)
      },
      settleQueue: (id, conversation) => {
        const delivery = queueCommands.settleAfterRun(id, conversation)
        void wakeAfterRun()
        return delivery
      },
    }), [clearStreamSnapshot, finishStreamingRun, finishStreamingRunWithConversation, freezeStreamSnapshot, queueCommands, wakeAfterRun])

  useTauriEvent(api.onChatProtocolIssue, ({ issue, conversationId }) => {
    if (issue === 'version_mismatch') {
      setProtocolVersionMismatch(true)
    } else if (
      issue === 'resync_required'
      && conversationId
      && conversationId === currentConversationIdRef.current
      && !popoutOwner.owns(conversationId)
    ) {
      void reloadConversation(conversationId)
    }
  }, [reloadConversation])

  useTauriEvent(api.onChatStream, (payload) => {
      const popout = popoutOwner.observeRun(payload)
      // Popout ownership hides main-window display only. Execution identity
      // still observes every arm, including terminals arriving after close.
      const result = streamLifecycleOwner.receive(payload, { project: !popout.suppressMainProjection })
      if (result.kind === 'ignored') return
      if (popout.suppressMainProjection) {
        syncGeneratingConversationIds()
        if (result.kind === 'ready') finishExternalStreamingRun(result)
        return
      }
      if (isStreamTerminal(payload)) {
        interactionInbox.observe({
          kind: 'runTerminal', conversationId: payload.conversationId, runId: payload.runId,
        })
      }
      if (result.kind === 'started') {
        interactionInbox.observe({
          kind: 'runStarted', conversationId: payload.conversationId, runId: payload.runId,
        })
        syncGeneratingConversationIds()
        return
      }
      syncGeneratingConversationIds()
      if (result.kind === 'ready') finishExternalStreamingRun(result)
  }, [finishExternalStreamingRun, interactionInbox, popoutOwner, streamLifecycleOwner, syncGeneratingConversationIds])

  useTauriEvent(api.onChatTodo, (payload) => {
    const currentConversationId = currentConversationIdRef.current
    if (!currentConversationId || payload.conversationId !== currentConversationId) {
      return
    }
    patchAgentTodoState(payload.todoState)
  }, [patchAgentTodoState])

  useTauriEvent(api.onChatPlan, (payload) => {
    const currentConversationId = currentConversationIdRef.current
    if (!currentConversationId || payload.conversationId !== currentConversationId) {
      return
    }
    patchAgentPlanState(payload.planState)
  }, [patchAgentPlanState])

  useTauriEvent(api.onChatGoal, (payload) => {
    const currentConversationId = currentConversationIdRef.current
    if (!currentConversationId || payload.conversationId !== currentConversationId) return
    patchGoalState(payload.goalState)
  }, [patchGoalState])

  useTauriEvent(api.onChatTitle, ({ conversationId }) => {
    if (currentConversationIdRef.current === conversationId) {
      void chatApi.getConversation(conversationId).then((updated) => {
        applyConversationIfCurrent(conversationId, updated)
      }).catch((error) => console.error('Failed to refresh generated title:', error))
    }
    refreshSidebar()
  }, [applyConversationIfCurrent, refreshSidebar])

  useTauriEvent(api.onChatHook, (payload) => {
    const currentConversationId = currentConversationIdRef.current
    if (!currentConversationId || payload.conversationId !== currentConversationId) {
      return
    }
    setHookWarning(payload)
  }, [])

  useTauriEvent(api.onChatTool, (payload) => {
      if (['agent', 'agent_control', 'native__agent', 'native__agent_control'].includes(payload.name) && payload.status === 'success') refreshSubAgents(payload.conversationId)
      if (popoutOwner.owns(payload.conversationId)) return
      if (!executionOwner.allowsStreamPayload(payload)) return
      // 忽略 invoke 结束后的迟到 tool 事件，否则会重新 setStreaming(true) 卡死输入栏。
      if (!executionOwner.snapshot(payload.conversationId).inFlight) return
      if (!executionOwner.observe({ kind: 'runEvent', conversationId: payload.conversationId, runId: payload.runId })) return
      const result = previewOwner.projectDisplay({ kind: 'tool', payload })
      if (!result.accepted) return
      // 插话卡到了 = 那条「立刻引导」真的进了模型历史，现在才把它从队列里摘掉。
      // （在此之前它一直留着，好让「没赶上轮次边界」退化成运行结束后的自动发送。）
      if (result.confirmedQueueMessageId) {
        queueCommands.confirm(payload.conversationId, result.confirmedQueueMessageId)
      }
      syncGeneratingConversationIds()
  }, [executionOwner, previewOwner, queueCommands, syncGeneratingConversationIds])

  // Live nested sub-agent progress (P3): merge onto the parent tool card's
  // structuredContent.subagentProgress, addressed by parentToolCallId.
  // 流状态行的瞬态一行字（上游重试等）：写进会话流快照，StreamStatusLine 每秒读。
  // 清除有两条路：后端显式 note=null，或正文/思考恢复流动（onChatStream 的 delta 分支）。
  useTauriEvent(api.onChatStatusNote, (payload) => {
    if (!executionOwner.allowsStreamPayload(payload)) return
    if (!executionOwner.observe({ kind: 'runEvent', conversationId: payload.conversationId, runId: payload.runId })) return
    previewOwner.projectDisplay({ kind: 'status', payload })
  }, [executionOwner, previewOwner])

  useTauriEvent(api.onChatSubagent, (payload) => {
      // 父轮还在飞：写流快照。父轮已经收尾后旧预览仍可能留着死快照，
      // 不能再当直播通道，否则步骤写进看不见的对象，卡上永远「运行中…」。
      const inFlight = executionOwner.snapshot(payload.parentConversationId).inFlight
      if (inFlight) {
        if (!executionOwner.allowsStreamPayload({ conversationId: payload.parentConversationId, runId: payload.parentRunId })) return
        if (!executionOwner.observe({ kind: 'runEvent', conversationId: payload.parentConversationId, runId: payload.parentRunId })) return
        previewOwner.projectDisplay({ kind: 'subagent', payload })
        return
      }
      if (currentConversationIdRef.current !== payload.parentConversationId) return
      setCurrentConversation((prev) => {
        if (!prev || prev.id !== payload.parentConversationId) return prev
        let changed = false
        const messages = prev.messages.map((message) => {
          const tools = messageToolCalls(message)
          const index = findSubagentToolIndex(tools, payload)
          if (index < 0) return message
          changed = true
          const nextTools = tools.map((item, i) => (
            i === index ? mergeSubagentProgress(item, payload) : item
          ))
          return { ...message, toolCalls: nextTools, tool_calls: nextTools }
        })
        return changed ? { ...prev, messages } : prev
      })
  }, [executionOwner, previewOwner])

  useTauriEvent(api.onChatUserPrompt, (payload) => {
      if (popoutOwner.owns(payload.conversationId)) return
      if (!executionOwner.allowsStreamPayload(payload)) return
      if (!executionOwner.snapshot(payload.conversationId).inFlight) return
      if (!executionOwner.observe({ kind: 'runEvent', conversationId: payload.conversationId, runId: payload.runId })) return
      if (!previewOwner.projectDisplay({ kind: 'userPrompt', payload }).accepted) return
      // 同时排进「输入框上方」那张面板的队列：消息流里的那条只是痕迹，真正作答在面板上。
      interactionInbox.observe({ kind: 'userPromptRequested', payload })
      syncGeneratingConversationIds()
  }, [executionOwner, interactionInbox, previewOwner, syncGeneratingConversationIds])

  /** 面板作答完（或那一轮结束了）就把它收起来。后端没有「已答复」事件 ——
   *  `resolve_user_prompt` 只清重放快照、不发事件，所以收起由前端自己负责。 */
  const dismissPendingUserPrompt = useCallback((conversationId: string, runId: string, toolCallId: string) => {
    interactionInbox.observe({ kind: 'userAnswered', conversationId, runId, toolCallId })
  }, [interactionInbox])

  useTauriEvent(api.onChatToolConfirm, (payload) => {
    if (popoutOwner.owns(payload.conversationId)) return
    if (!executionOwner.allowsStreamPayload(payload)) return
    if (!executionOwner.observe({ kind: 'runEvent', conversationId: payload.conversationId, runId: payload.runId })) return
    // 排队而不是覆盖：一条消息里并行调多个工具时，后端会同时挂着多条询问等答复
    // （按 request_id 路由）。覆盖会让用户没看见的那条静默超时 ⇒ 模型收到「用户拒绝」。
    const queued = interactionInbox.observe({ kind: 'toolRequested', payload })
    if (queued && currentConversationIdRef.current === payload.conversationId) {
      // 计划卡一出现就在右侧栏摊开整份计划 —— 卡片上那块小灰框读不完。
      if (isPlanApproval(payload) && payload.argumentsPreview?.trim()) {
        requestDockMarkdownPreview({ title: '计划', text: payload.argumentsPreview })
      }
    }
  }, [executionOwner, interactionInbox])

  useTauriEvent(api.onChatToolConfirmWithdraw, (payload) => {
    if (popoutOwner.owns(payload.conversationId)) return
    // 旧适配器只暴露 conversationId + toolCallId，没有 runId；撤销是清理事件，
    // 取消栅栏不能阻断它，否则已经超时的卡片会留在界面。run 身份由 inbox 的
    // request_id 队列定位；协议迁移时应补回 runId。
    interactionInbox.observe({ kind: 'toolWithdrawn', ...payload })
  }, [interactionInbox])

  const resolvePendingToolConfirm = useCallback(async (
    approved: boolean,
    always = false,
    permissionMode: string | null = null,
  ): Promise<boolean> => interactionInbox.respondTool({ approved, always, permissionMode }), [interactionInbox])

  useTauriEvent(api.onChatSessionConsent, (payload) => {
    if (popoutOwner.owns(payload.conversationId)) return
    if (!executionOwner.allowsStreamPayload(payload)) return
    if (!executionOwner.observe({ kind: 'runEvent', conversationId: payload.conversationId, runId: payload.runId })) return
    interactionInbox.observe({ kind: 'consentRequested', payload })
  }, [executionOwner, interactionInbox])

  const resolvePendingSessionConsent = useCallback((granted: boolean): Promise<boolean> => (
    interactionInbox.respondConsent(granted)
  ), [interactionInbox])

  useEffect(() => {
    const conversationId = currentConversation?.id
    if (!conversationId) return
    // 已弹出的会话:主窗只渲染占位卡、不渲染消息,run 边沿事件足够;
    // sync 会把该会话的运行快照(可能数百 KB)白拉到主窗协议状态里。
    if (popoutOwner.owns(conversationId)) return
    void api.chatSyncState(conversationId).catch((error) => {
      console.error('Failed to synchronize chat protocol state:', error)
    })
  }, [currentConversation?.id, popoutConversationIds, popoutOwner])

  useEffect(() => {
    currentConversationIdRef.current = currentConversation?.id ?? null
  }, [currentConversation?.id])

  useEffect(() => {
    if (!currentConversation?.id || chatView !== 'conversation') {
      setContextLoading(false)
      return
    }
    void refreshContextStats(currentConversation.id)
  }, [chatView, currentConversation?.id, activeModel, effectiveSkillId, refreshContextStats, setContextLoading])

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
    if (!isTauriRuntime()) return
    let cancelled = false
    void getSettingsCached().then((settings) => {
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
      void navigation.openConversation(payload.conversationId, { reload: payload.reload })
      refreshSidebar()
    }).then((dispose) => {
      if (cancelled) dispose()
      else unlisten = dispose
    }).catch(err => console.error(err))

    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [navigation, refreshSidebar])

  const handleConversationFirstCommit = useCallback((conversationId: string, requestId: number) => {
    window.requestAnimationFrame(() => {
      completeConversationTransition(conversationId, requestId)
    })
  }, [])

  const handleNewConversation = useCallback(async () => {
    navigation.startNewConversation()
  }, [navigation])

  const handleClearChat = useCallback(async () => {
    await navigation.clearCurrentChat()
  }, [navigation])

  const assistantIdentity = useMemo(() => ({
    activeProviderId,
    activeModel,
    projectId: selectedProject?.id ?? null,
    projectName: selectedProject?.name,
    setId: selectedSet?.id ?? null,
  }), [activeModel, activeProviderId, selectedProject?.id, selectedProject?.name, selectedSet?.id])

  const {
    startAssistantChat: handleStartAssistantChat,
    startBuilderChat: handleStartBuilderChat,
    applyAssistant: handleApplyAssistant,
    selectAssistant: handleSelectAssistant,
  } = useAssistantActions({
    currentConversationRef,
    navigation,
    identity: assistantIdentity,
    refreshSidebar,
    refreshContextStats,
    applyConversationIfCurrent,
    setStreamErrorForConversation,
    setAssistantStreamStatsByMessageId,
  })

  const ensureConversationForAgentPlan = useCallback(async (
    creation: ReturnType<typeof navigation.beginConversationCreation> | null,
  ) => {
    if (currentConversation) return currentConversation
    if (!creation) throw new Error('缺少创建会话的导航意图')
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
    refreshSidebar()
    navigation.commitCreatedConversation(creation, conversation)
    return conversation
  }, [activeModel, activeProviderId, currentConversation, draftAgentRuntime, navigation, refreshSidebar, selectedProject?.id, selectedProject?.name, selectedSet?.id])

  const handleAgentPlanModeChange = useCallback(async (mode: AgentPlanMode) => {
    const creation = currentConversation ? null : navigation.beginConversationCreation()
    let targetConversationId = currentConversation?.id ?? null
    try {
      const conversation = await ensureConversationForAgentPlan(creation)
      targetConversationId = conversation.id
      const updated = await chatApi.setAgentPlanMode(conversation.id, mode)
      applyConversationIfCurrent(conversation.id, updated)
      void refreshContextStats(updated.id)
      refreshSidebar()
    } catch (err) {
      console.error('Failed to update agent plan mode:', err)
      if (targetConversationId) {
        setStreamErrorForConversation(
          targetConversationId,
          typeof err === 'string' ? err : (err as Error).message || 'Plan 模式切换失败',
        )
      } else if (creation && navigation.isConversationCreationCurrent(creation)) {
        setStreamError(typeof err === 'string' ? err : (err as Error).message || 'Plan 模式切换失败')
      }
    }
  }, [applyConversationIfCurrent, currentConversation, ensureConversationForAgentPlan, navigation, refreshContextStats, refreshSidebar, setStreamErrorForConversation])

  const handleSelectProject = useCallback((project: ChatProject | null) => {
    setSelectedProject(project)
    setSelectedSet(null)
    setAssistantStreamStatsByMessageId({})
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
    const snapshot = previewOwner.timing(updatedConv.id)
    if (!lastAssistant || !snapshot?.startedAt) return

    const elapsedSec = Math.max((Date.now() - snapshot.startedAt) / 1000, 0.1)
    const streamedText = `${snapshot.content}${snapshot.reasoning ? `\n${snapshot.reasoning}` : ''}`
    const tokenEstimate = estimateTokens(
      streamedText.trim().length > 0
        ? streamedText
        : `${lastAssistant.content}${lastAssistant.reasoning ? `\n${lastAssistant.reasoning}` : ''}`,
    )
    const stats: AssistantStreamStats = {
      messageId: lastAssistant.id,
      tokensPerSec: tokenEstimate / elapsedSec,
      reasoningDurationMs: snapshot.reasoningDurationMs,
      reasoningDurationMsBySegmentId: snapshot.reasoningDurationMsBySegmentId,
    }
    setAssistantStreamStatsByMessageId((prev) => ({
      ...prev,
      [lastAssistant.id]: stats,
    }))
  }, [previewOwner])

  const presentSendEvent = useCallback((event: SendPresentationEvent) => {
    if (event.kind === 'created') {
      event.creation?.commit(event.conversation)
      return
    }
    if (event.kind === 'updated') {
      applyConversationIfCurrent(event.conversation.id, event.conversation)
      return
    }
    if (event.kind === 'rejected') {
      if (event.conversationId) {
        setStreamErrorForConversation(event.conversationId, event.error.message)
      } else if (event.creation?.isCurrent()
        || (!event.creation && currentConversationIdRef.current === event.startingConversationId)) {
        setStreamError(event.error.message || '创建对话失败')
      }
      return
    }
    if (event.kind === 'started') {
      const { conversation, content, attachments } = event
      setOptimisticSidebarConversations((items) => [
        optimisticConversationListItem(
          conversation, content, attachments.map((attachment) => attachment.name),
        ),
        ...items.filter((item) => item.id !== conversation.id),
      ])
      syncGeneratingConversationIds()
      if (currentConversationIdRef.current === conversation.id) {
        setStreamErrorForConversation(conversation.id, '')
        setHookWarning(null)
      }
      return
    }
    if (event.kind === 'settled') {
      syncGeneratingConversationIds()
      return
    }
    const { conversationId, outcome } = event
    if (outcome.kind === 'persisted') {
      if (currentConversationIdRef.current === conversationId) {
        applyAssistantStreamStats(outcome.conversation)
        setOptimisticSidebarConversations((items) =>
          settleOptimisticConversationListItems(items, conversationId, outcome.conversation))
        applyConversation(outcome.conversation)
      }
      refreshSidebar()
      return
    }
    console.error('Failed to send message:', outcome.error)
    const keptConversation = outcome.kind === 'persisted_error' ? outcome.conversation : null
    if (keptConversation && currentConversationIdRef.current === conversationId) {
      applyConversation(keptConversation)
    }
    setOptimisticSidebarConversations((items) =>
      settleOptimisticConversationListItems(items, conversationId, keptConversation))
    if (keptConversation) refreshSidebar()
    setStreamErrorForConversation(conversationId, outcome.error.message || '发送失败')
    if (!freezeStreamSnapshot(conversationId)) clearStreamSnapshot(conversationId)
  }, [
    applyAssistantStreamStats, applyConversation, applyConversationIfCurrent,
    clearStreamSnapshot, freezeStreamSnapshot, refreshSidebar,
    setStreamErrorForConversation, syncGeneratingConversationIds,
  ])

  const sendController = useMemo(() => createChatSendController({
    executionOwner,
    previewOwner,
    persistence: chatApi,
    settlementPorts,
    presentation: {
      currentConversationId: () => currentConversationIdRef.current,
      beginCreation: () => {
        const permit = navigation.beginConversationCreation()
        return {
          isCurrent: () => navigation.isConversationCreationCurrent(permit),
          commit: (conversation) => navigation.commitCreatedConversation(permit, conversation),
        }
      },
      present: presentSendEvent,
    },
  }), [executionOwner, navigation, previewOwner, settlementPorts, presentSendEvent])

  const handleSendMessage = useCallback(async (
    content: string,
    attachments: PendingAttachment[] = [],
    options: SendMessageOptions = {},
  ) => {
    const attachmentSkillId = resolveSendSkillId(
      attachments, enabledSkills, options.forceNewConversation ? null : effectiveSkillId, usesChatRuntime,
    )
    const result = await sendController.send({
      content,
      attachments,
      preparation: {
        conversation: options.conversationOverride ?? currentConversation,
        override: Boolean(options.conversationOverride),
        forceNew: Boolean(options.forceNewConversation),
        providerId: activeProviderId,
        model: activeModel,
        projectName: selectedProject?.name ?? null,
        projectId: selectedProject?.id ?? null,
        setId: selectedSet?.id ?? null,
        draft: {
          agentRuntime: draftAgentRuntime,
          knowledgeBaseIds: draftKnowledgeBaseIds,
          forceKnowledgeSearch: draftForceKnowledgeSearch,
          additionalDirectories: draftAdditionalDirectories,
          thinkingLevel: draftThinkingLevel,
          webSearchMode: draftWebSearchMode,
          rememberedWebSearchMode: loadLastWebSearchMode(),
          replyModels: draftReplyModels,
        },
        providerOAuthTypes,
      },
      attachmentSkillId,
      disabledReason: sendDisabledReason,
      planMessageId: options.planMessageId,
      onPartialConversation: options.onPartialConversation,
      onAccepted: options.onAccepted,
    })
    return result.composerAccepted
  }, [
    activeModel, activeProviderId, currentConversation, draftAgentRuntime,
    draftKnowledgeBaseIds, draftForceKnowledgeSearch, draftAdditionalDirectories,
    draftThinkingLevel, draftReplyModels, draftWebSearchMode, providerOAuthTypes,
    effectiveSkillId, enabledSkills, usesChatRuntime, selectedProject?.id,
    selectedProject?.name, selectedSet?.id, sendDisabledReason, sendController,
  ])
  // 历史预置（Lens「在 AI 客户端继续」交接）：用最新 reactive 值（provider/model/project）创建带历史的新会话。
  const importExternalConversation = useCallback(async (
    messages: { role: string; content: string }[],
    attachmentPaths: string[],
  ): Promise<boolean> => {
    const creation = navigation.beginConversationCreation()
    try {
      const conversation = await chatApi.importExternalConversation(
        messages,
        attachmentPaths,
        activeProviderId || undefined,
        activeModel || undefined,
        selectedProject?.id ?? null,
      )
      refreshSidebar()
      navigation.commitCreatedConversation(creation, conversation)
      return true
    } catch (err) {
      console.error('Failed to import external conversation:', err)
      if (navigation.isConversationCreationCurrent(creation)) {
        setStreamError(typeof err === 'string' ? err : (err as Error).message || '导入对话失败')
      }
      return false
    }
  }, [activeModel, activeProviderId, navigation, refreshSidebar, selectedProject?.id])

  const currentQueuedMessages = currentConversation
    ? messageQueue.queued[currentConversation.id] ?? NO_QUEUED_MESSAGES
    : NO_QUEUED_MESSAGES
  // 「立刻引导」能不能给入口，取决于这一轮由谁在跑：
  //   - 内置 agent 循环 → 能（轮首注入，见 chat/agent/steering.rs）；
  //   - 外部 CLI → 看它的协议支不支持（`supportsSteering`，后端 RuntimeAgentDef 是唯一真源）。
  //     codex 有 `turn/steer`；pi 有 RPC `steer`；dsh 有 bridge `session/steer`。
  //     claude 的 stream-json 输入是顺序处理的、ACP 只有 prompt/cancel。
  //   - 多模型一问多答 → 一律不给：同会话 N 条并发 run，按 conversation 键的信箱定不到某条臂。
  const activeExternalAgentSupportsSteering = useMemo(() => {
    const agentId = activeAgentRuntime.externalAgentId
    if (!agentId) return false
    const agent = detectedExternalAgents.find((item) => item.id === agentId)
    return Boolean(agent?.supportsSteering ?? agent?.supports_steering)
  }, [activeAgentRuntime.externalAgentId, detectedExternalAgents])
  const activeExternalAgentSupportsFollowUp = useMemo(() => {
    const agentId = activeAgentRuntime.externalAgentId
    if (!agentId) return false
    const agent = detectedExternalAgents.find((item) => item.id === agentId)
    return Boolean(agent?.supportsFollowUp ?? agent?.supports_follow_up)
  }, [activeAgentRuntime.externalAgentId, detectedExternalAgents])
  const canSteerCurrentConversation =
    (usesExternalRuntime ? activeExternalAgentSupportsSteering : true)
    && activeReplyModels.length < 2
  // Goal 的用户输入必须先于自动续跑，因此复用原生 follow-up；普通内置循环仍保留
  // 可见队列和「立刻引导」。外部 CLI 仅在协议原生支持时启用，多模型一问多答不给。
  const canFollowUpCurrentConversation =
    ((usesExternalRuntime && activeExternalAgentSupportsFollowUp) || goalActive)
    && activeReplyModels.length < 2

  const handleQueueMessage = useCallback((content: string, attachments: PendingAttachment[]) => {
    const conversation = currentConversationRef.current
    if (!conversation) return
    const message = queueCommands.enqueue(conversation.id, content, attachments)
    if (message && canFollowUpCurrentConversation) {
      void queueCommands.followUp(conversation, message.id)
    }
  }, [canFollowUpCurrentConversation, queueCommands])

  const [closedAsyncQuestions, setClosedAsyncQuestions] = useState<Record<string, string[]>>({})
  const asyncQuestionsValue = useMemo(() => {
    const conversationId = currentConversation?.id ?? ''
    const closedIds = new Set(closedAsyncQuestions[conversationId] ?? [])
    // A later user turn supersedes earlier questions, including after app restart.
    let hasLaterUser = false
    for (const message of [...(currentConversation?.messages ?? [])].reverse()) {
      if (message.role === 'user') hasLaterUser = true
      if (hasLaterUser) {
        for (const tool of message.tool_calls ?? message.toolCalls ?? []) closedIds.add(tool.id)
      }
    }
    return {
      closedIds,
      reply: async (toolId: string, text: string | null) => {
        const conversation = currentConversationRef.current
        if (!conversation || conversation.id !== conversationId) throw new Error('对话已切换，请重试')
        if (text) {
          const queued = queueCommands.enqueue(conversation.id, text, [])
          if (!queued) throw new Error('答复未能加入消息队列，请重试')
          if (!generatingConversationIdsRef.current.has(conversation.id)) {
            void queueCommands.drain(conversation)
          }
        }
        setClosedAsyncQuestions((previous) => ({
          ...previous, [conversationId]: [...(previous[conversationId] ?? []), toolId],
        }))
      },
    }
  }, [closedAsyncQuestions, currentConversation, queueCommands])

  const handleSteerQueuedMessage = useCallback((messageId: string) => {
    const conversationId = currentConversationIdRef.current
    if (!conversationId) return
    void queueCommands.steer(conversationId, messageId)
  }, [queueCommands])

  const handleRemoveQueuedMessage = useCallback((messageId: string) => {
    const conversationId = currentConversationIdRef.current
    if (!conversationId) return
    queueCommands.remove(conversationId, messageId)
  }, [queueCommands])

  const handleRestoreQueuedMessage = useCallback((messageId: string) => {
    const conversationId = currentConversationIdRef.current
    if (!conversationId) return
    queueCommands.restoreToComposer(conversationId, messageId)
  }, [queueCommands])

  const handleExecuteAgentPlan = useCallback(async (messageId: string) => {
    const conversation = currentConversation
    if (!conversation) return
    if (executionOwner.snapshot(conversation.id).inFlight) {
      setStreamErrorForConversation(conversation.id, '该对话正在生成中，请稍后再试')
      return
    }

    try {
      await handleSendMessage('按这条计划开始执行。', [], {
        conversationOverride: conversation,
        planMessageId: messageId,
      })
    } catch (err) {
      console.error('Failed to execute agent plan:', err)
      setStreamErrorForConversation(
        conversation.id,
        typeof err === 'string' ? err : (err as Error).message || '执行计划失败',
      )
    }
  }, [
    currentConversation,
    executionOwner,
    handleSendMessage,
    setStreamErrorForConversation,
  ])

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

  const {
    updateMessage: handleUpdateMessage,
    deleteMessage: handleDeleteMessage,
    rewindToMessage: handleRewindMessage,
    forkAtMessage: handleForkMessage,
    saveMessageToNote: handleSaveMessageToNote,
    setGroupSelection: handleSetGroupSelection,
  } = useMessageActions({
    currentConversationRef,
    navigation,
    applyConversationIfCurrent,
    applyConversationMeta,
    setStreamErrorForConversation,
    setAssistantStreamStatsByMessageId,
    refreshSidebar,
    refreshContextStats,
  })

  const presentRunCommandEvent = useCallback((event: RunCommandPresentationEvent) => {
    if (event.kind === 'truncated') {
      applyConversation(event.conversation)
      const removed = new Set(event.removedMessageIds)
      setAssistantStreamStatsByMessageId((prev) => Object.fromEntries(
        Object.entries(prev).filter(([id]) => !removed.has(id)),
      ))
      return
    }
    if (event.kind === 'started') {
      syncGeneratingConversationIds()
      if (currentConversationIdRef.current === event.conversationId) {
        setStreamErrorForConversation(event.conversationId, '')
      }
      return
    }
    if (event.kind === 'persisted') {
      if (currentConversationIdRef.current === event.conversationId) {
        applyAssistantStreamStats(event.conversation)
        applyConversation(event.conversation)
      }
      refreshSidebar()
      return
    }
    if (event.kind === 'failed') {
      setStreamErrorForConversation(event.conversationId, event.error.message)
      if (event.clearPreview) clearStreamSnapshot(event.conversationId)
      else syncGeneratingConversationIds()
      if (currentConversationIdRef.current === event.conversationId) {
        void reloadConversation(event.conversationId)
      }
      return
    }
    if (event.kind === 'rejected') {
      setStreamErrorForConversation(event.conversationId, event.error.message)
      return
    }
    syncGeneratingConversationIds()
  }, [
    applyAssistantStreamStats, applyConversation, clearStreamSnapshot,
    refreshSidebar, reloadConversation, setStreamErrorForConversation,
    syncGeneratingConversationIds,
  ])

  const runCommands = useMemo(() => createChatRunCommands({
    executionOwner,
    previewOwner,
    persistence: chatApi,
    settlementPorts,
    presentation: { present: presentRunCommandEvent },
  }), [executionOwner, previewOwner, settlementPorts, presentRunCommandEvent])

  const handleRegenerateMessage = useCallback(async (messageId: string, newContent?: string) => {
    await runCommands.regenerate({
      conversation: currentConversationRef.current,
      messageId,
      newContent,
    })
  }, [runCommands])

  const handleReplyWithModel = useCallback(async (
    messageId: string, providerId: string, model: string,
  ) => {
    await runCommands.replyWithModel({
      conversation: currentConversationRef.current,
      messageId,
      providerId,
      model,
    })
  }, [runCommands])

  // 底栏胶囊选档：本地 CLI 写沙盒档位；内置 Agent 写 Act/Plan/Orchestrate；Chat 运行时无胶囊。
  const handleComposerModeChange = useCallback(async (value: string) => {
    if (usesExternalRuntime) {
      await handleExternalSandboxChange(value)
      return
    }
    if (value === 'goal') {
      if (!goalActive) insertTextIntoComposer('/goal ')
      return
    }
    if (goalActive) {
      const conversationId = currentConversationIdRef.current
      if (conversationId) {
        const paused = await chatApi.pauseGoal(conversationId)
        applyConversationIfCurrent(conversationId, paused)
      }
    }
    await handleAgentPlanModeChange(value as AgentPlanMode)
  }, [applyConversationIfCurrent, goalActive, handleAgentPlanModeChange, handleExternalSandboxChange, usesExternalRuntime])

  const handleCancelStream = useCallback(async () => {
    const conversationId = currentConversationIdRef.current
    if (
      !conversationId
      || getStreamCoarse().cancelling
      || !(executionOwner.snapshot(conversationId).inFlight || previewOwner.isStreaming(conversationId))
    ) {
      return
    }

    const result = await streamLifecycleOwner.cancelRun(
      conversationId,
      () => chatApi.cancelStream(conversationId),
      () => {
        setStreamCoarse({ cancelling: true })
        interactionInbox.observe({ kind: 'drop', conversationId })
      },
    )
    if (result.kind === 'failed') {
      console.error('Failed to cancel chat stream:', result.error)
      syncGeneratingConversationIds()
      setStreamErrorForConversation(conversationId, result.error.message)
    }
    if (result.kind !== 'ignored' && result.kind !== 'superseded'
      && currentConversationIdRef.current === conversationId) setStreamCoarse({ cancelling: false })
  }, [executionOwner, interactionInbox, previewOwner, setStreamErrorForConversation, streamLifecycleOwner, syncGeneratingConversationIds])

  const displayMessages = executionOwner.overlayMessages(
    currentConversation?.id,
    currentConversation?.messages ?? [],
  )

  const hasMessages = displayMessages.length > 0
  const conversationOccupied = Boolean(
    currentConversation?.id && popoutConversationIds.has(currentConversation.id),
  )
  const showEmptyHero = chatView === 'conversation'
    && !conversationOccupied
    && !hasMessages
    && !streamCoarse.streaming
    && !streamCoarse.streamError

  // 输入栏是聊天主区里除 MessageList 外最大的常驻子树。把它的 slot 和对象值稳定下来，
  // 配合 InputBar 自身的 memo，侧栏/设置路由等无关状态变化不会再让输入栏重跑整棵树。
  const composerCurrentAssistant = useMemo(
    () => currentAssistantSnapshot
      ? { id: currentAssistantSnapshot.id, name: currentAssistantSnapshot.name }
      : null,
    [currentAssistantSnapshot],
  )
  const composerKnowledgeBaseIds = useMemo(
    () => currentConversation
      ? (currentConversation.knowledge_base_ids ?? currentConversation.knowledgeBaseIds ?? [])
      : draftKnowledgeBaseIds,
    [
      currentConversation,
      draftKnowledgeBaseIds,
    ],
  )
  const composerForceKnowledgeSearch = currentConversation
    ? (currentConversation.force_knowledge_search ?? currentConversation.forceKnowledgeSearch ?? false)
    : draftForceKnowledgeSearch
  const composerAdditionalDirectories = useMemo(
    () => currentConversation
      ? additionalDirectoriesOf(currentConversation)
      : draftAdditionalDirectories,
    [currentConversation, draftAdditionalDirectories],
  )
  const composerContextSlot = useMemo(
    () => (
      <ContextIndicator
        contextState={contextState}
        messageCount={displayMessages.length}
        lastMessageId={displayMessages[displayMessages.length - 1]?.id}
        loading={contextLoading}
        compressing={contextCompressing}
        generating={streamCoarse.streaming}
        error={contextError}
        usesExternalRuntime={usesExternalRuntime}
        onRefresh={handleRefreshContext}
        onCompress={handleCompressContext}
        onClear={usesExternalRuntime ? undefined : handleClearContext}
        lang={uiLang}
      />
    ),
    [
      contextCompressing,
      contextError,
      contextLoading,
      contextState,
      displayMessages,
      handleClearContext,
      handleCompressContext,
      handleRefreshContext,
      streamCoarse.streaming,
      uiLang,
      usesExternalRuntime,
    ],
  )
  const composerUsageSlot = useMemo(
    () => (
      <SessionUsageStrip
        messages={displayMessages}
        lang={uiLang}
        apiFormats={providerApiFormats}
        defaultApiFormat={currentConversation ? (providerApiFormats[currentConversation.provider_id] ?? '') : ''}
        cacheIncludedInInput={
          usesExternalRuntime
            ? activeAgentRuntime.externalAgentId === 'codex'
            : undefined
        }
      />
    ),
    [
      activeAgentRuntime.externalAgentId,
      currentConversation,
      displayMessages,
      providerApiFormats,
      uiLang,
      usesExternalRuntime,
    ],
  )

  // ---------- Right Dock ----------
  const dock = useRightDock({
    conversationId: currentConversation?.id ?? null,
    projectId: selectedProject?.id ?? null,
    agentRuntimeKind: activeAgentRuntime.kind,
    currentConversationIdRef,
  })
  const {
    open: dockOpen,
    workdir: dockWorkdir,
    toggle: handleToggleDock,
    openGit: handleOpenDockGit,
    openTasks: handleOpenDockTasks,
  } = dock

  useEffect(() => {
    const prev = skillProjectCwdRef.current
    skillProjectCwdRef.current = dockWorkdir
    if (prev !== dockWorkdir) void loadSkills()
  }, [dockWorkdir, loadSkills])

  const handleSidebarSelectProject = useCallback((project: ChatProject | null) => {
    runAfterLeavingSettings(() => handleSelectProject(project))
  }, [handleSelectProject, runAfterLeavingSettings])

  const handleSidebarSelectSet = useCallback((set: ChatSet | null) => {
    runAfterLeavingSettings(() => handleSelectSet(set))
  }, [handleSelectSet, runAfterLeavingSettings])

  const handleSidebarSelectConversation = useCallback((
    id: string,
    conversation?: ConversationListItem | ConversationSearchHit,
    scope?: ConversationSelectionScope,
  ) => {
    const focusMessageId =
      conversation && 'match_message_id' in conversation
        ? conversation.match_message_id ?? conversation.matchMessageId ?? undefined
        : conversation && 'matchMessageId' in conversation
          ? conversation.matchMessageId ?? undefined
          : undefined
    runAfterLeavingSettings(() => {
      // 跨项目/集点击必须是一次原子导航。这里仅更新导航上下文，不调用
      // handleSelectProject/handleSelectSet（两者会清空会话并写 #chat）。
      if (scope) {
        setSelectedProject(scope.project)
        setSelectedSet(scope.set)
      }
      if (popoutOwner.owns(id)) {
        void chatApi.focusConversationPopout(id)
        occupyConversationInMain(id, conversation ?? currentConversationRef.current)
        return
      }
      void handleSelectConversation(id, {
        messageCount: conversation?.message_count,
        focusMessageId: focusMessageId || undefined,
      })
    }, { restoreCurrentRoute: false })
  }, [handleSelectConversation, occupyConversationInMain, popoutOwner, runAfterLeavingSettings])

  const handleSidebarNewConversation = useCallback(() => {
    runAfterLeavingSettings(() => void handleNewConversation())
  }, [handleNewConversation, runAfterLeavingSettings])

  const handleSidebarConversationDeleted = useCallback(() => {
    forgetRememberedChatRoute()
    applyConversation(null)
    // 对话库/中心页删当前会话时只清会话态，别写 #chat 把中心页冲掉
    const path = hashPath()
    if (getRouteConversationId() !== null || path === 'chat' || path === '') {
      syncConversationRoute(null)
    }
    // 不在这里 refreshSidebar。归档会在 persist 之前就清当前会话；提前 refetch
    // 会把尚未 archived 的条目写回侧栏，下面的行跟着上下抽一截。调用方在写盘
    // 之后自己 loadSidebarData / onConversationsChanged。
  }, [applyConversation, syncConversationRoute])

  const handleSidebarForceDropConversation = useCallback((id: string) => {
    // B3：侧栏删除时强制清掉该会话的 in-flight/快照/乐观项，
    // 使乐观合并不再保留它（删"generating"会话也能立即从侧栏消失）。
    dropConversationLocally(id)
  }, [dropConversationLocally])

  // 侧栏真实列表 refetch 落地 → 剪掉不再生成中的乐观条目。乐观项的生命期是
  // 「发送 → settle（原地换成模型标题，SwapTitle 播打字机）→ 下一次 refetch 接管」：
  // settle 时不能立即剪（refetch 未落地、新会话在真实列表里还没有，剪了行就卸载一帧、
  // 重建后打字机不播）；refetch 落地后真实条目已就位（同 key 无缝接管），或该会话已被
  // 归档/删除（不该再并回）——两种情况都该剪。仍在 generating 的保留（长跑 run 期间
  // 任何无关刷新不得把乐观标题打回「新对话」）。
  const handleSidebarConversationsLoaded = useCallback(() => {
    setOptimisticSidebarConversations((prev) =>
      pruneSettledOptimisticItems(prev, generatingConversationIdsRef.current))
  }, [])

  const openSidebarConversation = useMemo(() => {
    if (!currentConversation) return null
    return optimisticConversationListItem(
      currentConversation,
      conversationLastMessageContent(currentConversation),
    )
  }, [currentConversation])

  const settingsPanelActive = chatView === 'settings' && extensionsNavItem === null

  const handleSidebarOpenExtensionsItem = useCallback((item: ExtensionsNavItem) => {
    // 设置页开着时点扩展项：先走退场（会 flush 自动保存），否则设置页被硬切走、动画不播。
    // 侧栏在设置页下常驻可点（见下方 collapsed 注释）后这条路径才可达。
    runAfterLeavingSettings(() => openExtensionsItem(item))
  }, [openExtensionsItem, runAfterLeavingSettings])

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

  // 侧栏账户菜单：语言切换 / 用量。都是全局行为，所以留在 Chat 这层，
  // 侧栏只负责触发（它拿不到 settings 也不该自己全量保存）。
  const handleSidebarSelectLang = useCallback((next: Lang) => {
    setUiLang(next)
    void (async () => {
      try {
        await updateSettingsCached((settings) => ({ ...settings, settingsLanguage: next }))
      } catch (err) {
        console.error('Failed to save UI language:', err)
      }
    })()
  }, [])

  const handleSidebarOpenUsage = useCallback(() => {
    setExtensionsNavItem(null)
    openEmbeddedSettings('usage')
  }, [openEmbeddedSettings])

  const handleSidebarSearchOpenChange = useCallback((open: boolean) => {
    if (open) {
      runAfterLeavingSettings(() => setSearchOpen(true))
      return
    }
    setSearchOpen(false)
  }, [runAfterLeavingSettings])

  // 中心页（专家/技能/MCP）去掉了整行「返回聊天」顶栏后，窗口顶部不再可拖拽；
  // 且侧栏收起时页面上没有任何展开侧栏/离开中心页的入口（会被困住）。
  // 用一条浮在内容 padding 区上的细拖拽带兜底：始终可拖动窗口，
  // 侧栏收起时在带内浮出「展开侧栏 + 新建聊天」，与会话页收起态的顶栏行为一致。
  // 带高 24px（低于各中心页 pt-7/py-6 的内容起点），不遮挡任何可交互内容；
  // 收起态按钮行复用会话页收起态顶栏的同一套行高/缩进类（52px 行 + mac 交通灯缩进），
  // 保证收起/展开、中心页/会话页之间按钮位置完全不跳。
  //
  // 仅 macOS 需要：Windows / Linux 的 ChatTitlebar 是一条常驻全宽带，
  // 拖拽区与那两枚按钮本就在带里且不随侧栏收展移动，这条兜底带纯属重复。
  const centerPageTopStrip = usesNativeTitlebar ? (
    <div className="absolute inset-x-0 top-0 z-20 h-6" data-tauri-drag-region>
      {sidebarCollapsed && (
        <div
          className={`chat-titlebar-row ${chatTitlebarRowClass} ${chatTitlebarMacInsetClass} chat-titlebar-row--collapsed-mac`}
          data-tauri-drag-region
        >
          <ChatTitlebarActions
            sidebarExpanded={false}
            onToggleSidebar={() => setSidebarCollapsedPersisted(false)}
            onNewConversation={() => void handleNewConversation()}
          />
        </div>
      )}
    </div>
  ) : null

  // 中心页为上方那条收起态按钮行让出的高度（Windows / Linux 无此行，见 centerPageTopStrip）。
  const centerPagePadTop = usesNativeTitlebar && sidebarCollapsed ? 'pt-12' : ''
  // 扩展中心页共用的外壳：与会话主区同款浮起卡片（见 .chat-center-page）。
  // 六个中心页共用 key="center"：React 复用同一个 div，入场动画只在「从会话页进来」时跑一次。
  // 各页各自 key 的话每次互切都是新节点 → 重播 opacity 0→1，中间几帧透出背景，就是那下闪。
  const centerPageClass = `chat-motion-view-in chat-center-page relative flex min-h-0 min-w-0 flex-1 flex-col ${centerPagePadTop}`

  const handleOpenConversationPopout = useCallback(async (conversationId: string) => {
    try {
      const change = await popoutOwner.open(conversationId)
      setExclusiveConversationIds(change.next)
      await navigation.reconcilePopouts(change.previous, change.next)
    } catch (err) {
      const message = typeof err === 'string' ? err : (err as Error).message || i18n[uiLang].chatPopoutLimit
      setPopoutNotice(message)
    }
  }, [navigation, popoutOwner, uiLang])

  const handleDockConversationPopout = useCallback(async (conversationId: string) => {
    try {
      const change = await popoutOwner.close(conversationId)
      setExclusiveConversationIds(change.next)
      await navigation.reconcilePopouts(change.previous, change.next)
    } catch (err) {
      setPopoutNotice(typeof err === 'string' ? err : (err as Error).message || '无法收回独立窗口')
    }
  }, [navigation, popoutOwner])

  // 会话页顶栏控件。非 mac 渲染进全宽标题栏带（单行 chrome），mac 仍留在主区 52px 顶栏。
  const conversationTitlebarControls = useMemo(() => (
    <ConversationTitlebarControls
      activeAgentRuntime={activeAgentRuntime}
      conversationId={currentConversation?.id ?? null}
      runtimeLocked={
        // 一 agent 一对话：有消息后锁死 kind/agent（内置 Kivio 与本地 CLI 一律）。
        // 拉出独立窗口后主窗卸掉了消息，仍按「已有对话」锁死。
        !!currentConversation && (
          (currentConversation.messages?.length ?? 0) > 0
          || popoutConversationIds.has(currentConversation.id)
        )
      }
      usesExternalRuntime={usesExternalRuntime}
      usesChatRuntime={usesChatRuntime}
      activeProviderId={activeProviderId}
      activeModel={activeModel}
      thinkingLevel={
        currentConversation
          ? (currentConversation.thinking_level
              ?? currentConversation.thinkingLevel
              ?? draftThinkingLevel)
          : draftThinkingLevel
      }
      approvalPolicy={approvalPolicy}
      dockOpen={dockOpen}
      uiLang={uiLang}
      onRuntimeChange={handleRuntimeChange}
      onExternalModelChange={handleExternalModelChange}
      onModelChange={handleModelChange}
      onThinkingLevelChange={handleThinkingLevelChange}
      onApprovalPolicyChange={handleApprovalPolicyChange}
      onOpenPopout={(conversationId) => { void handleOpenConversationPopout(conversationId) }}
      onOpenDockTasks={handleOpenDockTasks}
      onToggleDock={handleToggleDock}
    />
  ), [
    activeAgentRuntime,
    activeModel,
    activeProviderId,
    approvalPolicy,
    currentConversation,
    dockOpen,
    draftThinkingLevel,
    handleApprovalPolicyChange,
    handleExternalModelChange,
    handleModelChange,
    handleOpenConversationPopout,
    handleOpenDockTasks,
    handleRuntimeChange,
    handleThinkingLevelChange,
    handleToggleDock,
    popoutConversationIds,
    uiLang,
    usesChatRuntime,
    usesExternalRuntime,
  ])

  const handleTitlebarToggleSidebar = useCallback(() => {
    if (sidebarCollapsed) setSidebarCollapsedPersisted(false)
    else handleCollapseSidebar()
  }, [handleCollapseSidebar, setSidebarCollapsedPersisted, sidebarCollapsed])
  const handleTitlebarNewConversation = useCallback(() => {
    runAfterLeavingSettings(() => void handleNewConversation())
  }, [handleNewConversation, runAfterLeavingSettings])
  const handleDismissHookWarning = useCallback(() => setHookWarning(null), [])
  const handleCloseImageViewer = useCallback(() => setImageViewerItem(null), [])

  const inputBarProps = useMemo<InputBarProps>(() => ({
    onSend: handleSendMessage,
    onQueue: handleQueueMessage,
    disabled: isCurrentConversationBusy(),
    onCancel: handleCancelStream,
    cancelVisible: streamCoarse.streaming,
    cancelling: streamCoarse.cancelling,
    onOpenSettings: handleOpenChatSettings,
    onOpenTools: openSkillCenter,
    onNewChat: handleNewConversation,
    onCompactContext: handleCompressContext,
    onClearChat: handleClearChat,
    enabledTools,
    toolsDisabledReason,
    toolStatusHint,
    sendDisabledReason,
    agentPlanState: currentConversation?.agent_plan_state ?? currentConversation?.agentPlanState ?? null,
    agentTodoState: currentConversation?.agent_todo_state ?? currentConversation?.agentTodoState ?? null,
    onAgentPlanModeChange: handleAgentPlanModeChange,
    usesChatRuntime,
    enabledSkills: usesChatRuntime ? [] : slashSkills,
    onOpenSkillSettings: openSkillCenter,
    selectedProject,
    conversationProject,
    onSelectProject: handleSidebarSelectProject,
    showProjectEntry: true,
    selectedSet,
    onSelectSet: handleSidebarSelectSet,
    currentAssistant: composerCurrentAssistant,
    onOpenAssistantCenter: openAssistantCenter,
    onSelectAssistant: handleSelectAssistant,
    autoFocus: true,
    usesExternalRuntime,
    externalAgentName: activeAgentRuntime.externalAgentId ?? null,
    conversationId: currentConversation?.id ?? null,
    inputHistory: currentConversation?.messages.filter((message) => message.role === 'user').map((message) => message.content),
    knowledgeBaseIds: composerKnowledgeBaseIds,
    onChangeKnowledgeBaseIds: handleChangeKnowledgeBaseIds,
    forceKnowledgeSearch: composerForceKnowledgeSearch,
    onToggleForceKnowledgeSearch: handleToggleForceKnowledgeSearch,
    additionalDirectories: composerAdditionalDirectories,
    onChangeAdditionalDirectories: handleChangeAdditionalDirectories,
    additionalDirectoryPrimaryRoot: selectedProject?.root_path ?? selectedProject?.rootPath ?? null,
    mcpServers,
    onToggleMcpServer: handleToggleMcpServer,
    webSearchMode: activeWebSearchMode,
    onSetWebSearchMode: handleSetWebSearchMode,
    builtinWebSearchSupported: activeBuiltinWebSearchSupported,
    replyModels: activeReplyModels,
    onChangeReplyModels: handleChangeReplyModels,
    contextSlot: composerContextSlot,
    gitWorkdir: usesChatRuntime ? null : dockWorkdir || null,
    gitLang: uiLang,
    onOpenGitPanel: handleOpenDockGit,
    modeOptions: composerModes.options,
    modeValue: composerModes.current,
    onModeChange: handleComposerModeChange,
    presetOptions: composerPresets.options,
    presetValue: composerPresets.current,
    onPresetChange: handleExternalPresetChange,
    presetLocked: Boolean(currentConversation) && !currentConversationIsBlank,
    presetLockedReason: i18n[uiLang].chatAgentPresetLocked,
    usageSlot: composerUsageSlot,
  }), [
    activeAgentRuntime.externalAgentId,
    activeBuiltinWebSearchSupported,
    activeReplyModels,
    activeWebSearchMode,
    composerModes,
    composerPresets,
    composerContextSlot,
    composerCurrentAssistant,
    composerForceKnowledgeSearch,
    composerKnowledgeBaseIds,
    composerAdditionalDirectories,
    composerUsageSlot,
    conversationProject,
    currentConversation,
    currentConversationIsBlank,
    dockWorkdir,
    enabledTools,
    handleAgentPlanModeChange,
    handleCancelStream,
    handleChangeKnowledgeBaseIds,
    handleChangeAdditionalDirectories,
    handleChangeReplyModels,
    handleClearChat,
    handleCompressContext,
    handleComposerModeChange,
    handleExternalPresetChange,
    handleOpenChatSettings,
    handleOpenDockGit,
    handleQueueMessage,
    handleSelectAssistant,
    handleSendMessage,
    handleSetWebSearchMode,
    handleSidebarSelectProject,
    handleSidebarSelectSet,
    handleToggleForceKnowledgeSearch,
    handleToggleMcpServer,
    handleNewConversation,
    isCurrentConversationBusy,
    mcpServers,
    openAssistantCenter,
    openSkillCenter,
    selectedProject,
    selectedSet,
    slashSkills,
    streamCoarse,
    toolsDisabledReason,
    toolStatusHint,
    sendDisabledReason,
    uiLang,
    usesChatRuntime,
    usesExternalRuntime,
  ])

  const messageListProps = useMemo<MessageListProps>(() => ({
    conversationId: currentConversation?.id,
    messages: displayMessages,
    renderRequestId: conversationRenderRequestId,
    onInitialRender: handleConversationFirstCommit,
    agentPlanState: currentConversation?.agent_plan_state ?? currentConversation?.agentPlanState ?? null,
    assistantStreamStatsByMessageId,
    onUpdateMessage: handleUpdateMessage,
    onRegenerateMessage: handleRegenerateMessage,
    onReplyWithModel: (
      usesExternalRuntime || activeAgentPlanMode !== 'act'
        ? undefined
        : handleReplyWithModel
    ),
    sessionProviderId: currentConversation?.provider_id,
    sessionModel: currentConversation?.model,
    onForkMessage: handleForkMessage,
    onRewindMessage: handleRewindMessage,
    onDeleteMessage: handleDeleteMessage,
    onSaveMessageToNote: handleSaveMessageToNote,
    onRetryLastUser: handleRegenerateMessage,
    onExecuteAgentPlan: handleExecuteAgentPlan,
    groupSelections: currentConversation?.group_selections ?? currentConversation?.groupSelections ?? {},
    onSetGroupSelection: handleSetGroupSelection,
    contextState,
    compactionInProgress: contextCompressing,
    animateCompactionBoundaryId: animateCompactionBoundaryId,
    animateClearBoundaryId: animateClearBoundaryId,
    lang: uiLang,
    focusMessageId,
    onFocusMessageHandled: () => setFocusMessageId(null),
  }), [
    animateCompactionBoundaryId,
    animateClearBoundaryId,
    assistantStreamStatsByMessageId,
    contextCompressing,
    contextState,
    currentConversation,
    conversationRenderRequestId,
    displayMessages,
    handleConversationFirstCommit,
    handleDeleteMessage,
    handleExecuteAgentPlan,
    handleForkMessage,
    handleRegenerateMessage,
    handleReplyWithModel,
    handleRewindMessage,
    handleSaveMessageToNote,
    handleSetGroupSelection,
    handleUpdateMessage,
    uiLang,
    focusMessageId,
    usesExternalRuntime,
    activeAgentPlanMode,
  ])

  const forkOrigin = useMemo(() => {
    const origin = currentConversation?.forked_from ?? currentConversation?.forkedFrom
    if (!origin) return null
    const sourceId = origin.conversation_id ?? origin.conversationId
    return sourceId ? { sourceId, title: origin.title } : null
  }, [currentConversation])

  const pendingSlot = useMemo(() => (
    <PendingInteractionSlot
      snapshot={interactionSnapshot}
      activeAgentRuntime={activeAgentRuntime}
      onResolveToolConfirm={resolvePendingToolConfirm}
      onResolveSessionConsent={resolvePendingSessionConsent}
      onDismissUserPrompt={dismissPendingUserPrompt}
      onPersistApprovedSandbox={persistApprovedExternalSandbox}
    />
  ), [
    activeAgentRuntime, dismissPendingUserPrompt, interactionSnapshot, persistApprovedExternalSandbox,
    resolvePendingSessionConsent, resolvePendingToolConfirm,
  ])

  return (
    <LangContext.Provider value={uiLang}>
    <AsyncQuestionsContext.Provider value={asyncQuestionsValue}>
    <Profiler id="ChatShell" onRender={onChatPerfProfiler}>
      <div
        className={`chat-window-shell${usesNativeTitlebar ? ' chat-window-shell--native-titlebar' : ''}`}
      >
      {!usesNativeTitlebar && (
        <ChatTitlebar
          sidebarExpanded={!sidebarCollapsed}
          /* 与下方 <Sidebar collapsed> 取反同源：设置页里侧栏也是收起的，
             只看 sidebarCollapsed 会在设置页多留一侧栏宽的空档。 */
          sidebarVisible={!(sidebarCollapsed || settingsPanelActive)}
          settingsMode={settingsPanelActive}
          onToggleSidebar={handleTitlebarToggleSidebar}
          onNewConversation={handleTitlebarNewConversation}
        >
          {chatView === 'conversation' ? conversationTitlebarControls : null}
        </ChatTitlebar>
      )}
      <div className="flex min-h-0 w-full flex-1">
        {chatView !== 'onboarding' ? (
        /* 设置页自带 200px 导航栏，聊天侧栏此时借用已有的折叠过渡整体滑出（不再直接卸载，
           否则左列会先空一帧、且关闭时侧栏是瞬间 pop 回来的）。退场期保持折叠，
           等视图真正切回会话后再滑入，与会话页入场同时发生 —— 否则侧栏会在设置页
           淡出的同时把它挤窄。 */
        <ChatSidebarPane
          onRender={onChatPerfProfiler}
          lang={uiLang}
          currentConversationId={currentConversation?.id}
          generatingConversationIds={generatingConversationIds}
          optimisticConversations={optimisticSidebarConversations}
          openConversation={openSidebarConversation}
          selectedProject={selectedProject}
          onSelectProject={handleSidebarSelectProject}
          selectedSet={selectedSet}
          onSelectSet={handleSidebarSelectSet}
          onSelectConversation={handleSidebarSelectConversation}
          onNewConversation={handleSidebarNewConversation}
          onOpenInPopout={handleOpenConversationPopout}
          onConversationDeleted={handleSidebarConversationDeleted}
          onForceDropConversation={handleSidebarForceDropConversation}
          onConversationsLoaded={handleSidebarConversationsLoaded}
          onOpenExtensionsItem={handleSidebarOpenExtensionsItem}
          onOpenSettings={handleSidebarOpenSettings}
          onSelectLang={handleSidebarSelectLang}
          onOpenUsage={handleSidebarOpenUsage}
          settingsActive={settingsPanelActive}
          extensionsActive={extensionsActive}
          collapsed={sidebarCollapsed || settingsPanelActive}
          onToggleCollapsed={handleCollapseSidebar}
          width={sidebarWidth}
          onWidthChange={handleSidebarWidthChange}
          refreshKey={sidebarRefreshKey}
          profileRefreshKey={sidebarProfileRefreshKey}
          searchOpen={searchOpen}
          onSearchOpenChange={handleSidebarSearchOpenChange}
        />
        ) : null}

        <ChatRouteKeepAlive
          activeKey={chatView === 'conversation' || chatView === 'settings' ? chatView : 'center'}
        >
        {chatView === 'onboarding' ? (
          <div className="flex min-h-0 min-w-0 flex-1 flex-col">
            <OnboardingShell
              onComplete={handleOnboardingExit}
              onSkip={handleOnboardingExit}
              onSettingsChange={onSettingsChange}
            />
          </div>
        ) : chatView === 'settings' ? (
          <ChatSettingsPane
            settingsRef={settingsRef}
            exiting={settingsExiting}
            className={`flex min-h-0 min-w-0 flex-1 flex-col${
              !usesNativeTitlebar && settingsPanelActive ? ' settings-embedded-under-strip' : ''
            }`}
            initialTab={settingsInitialTab}
            reserveTrafficLightSpace={(sidebarCollapsed || extensionsNavItem === null) && usesNativeTitlebar}
            onClose={handleSettingsClose}
            onSettingsChange={handleSettingsChange}
            onReady={emitContentReady}
            renderSessionCenter={(lang) => (
              <Suspense fallback={null}>
                <SessionCenter
                  lang={lang}
                  embedded
                  currentConversationId={currentConversation?.id}
                  generatingConversationIds={generatingConversationIds}
                  onSelectConversation={handleSidebarSelectConversation}
                  onConversationDeleted={handleSidebarConversationDeleted}
                  onForceDropConversation={handleSidebarForceDropConversation}
                  onConversationsChanged={refreshSidebar}
                />
              </Suspense>
            )}
            onRender={onChatPerfProfiler}
          />
        ) : chatView === 'assistants' ? (
          <div key="center" className={centerPageClass}>
            {centerPageTopStrip}
            <Suspense fallback={null}>
              <AssistantCenter
                skills={enabledSkills}
                currentAssistantId={currentAssistantId}
                onStartAssistantChat={(assistant) => void handleStartAssistantChat(assistant)}
                onStartBuilder={() => void handleStartBuilderChat()}
                onApplyAssistant={currentConversation ? (assistantId) => void handleApplyAssistant(assistantId) : undefined}
              />
            </Suspense>
          </div>
        ) : chatView === 'skill' ? (
          <div key="center" className={centerPageClass}>
            {centerPageTopStrip}
            <Suspense fallback={null}>
              <SkillCenter
                onSkillsChanged={() => void loadSkills()}
                projectCwd={dockWorkdir || undefined}
              />
            </Suspense>
          </div>
        ) : chatView === 'mcp' ? (
          <div key="center" className={centerPageClass}>
            {centerPageTopStrip}
            <Suspense fallback={null}>
              <McpCenter />
            </Suspense>
          </div>
        ) : chatView === 'knowledge' ? (
          <div key="center" className={centerPageClass}>
            {centerPageTopStrip}
            <Suspense fallback={null}>
              <KnowledgeCenter />
            </Suspense>
          </div>
        ) : chatView === 'artifacts' ? (
          <div key="center" className={centerPageClass}>
            {centerPageTopStrip}
            <Suspense fallback={null}>
              <ArtifactsCenter onOpenConversation={handleSidebarSelectConversation} />
            </Suspense>
          </div>
        ) : chatView === 'notes' ? (
          <div key="center" className={centerPageClass}>
            {centerPageTopStrip}
            <Suspense fallback={null}>
              <NotesCenter />
            </Suspense>
          </div>
        ) : chatView === 'automations' ? (
          <div key="center" className={centerPageClass}>
            {centerPageTopStrip}
            <Suspense fallback={null}>
              <AutomationCenter />
            </Suspense>
          </div>
        ) : conversationOccupied ? (
          <PopoutOccupiedPlaceholder
            lang={uiLang}
            onFocus={() => {
              if (currentConversation?.id) void chatApi.focusConversationPopout(currentConversation.id)
            }}
            onDock={() => {
              if (currentConversation?.id) void handleDockConversationPopout(currentConversation.id)
            }}
            sidebarCollapsed={sidebarCollapsed}
            titlebarControls={conversationTitlebarControls}
            onToggleSidebar={handleTitlebarToggleSidebar}
            onNewConversation={handleTitlebarNewConversation}
          />
        ) : (
          <ChatConversationPane
            titlebarControls={conversationTitlebarControls}
            usesNativeTitlebar={usesNativeTitlebar}
            sidebarCollapsed={sidebarCollapsed}
            titlebarRowClass={chatTitlebarRowClass}
            titlebarMacInsetClass={chatTitlebarMacInsetClass}
            onToggleSidebar={handleTitlebarToggleSidebar}
            onNewConversation={handleTitlebarNewConversation}
            protocolVersionMismatch={protocolVersionMismatch}
            showEmptyHero={showEmptyHero}
            currentAssistantName={currentAssistantSnapshot?.name ?? null}
            selectedProjectName={selectedProject?.name ?? null}
            selectedSetName={selectedSet?.name ?? null}
            inputBarProps={inputBarProps}
            messageListProps={messageListProps}
            hookWarning={hookWarning}
            currentConversationId={currentConversation?.id ?? null}
            onDismissHookWarning={handleDismissHookWarning}
            forkOrigin={forkOrigin}
            onSelectConversation={handleSelectConversation}
            importedHistoryStale={importedHistoryStale}
            pendingSlot={pendingSlot}
            subAgentSlot={currentConversation?.id && <SubAgentIndicator key={currentConversation.id} conversationId={currentConversation.id} lang={uiLang} onOpen={handleOpenDockTasks} />}
            goalSlot={visibleGoal ? (
              <GoalCard
                goal={visibleGoal}
                onEdit={handleEditGoal}
                onPause={handlePauseGoal}
                onResume={handleResumeGoal}
                onCancel={handleCancelGoal}
              />
            ) : null}
            queuedMessages={currentQueuedMessages}
            canSteerQueuedMessages={canSteerCurrentConversation}
            onSteerQueuedMessage={handleSteerQueuedMessage}
            onRemoveQueuedMessage={handleRemoveQueuedMessage}
            onRestoreQueuedMessage={handleRestoreQueuedMessage}
            lang={uiLang}
            imageViewerItem={imageViewerItem}
            onCloseImageViewer={handleCloseImageViewer}
            onRender={onChatPerfProfiler}
          />
        )}
        </ChatRouteKeepAlive>
        {chatView === 'conversation' && !usesChatRuntime && !conversationOccupied && (
          <RightDock
            subAgentRequest={dock.subAgentRequest}
            open={dockOpen}
            width={dock.width}
            activeTab={dock.tab}
            workdir={dockWorkdir}
            lang={uiLang}
            conversationId={currentConversation?.id ?? null}
            treeExpanded={dock.treeExpanded}
            revealRequest={dock.reveal}
            previewRequest={dock.preview}
            onToggleTab={dock.setTab}
            onWidthChange={dock.setWidth}
            onClose={dock.close}
            onTreeExpandedChange={dock.setTreeExpanded}
            onInsertMention={dock.insertMention}
            onRevealInTree={dock.revealInTree}
          />
        )}
      </div>
      {popoutNotice && (
        <div className="pointer-events-none absolute bottom-5 left-1/2 z-50 -translate-x-1/2 rounded-lg bg-neutral-900/90 px-3 py-1.5 text-[12px] text-white shadow-lg dark:bg-neutral-100/90 dark:text-neutral-900">
          {popoutNotice}
        </div>
      )}
      </div>
    </Profiler>
    </AsyncQuestionsContext.Provider>
    </LangContext.Provider>
  )
}
