import { refreshSubAgents } from './useSubAgents'
import { SubAgentIndicator } from './SubAgentPanel'
import { lazy, memo, Profiler, startTransition, Suspense, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore, type ProfilerOnRenderCallback, type ReactNode, type Ref } from 'react'
import { PanelRight, SquareArrowOutUpRight } from 'lucide-react'
import { type ConversationSelectionScope, type ExtensionsNavItem } from './Sidebar'
import { ChatSidebarPane } from './ChatSidebarPane'
import { completeSettingsExit, type PendingSettingsAction } from './settingsExit'
import { useChatRouting } from './hooks/useChatRouting'
import { createChatNavigationController } from './chatNavigationController'
import { createChatExecutionOwner, type ExecutionLease } from './chatExecutionOwner'
import { createChatSendController, type SendPresentationEvent } from './chatSendController'
import { createStreamPreviewOwner } from './streamPreviewOwner'
import { useExternalSendQueue } from './hooks/useExternalSendQueue'
import { useMessageQueue } from './hooks/useMessageQueue'
import type { QueuedMessage } from './hooks/useMessageQueue'
import { useComposerDraft } from './hooks/useComposerDraft'
import { useTauriEvent } from './hooks/useTauriEvent'
import {
  clearConversationLocalState,
  type ConversationLocalState,
} from './conversationLocalState'
import {
  getRouteConversationId,
  hashPath,
  isChatAssistantCenterPath,
  isChatKnowledgeCenterPath,
  isChatMcpCenterPath,
  isChatNotesPath,
  isChatOnboardingRoute,
  isChatPluginCenterPath,
  isChatSessionCenterPath,
  isChatAutomationsPath,
  isChatSettingsPath,
  isChatSkillCenterPath,
  conversationHash,
  setHash,
} from './chatRoutes'
import { ApprovalCard } from './ApprovalCard'
import { AskUserBlock } from './AskUserBlock'
import { AsyncQuestionsContext } from './asyncQuestionsContext'
import { ChatTitlebar } from './ChatTitlebar'
import { withExternalModel } from './externalModelEffort'
import { findUnavailableRecommendedTools } from './toolAvailability'
import { ChatTitlebarActions } from './ChatTitlebarActions'
import {
  completeConversationTransition,
  invalidateConversationTransition,
} from './conversationTransitionStore'
import type { AssistantStreamStats, MessageListProps } from './MessageList'
import type { InputBarProps } from './InputBar'
import { SessionUsageStrip } from './SessionUsageStrip'
import { ModelSelector } from './ModelSelector'
import { ThinkingLevelSelector } from './ThinkingLevelSelector'
import { ExternalModelSelector, RuntimePicker } from './RuntimePicker'
import { PermissionPicker } from './PermissionPicker'
import { deriveDshPresetModes, derivePermissionModes, useDetectedExternalAgents, useDshCustomPresets } from './permissionModes'
import { BackgroundJobsIndicator } from './BackgroundJobsIndicator'
import { ContextIndicator } from './ContextIndicator'
import {
  agentRuntimesEqual,
  BUILTIN_AGENT_RUNTIME,
  chatApi,
  normalizeAgentRuntime,
  type AgentRuntimeConfig,
} from './api'
import { loadLastAgentRuntime, saveLastAgentRuntime } from './lastAgentRuntime'
import { loadLastModel, resolvePreferredChatModel, saveLastModel } from './lastModel'
import {
  chatTitlebarMacInsetClass,
  chatTitlebarRowClass,
  usesNativeTitlebar,
} from './platform'
import type {
  ChatProject,
  ChatSet,
  ChatAssistant,
  Conversation,
  ConversationListItem,
  ConversationSearchHit,
  ConversationContextState,
  AgentPlanMode,
  AgentPlanState,
  AgentTodoState,
  GoalState,
  PendingAttachment,
  SkillMeta,
  ThinkingLevel,
  ModelRef,
  WebSearchMode,
  AdditionalDirectory,
} from './types'
import {
  api,
  builtinWebSearchSupported,
  resolveProviderWebSearchMode,
  type ChatSessionConsentPayload,
  type ChatHookPayload,
  type ChatToolConfirmPayload,
  type ChatToolDefinition,
  type ChatMcpServer,
  type ChatUserPromptPayload,
} from '../api/tauri'
import { getSettingsCached, refreshSettings, subscribeSettings, updateSettingsCached } from '../api/settingsCache'
import { setExclusiveConversationIds } from '../api/chatProtocol'
import { isPluginManagedServer, preservePluginManagedServers } from '../settings/public/connectors'
import { OnboardingShell } from '../onboarding/public/shell'
import type { SettingsShellHandle, SettingsShellProps, SettingsTab } from '../settings/public/shell'
import { i18n, LangContext, type Lang } from '../settings/public/i18n'
import { estimateTokens } from '../utils/tokens'
import {
  CHAT_MIN_SIZE_COLLAPSED,
  forgetRememberedChatRoute,
  getRememberedChatSidebarCollapsed,
  getRememberedDockOpen,
  getRememberedDockTab,
  getRememberedDockWidth,
  getRememberedSidebarWidth,
  getRememberedTreeExpanded,
  rememberChatSidebarCollapsed,
  rememberChatSize,
  rememberDockOpen,
  rememberDockTab,
  rememberDockWidth,
  rememberSidebarWidth,
  rememberTreeExpanded,
} from './persistence'
import { RightDock, type DockPreviewRequest, type DockRevealRequest, type DockTab } from './dock/RightDock'
import { dockApi } from './dock/api'
import { insertTextIntoComposer } from './composerInsert'
import { onDockSubAgentRequest, onDockDiffPreviewRequest, onDockMarkdownPreviewRequest, onDockPreviewRequest, requestDockMarkdownPreview } from './dock/dockPreview'
import { IconButton } from '../components/Button'
import { isTauriRuntime } from './utils'
import { hasEnabledNativeBuiltinTool, hasEnabledSkillRuntime } from '../utils/chatTools'
import { onChatImageViewerOpen, type ChatImageViewerItem } from './imageViewer'
import { isPlaceholderTitle, optimisticConversationTitle } from './conversationTitle'
import {
  getCoarse as getStreamCoarse,
  setCoarse as setStreamCoarse,
  useStreamCoarse,
} from './streamingStore'
import {
  endGroup,
  getActiveGroup,
  resetGroups,
} from './groupStreamingStore'
import { assistantTurnSpan } from './messageGroups'
import { latestCompactionBoundaryId, mergeCompactionContextState } from './compactionBoundary'
import { latestClearBoundaryId, mergeClearContextState } from './contextClearBoundary'
import { applyLiveContextUsage } from './contextPanel'
import { measureChatSurface, onChatPerfProfiler, useChatPerfLongTaskProbe, useChatPerfRenderProbe } from './chatPerformanceProbe'
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
  streamTerminalReason,
  userPromptEventToRecord,
} from './streamApply'
import {
  isEnterPlanApproval,
  isPlanApproval,
  PLAN_APPROVAL_ACTIONS,
  toolApprovalTitle,
} from './toolApproval'

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

const AutomationCenter = lazy(() => import('./automation/AutomationCenter').then((module) => ({
  default: module.AutomationCenter,
})))

type ChatView = 'conversation' | 'settings' | 'assistants' | 'skill' | 'mcp' | 'knowledge' | 'notes' | 'automations' | 'onboarding'

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
  sessionLibrary,
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
  sessionLibrary: NonNullable<SettingsShellProps['sessionLibrary']>
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
            sessionLibrary={sessionLibrary}
          />
        </Profiler>
      </SettingsEnterPane>
    </Suspense>
  )
})

/**
 * 记住用户在顶栏最后一次选的思考等级，作为新会话/空会话草稿（以用户的选择为准，
 * 也不再把思考等级硬回落到 high）。仅前端偏好，存 localStorage。聊天模型见 lastModel.ts。
 */
const LAST_THINKING_KEY = 'kivio.chat.lastThinkingLevel'

const VALID_THINKING_LEVELS: ReadonlySet<string> = new Set([
  'off',
  'low',
  'medium',
  'high',
  'xhigh',
  'max',
])

// 网络搜索模式全局默认（任务 07-23，与思考等级同款「记住上次选择」模式）：
// 选一次即成为新会话/未显式设置会话的默认，免去每个对话重复切换。
const LAST_WEB_SEARCH_MODE_KEY = 'kivio.chat.lastWebSearchMode'
const VALID_WEB_SEARCH_MODES: ReadonlySet<string> = new Set(['off', 'builtin', 'third_party'])

function loadLastWebSearchMode(): WebSearchMode | undefined {
  try {
    const raw = window.localStorage.getItem(LAST_WEB_SEARCH_MODE_KEY)
    return raw && VALID_WEB_SEARCH_MODES.has(raw) ? (raw as WebSearchMode) : undefined
  } catch {
    return undefined
  }
}

function saveLastWebSearchMode(mode: WebSearchMode): void {
  try {
    window.localStorage.setItem(LAST_WEB_SEARCH_MODE_KEY, mode)
  } catch {
    /* ignore */
  }
}

function loadLastThinkingLevel(): ThinkingLevel | null {
  try {
    const raw = window.localStorage.getItem(LAST_THINKING_KEY)
    return raw && VALID_THINKING_LEVELS.has(raw) ? (raw as ThinkingLevel) : null
  } catch {
    return null
  }
}

function saveLastThinkingLevel(level: ThinkingLevel | null): void {
  try {
    if (level) window.localStorage.setItem(LAST_THINKING_KEY, level)
    else window.localStorage.removeItem(LAST_THINKING_KEY)
  } catch {
    /* ignore */
  }
}

/** 把聊天里刚选的模型同步进 settings，供 Mixer / 后端回落，不当作引导里的「默认模型」。 */
async function persistLastChatModelToSettings(providerId: string, model: string): Promise<void> {
  if (!providerId.trim()) return
  try {
    await updateSettingsCached((settings) => {
      const current = settings.defaultModels?.chat
      if (current?.providerId === providerId && current?.model === model) return settings
      return {
        ...settings,
        defaultModels: {
          ...settings.defaultModels,
          chat: { providerId, model },
        },
        chatProviderId: providerId,
        chatModel: model,
      }
    })
  } catch (err) {
    console.error('Failed to persist last chat model:', err)
  }
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

// 设置当前视图的流式错误（写 streamingStore 的 coarse 片）。模块级函数，调用点无需进
// useCallback 依赖。注意：与 setStreamErrorForConversation 不同，这里只改当前视图、不写
// streamErrorsRef（保持原 setStreamError(useState) 的语义）。
function setStreamError(error: string): void {
  setStreamCoarse({ streamError: error })
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

function additionalDirectoriesOf(conversation: Conversation | null | undefined): AdditionalDirectory[] {
  return conversation?.additional_directories ?? conversation?.additionalDirectories ?? []
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

function isPlainBlankConversation(conversation: Conversation | null): boolean {
  return Boolean(
    conversation
    && conversation.messages.length === 0
    && !(conversation.assistant_id ?? conversation.assistantId),
  )
}

function optimisticConversationListItem(
  conversation: Conversation,
  content: string,
  attachmentNames: readonly string[] = [],
): ConversationListItem {
  const preview = content.replace(/\s+/g, ' ').trim()
  const title = isPlaceholderTitle(conversation.title)
    ? optimisticConversationTitle(content, attachmentNames)
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

/** 取会话最后一条 user/assistant 消息文本（侧栏 preview 口径，与 api.ts toListItem 一致）。 */
function conversationLastMessageContent(conversation: Conversation): string {
  for (let i = conversation.messages.length - 1; i >= 0; i--) {
    const message = conversation.messages[i]
    if (message.role === 'user' || message.role === 'assistant') {
      return message.content?.trim() ?? ''
    }
  }
  return ''
}

/** 用持久化后的真实会话替换侧栏乐观条目（同 id 原地替换，行实例不销毁，
 *  SwapTitle 才能感知标题从「截断第一句」变成「模型标题」并播放替换过渡）。
 *  keptConversation 为空（发送彻底失败）时退回移除条目。 */
function settleOptimisticConversationListItem(
  setOptimistic: (updater: (items: ConversationListItem[]) => ConversationListItem[]) => void,
  conversationId: string,
  keptConversation: Conversation | null,
): void {
  setOptimistic((items) =>
    keptConversation
      ? items.map((item) =>
          item.id === conversationId
            ? optimisticConversationListItem(
                keptConversation,
                keptConversation.messages.find((message) => message.role === 'user')?.content
                  ?? conversationLastMessageContent(keptConversation),
                keptConversation.messages.find((message) => message.role === 'user')
                  ?.attachments?.map((attachment) => attachment.name) ?? [],
              )
            : item,
        )
      : items.filter((item) => item.id !== conversationId),
  )
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
  const [sidebarCollapsed, setSidebarCollapsed] = useState(() => getRememberedChatSidebarCollapsed())
  const [sidebarWidth, setSidebarWidth] = useState(() => getRememberedSidebarWidth())
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
  const poppedGeneratingRunsRef = useRef<Map<string, Set<string>>>(new Map())
  const [popoutConversationIds, setPopoutConversationIds] = useState<ReadonlySet<string>>(() => new Set())
  const [popoutNotice, setPopoutNotice] = useState<string | null>(null)
  const popoutConversationIdsRef = useRef<ReadonlySet<string>>(new Set())
  popoutConversationIdsRef.current = popoutConversationIds
  const popoutsListedRef = useRef(false)
  const replacePopoutIds = useCallback((ids: Iterable<string>) => {
    const next = ids instanceof Set ? ids as Set<string> : new Set(ids)
    popoutConversationIdsRef.current = next
    popoutsListedRef.current = true
    setExclusiveConversationIds(next)
    setPopoutConversationIds(next)
  }, [])
  const addPopoutId = useCallback((conversationId: string) => {
    if (popoutConversationIdsRef.current.has(conversationId)) return
    const next = new Set(popoutConversationIdsRef.current)
    next.add(conversationId)
    popoutConversationIdsRef.current = next
    setExclusiveConversationIds(next)
    setPopoutConversationIds(next)
  }, [])
  const ensurePopoutIds = useCallback(async () => {
    if (popoutsListedRef.current) return popoutConversationIdsRef.current
    const ids = await chatApi.listConversationPopouts()
    const next = new Set(ids)
    popoutConversationIdsRef.current = next
    popoutsListedRef.current = true
    setExclusiveConversationIds(next)
    setPopoutConversationIds(next)
    return next
  }, [])
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
    setProviderModel: setDraftProviderModel,
    setKnowledgeBaseIds: setDraftKnowledgeBaseIds,
    setForceKnowledgeSearch: setDraftForceKnowledgeSearch,
    setAdditionalDirectories: setDraftAdditionalDirectories,
    setThinkingLevel: setDraftThinkingLevel,
    setWebSearchMode: setDraftWebSearchMode,
    setReplyModels: setDraftReplyModels,
    setAgentRuntime: setDraftAgentRuntime,
    resetConversationContext: resetComposerDraftContext,
  } = useComposerDraft({
    providerId: '',
    model: '',
    thinkingLevel: loadLastThinkingLevel(),
    agentRuntime: loadLastAgentRuntime() ?? BUILTIN_AGENT_RUNTIME,
  })
  const [skills, setSkills] = useState<SkillMeta[]>([])
  const [disabledSkillIds, setDisabledSkillIds] = useState<string[]>([])
  const [settingsInitialTab, setSettingsInitialTab] = useState<SettingsTab>(() => {
    const path = hashPath()
    if (isChatPluginCenterPath(path)) return 'plugins'
    if (isChatSessionCenterPath(path)) return 'sessions'
    return 'chat'
  })
  const [uiLang, setUiLang] = useState<Lang>('zh')
  const [extensionsNavItem, setExtensionsNavItem] = useState<ExtensionsNavItem | null>(null)
  const [enabledTools, setEnabledTools] = useState<ChatToolDefinition[]>([])
  const [mcpServers, setMcpServers] = useState<ChatMcpServer[]>([])
  const [webSearchEnabled, setWebSearchEnabled] = useState(true)
  // provider id → apiFormat（任务 07-23）：用于判断当前模型是否支持内置搜索。
  const [providerApiFormats, setProviderApiFormats] = useState<Record<string, string>>({})
  const [providerOAuthTypes, setProviderOAuthTypes] = useState<Record<string, string>>({})
  const [providerBaseUrls, setProviderBaseUrls] = useState<Record<string, string>>({})
  const [enabledToolCount, setEnabledToolCount] = useState<number | null>(null)
  const [toolDiscoveryPending, setToolDiscoveryPending] = useState(true)
  const [toolsDisabledReason, setToolsDisabledReason] = useState('')
  const [toolsRequested, setToolsRequested] = useState(false)
  const [approvalPolicy, setApprovalPolicy] = useState('readonly_auto_sensitive_confirm')
  const [pendingToolConfirm, setPendingToolConfirm] = useState<ChatToolConfirmPayload | null>(null)
  const [toolConfirmSubmittingId, setToolConfirmSubmittingId] = useState<string | null>(null)
  const [toolConfirmError, setToolConfirmError] = useState('')
  /** 待答的问用户询问：整张可作答的面板吊在**输入框上方**（与审批卡同一个槽位），
   *  消息流里只留一行痕迹。生成这一刻是停在这里等人的，把它放在视线和手都在的地方。 */
  const [pendingUserPrompt, setPendingUserPrompt] = useState<ChatUserPromptPayload | null>(null)
  const [pendingSessionConsent, setPendingSessionConsent] = useState<ChatSessionConsentPayload | null>(null)
  const [sessionConsentSubmittingConversationId, setSessionConsentSubmittingConversationId] = useState<string | null>(null)
  const [sessionConsentError, setSessionConsentError] = useState('')
  const [contextState, setContextState] = useState<ConversationContextState | null>(null)
  const [contextLoading, setContextLoading] = useState(false)
  // 压缩状态必须按会话记，不能用全局 boolean：压缩中切会话会把「压缩中」动画留在
  // 另一个会话上，而压缩事件按 conversationId 派发，收敛条件不能再是「是不是当前会话」
  // （那样后台会话的 completed 会被丢掉，标志永远清不掉）。手动与自动压缩共用这一个集合。
  const [compactingConversationIds, setCompactingConversationIds] = useState<ReadonlySet<string>>(
    () => new Set(),
  )
  const markConversationCompacting = useCallback((conversationId: string, compacting: boolean) => {
    setCompactingConversationIds((previous) => {
      if (previous.has(conversationId) === compacting) return previous
      const next = new Set(previous)
      if (compacting) next.add(conversationId)
      else next.delete(conversationId)
      return next
    })
  }, [])
  const contextCompressing = currentConversation
    ? compactingConversationIds.has(currentConversation.id)
    : false
  const [animateCompactionBoundaryId, setAnimateCompactionBoundaryId] = useState<string | null>(null)
  const [animateClearBoundaryId, setAnimateClearBoundaryId] = useState<string | null>(null)
  const [contextError, setContextError] = useState('')
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
  const restoredRunIdsRef = useRef<Set<string>>(new Set())
  const streamErrorsRef = useRef<Record<string, string>>({})
  const pendingToolConfirmsRef = useRef<Record<string, ChatToolConfirmPayload[]>>({})
  const toolConfirmSubmissionsRef = useRef<Set<string>>(new Set())
  /** 按会话排队（同审批卡）：切会话回来还在等的那条要还在。 */
  const pendingUserPromptsRef = useRef<Record<string, ChatUserPromptPayload[]>>({})
  const pendingSessionConsentsRef = useRef<Record<string, ChatSessionConsentPayload>>({})
  const sessionConsentSubmissionsRef = useRef<Set<string>>(new Set())
  const settingsRef = useRef<SettingsShellHandle>(null)
  const pendingAfterSettingsCloseRef = useRef<PendingSettingsAction | null>(null)
  // A 合帧（render coalescing）：高频 stream/tool/subagent/userprompt 事件不再每条都同步
  // setState 重渲，而是把"待显示的快照"记到 ref，用 requestAnimationFrame 每帧最多 flush 一次。

  useEffect(() => onChatImageViewerOpen(setImageViewerItem), [])

  // 待交互卡片与错误仍按会话保存；流预览由 previewOwner 独占。
  const localState = useCallback((): ConversationLocalState => ({
    streamErrors: streamErrorsRef.current,
    pendingToolConfirms: pendingToolConfirmsRef.current,
    pendingSessionConsents: pendingSessionConsentsRef.current,
    pendingUserPrompts: pendingUserPromptsRef.current,
  }), [])

  const generatingConversationIdsRef = useRef<Set<string>>(new Set())
  const syncGeneratingConversationIds = useCallback(() => {
    const next = new Set([...executionOwner.activeConversationIds(), ...previewOwner.streamingConversationIds()])
    for (const [conversationId, queue] of Object.entries(pendingToolConfirmsRef.current)) {
      if (queue.length > 0) next.add(conversationId)
    }
    const previous = generatingConversationIdsRef.current
    if (previous.size === next.size && [...previous].every((id) => next.has(id))) return
    generatingConversationIdsRef.current = next
    setGeneratingConversationIds(next)
  }, [executionOwner, previewOwner])

  const markConversationInFlight = useCallback((conversationId: string) => {
    executionOwner.observe({ kind: 'externalStarted', conversationId })
    syncGeneratingConversationIds()
  }, [executionOwner, syncGeneratingConversationIds])

  const clearConversationInFlight = useCallback((conversationId: string) => {
    executionOwner.observe({ kind: 'externalEnded', conversationId })
    syncGeneratingConversationIds()
  }, [executionOwner, syncGeneratingConversationIds])

  // B：彻底把一个会话从所有本地乐观/in-flight/快照状态中剔除（ghost 清理）。
  // 不触碰 currentConversation/route，由调用方按场景决定。
  const dropConversationLocally = useCallback((conversationId: string) => {
    clearConversationLocalState(localState(), conversationId, { streamErrors: true })
    previewOwner.drop(conversationId)
    executionOwner.observe({ kind: 'drop', conversationId })
    // 排队消息也一起剔除：会话没了，队列里那几条再没有能落到的地方（`drain` 也拿不到会话对象）。
    // 经 ref 调用是因为队列 hook 声明在下方（它要转发 handleSendMessage）。
    messageQueueRef.current.clearConversation(conversationId)
    setOptimisticSidebarConversations((items) => items.filter((item) => item.id !== conversationId))
    syncGeneratingConversationIds()
  }, [executionOwner, localState, previewOwner, syncGeneratingConversationIds])

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
    setCurrentConversation(conversation)
    setContextState(conversation?.context_state ?? conversation?.contextState ?? null)
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

  const patchContextState = useCallback((nextState: ConversationContextState) => {
    setContextState((prev) => {
      const merged = mergeClearContextState(prev, mergeCompactionContextState(prev, nextState))
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
    if (!conversationId) {
      setPendingToolConfirm(null)
      setPendingSessionConsent(null)
      setPendingUserPrompt(null)
      setToolConfirmError('')
      setSessionConsentError('')
      setStreamCoarse({ streamError: '' })
      return
    }
    setStreamCoarse({ streamError: streamErrorsRef.current[conversationId] ?? '' })
    setPendingToolConfirm(pendingToolConfirmsRef.current[conversationId]?.[0] ?? null)
    setPendingSessionConsent(pendingSessionConsentsRef.current[conversationId] ?? null)
    setPendingUserPrompt(pendingUserPromptsRef.current[conversationId]?.[0] ?? null)
    setToolConfirmError('')
    setSessionConsentError('')
  }, [previewOwner])

  useEffect(() => () => {
    previewOwner.dispose()
    resetGroups()
  }, [previewOwner])

  const clearStreamSnapshot = useCallback((conversationId: string | null) => {
    if (!conversationId) return
    clearConversationLocalState(localState(), conversationId)
    previewOwner.drop(conversationId)
    syncGeneratingConversationIds()
    if (currentConversationIdRef.current === conversationId) {
      setPendingToolConfirm(null)
      setPendingSessionConsent(null)
      setPendingUserPrompt(null)
    }
  }, [localState, previewOwner, syncGeneratingConversationIds])

  const freezeCancelledRunLocally = useCallback((conversationId: string) => {
    // 立即停掉"生成中"视觉（撤掉取消按钮 + 停 shimmer），但保留已生成文本：
    // 切到 frozen 态冻结展示，等 send invoke 返回持久化消息时由
    // finishStreamingRunWithConversation 无缝替换冻结的预览。
    // 过滤迟到的内容事件，但仍接收终局事件，供没有 send invoke 的恢复运行收尾。
    previewOwner.freezeForCancellation(conversationId)
    delete pendingToolConfirmsRef.current[conversationId]
    delete pendingSessionConsentsRef.current[conversationId]
    delete pendingUserPromptsRef.current[conversationId]
    setPendingToolConfirm(null)
    setPendingSessionConsent(null)
    setPendingUserPrompt(null)
  }, [previewOwner])

  const activeAgentRuntime = useMemo(
    () => (currentConversation ? normalizeAgentRuntime(currentConversation) : draftAgentRuntime),
    [currentConversation, draftAgentRuntime],
  )
  const usesExternalRuntime = activeAgentRuntime.kind === 'external' && !!activeAgentRuntime.externalAgentId
  const usesChatRuntime = activeAgentRuntime.kind === 'chat'
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

  const refreshToolIndicator = useCallback(async () => {
    setToolDiscoveryPending(true)
    setEnabledToolCount(null)
    setToolsDisabledReason('')
    if (!isTauriRuntime()) {
      setToolDiscoveryPending(false)
      setEnabledTools([])
      setEnabledToolCount(null)
      setToolsDisabledReason('')
      setToolsRequested(false)
      setApprovalPolicy('readonly_auto_sensitive_confirm')
      setMcpServers([])
      return
    }
    try {
      const settings = await getSettingsCached()
      const chatTools = settings.chatTools
      setMcpServers(chatTools?.servers ?? [])
      setWebSearchEnabled(chatTools?.nativeTools?.webSearch !== false)
      setProviderOAuthTypes(
        Object.fromEntries(settings.providers.map((p) => [p.id, p.request.oauth?.provider ?? ''])),
      )
      setProviderApiFormats(
        Object.fromEntries((settings.providers ?? []).map((p) => [p.id, p.apiFormat ?? ''])),
      )
      setProviderBaseUrls(
        Object.fromEntries((settings.providers ?? []).map((p) => [p.id, p.baseUrl ?? ''])),
      )
      setApprovalPolicy(chatTools?.approvalPolicy || 'readonly_auto_sensitive_confirm')
      const nextDisabledSkillIds = chatTools?.disabledSkillIds ?? []
      setDisabledSkillIds((prev) =>
        prev.length === nextDisabledSkillIds.length
        && prev.every((id, index) => id === nextDisabledSkillIds[index])
          ? prev
          : nextDisabledSkillIds,
      )
      if (!chatTools) {
        setToolDiscoveryPending(false)
        setEnabledTools([])
        setEnabledToolCount(null)
        setToolsDisabledReason('')
        setToolsRequested(false)
        setApprovalPolicy('readonly_auto_sensitive_confirm')
        return
      }
      const anyMcpEnabled = chatTools.enabled && chatTools.servers.some((server) => server.enabled)
      const anyNativeEnabled = hasEnabledNativeBuiltinTool(chatTools.nativeTools)
      const skillRuntimeEnabled = hasEnabledSkillRuntime(chatTools.nativeTools)
      const requested = anyMcpEnabled || anyNativeEnabled || skillRuntimeEnabled
      setToolsRequested(requested)
      if (!requested) {
        setToolDiscoveryPending(false)
        setEnabledTools([])
        setEnabledToolCount(null)
        setToolsDisabledReason('')
        return
      }
      const result = await api.chatMcpListTools(true)
      const tools = result.success ? result.tools : []
      setEnabledTools(tools)
      setToolDiscoveryPending(Boolean(result.discoveryPending))
      setEnabledToolCount(result.discoveryPending ? null : tools.length)
      setToolsDisabledReason(result.success ? '' : result.error || '工具不可用')
    } catch (err) {
      setToolDiscoveryPending(false)
      setEnabledTools([])
      setToolsRequested(false)
      setEnabledToolCount(null)
      setApprovalPolicy('readonly_auto_sensitive_confirm')
      setToolsDisabledReason(err instanceof Error ? err.message : String(err))
    }
  }, [])

  const handleApprovalPolicyChange = useCallback(async (nextApprovalPolicy: string) => {
    setApprovalPolicy(nextApprovalPolicy)
    try {
      // 读-改-写：必须现读后端最新态（refreshSettings），不能用缓存快照。否则若后端 OAuth
      // 令牌刷新（mcp/manager.rs persist_refreshed_server）已改写 servers[].auth 而缓存未失效，
      // 这次整体保存会把刷新后的 token 覆盖回旧值。
      await updateSettingsCached((settings) => ({
        ...settings,
        chatTools: {
          ...settings.chatTools,
          approvalPolicy: nextApprovalPolicy,
        },
      }))
      onSettingsChange()
    } catch (err) {
      console.error('Failed to update approval policy:', err)
      void refreshToolIndicator()
    }
  }, [onSettingsChange, refreshToolIndicator])

  useTauriEvent(api.onMcpServerState, (event) => {
    if (event.state.kind !== 'connecting') void refreshToolIndicator()
  }, [refreshToolIndicator])

  const handleToggleMcpServer = useCallback(async (serverId: string) => {
    try {
      // 读-改-写：现读后端最新态，避免用缓存快照 map servers[] 时把后端刚 OAuth 刷新的
      // token 覆盖回旧值（见 handleApprovalPolicyChange 注释）。
      const settings = await refreshSettings()
      const prevServers = settings.chatTools?.servers ?? []
      const current = prevServers.find((server) => server.id === serverId)
      if (current && isPluginManagedServer(current)) return
      const desiredEnabled = !current?.enabled
      const servers = preservePluginManagedServers(
        prevServers,
        prevServers.map((server) =>
          server.id === serverId ? { ...server, enabled: !server.enabled } : server,
        ),
      )
      // 乐观更新本地列表（开关即时反馈），保存后由 refreshToolIndicator 校正。
      setMcpServers(servers)
      await updateSettingsCached((fresh) => {
        const currentServers = fresh.chatTools?.servers ?? []
        const currentServer = currentServers.find((server) => server.id === serverId)
        if (!currentServer || isPluginManagedServer(currentServer)) return fresh
        const nextServers = preservePluginManagedServers(
          currentServers,
          currentServers.map((server) => (
            server.id === serverId ? { ...server, enabled: desiredEnabled } : server
          )),
        )
        return { ...fresh, chatTools: { ...fresh.chatTools, servers: nextServers } }
      })
      onSettingsChange()
      await refreshToolIndicator()
    } catch (err) {
      console.error('Failed to toggle MCP server:', err)
      void refreshToolIndicator()
    }
  }, [onSettingsChange, refreshToolIndicator])

  const unavailableRecommendedTools = useMemo(
    () => findUnavailableRecommendedTools(effectiveSkillRecommendedTools, enabledTools, toolDiscoveryPending),
    [effectiveSkillRecommendedTools, enabledTools, toolDiscoveryPending],
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


  const refreshSidebar = useCallback(() => {
    setSidebarRefreshKey((key) => key + 1)
  }, [])

  // Navigation owns the route/load/popout commit lease. The page supplies only
  // its display effects and the Tauri read adapter; no hook is wired back via ref.
  const navigation = useMemo(() => createChatNavigationController({
    currentConversation: () => currentConversationRef.current,
    currentConversationId: () => currentConversationIdRef.current,
    listPopouts: ensurePopoutIds,
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
      currentConversationIdRef.current = null
      applyConversation(null)
      restoreStreamingPreview(null)
      setContextError('')
      setContextLoading(false)
      setStreamError('')
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
        currentConversationIdRef.current = null
        setAssistantStreamStatsByMessageId({})
        setContextState(null)
        setContextError('')
        applyConversation(null)
        restoreStreamingPreview(null)
        setStreamError('')
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
      currentConversationIdRef.current = null
      applyConversation(null)
      restoreStreamingPreview(null)
    },
    discardConversation: (conversationId, error, selection) => {
      console.error('Failed to load conversation:', error)
      dropConversationLocally(conversationId)
      if (
        currentConversationIdRef.current === conversationId
        || (!selection && currentConversationIdRef.current === null)
      ) {
        currentConversationIdRef.current = null
        applyConversation(null)
      }
      refreshSidebar()
      setStreamError(error.message)
    },
  }), [
    activeAgentRuntime, activeModel, activeProviderId, applyConversation,
    dropConversationLocally, ensurePopoutIds, executionOwner, occupyConversationInMain,
    previewOwner, refreshSidebar, resetComposerDraftContext, restoreStreamingPreview,
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
    syncSettingsRoute,
    syncOnboardingRoute,
    syncAssistantCenterRoute,
    syncSkillCenterRoute,
    syncMcpCenterRoute,
    syncKnowledgeCenterRoute,
    syncNotesRoute,
    syncAutomationsRoute,
  } = useChatRouting({
    onViewChange: setChatView,
    onLoadConversation: navigation.loadRouteConversation,
    onResetConversation: navigation.resetRouteConversation,
    onLeaveConversation: navigation.leaveConversation,
    currentConversationIdRef,
    onOpenPluginsSettings: openEmbeddedSettingsForPlugins,
    onOpenSessionsSettings: openEmbeddedSettingsForSessions,
  })

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
    return scheduleIdleTask(() => {
      void refreshToolIndicator()
    }, 1500)
  }, [refreshToolIndicator])

  useEffect(() => {
    return subscribeSettings((next) => {
      setMcpServers(next.chatTools?.servers ?? [])
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

  const openEmbeddedSettings = useCallback((tab: SettingsTab = 'chat') => {
    setSettingsInitialTab(tab)
    setChatView('settings')
    syncSettingsRoute()
  }, [syncSettingsRoute])

  const handleOpenChatSettings = useCallback(() => {
    openEmbeddedSettings('chat')
  }, [openEmbeddedSettings])

  const openAssistantCenter = useCallback(() => {
    setChatView('assistants')
    syncAssistantCenterRoute()
  }, [syncAssistantCenterRoute])

  const openSkillCenter = useCallback(() => {
    setChatView('skill')
    syncSkillCenterRoute()
  }, [syncSkillCenterRoute])

  const openMcpCenter = useCallback(() => {
    setChatView('mcp')
    syncMcpCenterRoute()
  }, [syncMcpCenterRoute])

  const openKnowledgeCenter = useCallback(() => {
    setChatView('knowledge')
    syncKnowledgeCenterRoute()
  }, [syncKnowledgeCenterRoute])

  const openNotesCenter = useCallback(() => {
    setChatView('notes')
    syncNotesRoute()
  }, [syncNotesRoute])

  const openAutomationsCenter = useCallback(() => {
    setChatView('automations')
    syncAutomationsRoute()
  }, [syncAutomationsRoute])

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
    if (item === 'mcp') {
      openMcpCenter()
      return
    }
    if (item === 'knowledge') {
      openKnowledgeCenter()
      return
    }
    if (item === 'notes') {
      openNotesCenter()
      return
    }
    if (item === 'automations') {
      openAutomationsCenter()
      return
    }
  }, [openAssistantCenter, openSkillCenter, openMcpCenter, openKnowledgeCenter, openNotesCenter, openAutomationsCenter])

  const extensionsActive = useMemo<ExtensionsNavItem | null>(() => {
    if (chatView === 'assistants') return 'assistants'
    if (chatView === 'skill') return 'skill'
    if (chatView === 'mcp') return 'mcp'
    if (chatView === 'knowledge') return 'knowledge'
    if (chatView === 'notes') return 'notes'
    if (chatView === 'automations') return 'automations'
    return null
  }, [chatView])

  const [settingsExiting, setSettingsExiting] = useState(false)
  const handleSettingsClose = useCallback(() => {
    // 先播退场下滑动画，动画结束再真正切视图卸载（与 CSS 时长对齐）。
    setSettingsExiting(true)
    window.setTimeout(() => {
      setSettingsExiting(false)
      setChatView('conversation')
      const pending = pendingAfterSettingsCloseRef.current
      pendingAfterSettingsCloseRef.current = null
      completeSettingsExit(
        currentConversationIdRef.current,
        pending,
        syncConversationRoute,
      )
      void loadSkills()
      void refreshToolIndicator()
    }, 220)
  }, [loadSkills, refreshToolIndicator, syncConversationRoute])

  // 中心页（技能/MCP/专家）没有自己的返回按钮，离开靠侧栏选会话/新建等任意路径。
  // 统一在「回到会话视图」这个转变点刷新技能列表与工具指示器，
  // 保证中心页里的启停/增删在回到聊天后立即生效（替代原各页 onClose 的刷新职责）。
  const prevChatViewRef = useRef(chatView)
  useEffect(() => {
    const prev = prevChatViewRef.current
    prevChatViewRef.current = chatView
    if (chatView !== 'conversation' || prev === chatView) return
    if (prev === 'skill' || prev === 'mcp' || prev === 'assistants' || prev === 'knowledge' || prev === 'settings') {
      void loadSkills()
      void refreshToolIndicator()
    }
  }, [chatView, loadSkills, refreshToolIndicator])

  const runAfterLeavingSettings = useCallback((
    action: () => void,
    options?: { restoreCurrentRoute?: boolean },
  ) => {
    if (chatView !== 'settings') {
      action()
      return
    }
    if (!settingsRef.current) {
      setChatView('conversation')
      completeSettingsExit(
        currentConversationIdRef.current,
        {
          action,
          restoreCurrentRoute: options?.restoreCurrentRoute ?? true,
        },
        syncConversationRoute,
      )
      return
    }
    pendingAfterSettingsCloseRef.current = {
      action,
      restoreCurrentRoute: options?.restoreCurrentRoute ?? true,
    }
    // The queued navigation fires only from SettingsShell.onClose after its
    // canonical draft flush succeeds. On failure, keep settings and the action
    // in place so the user can repair/retry instead of losing the draft.
    settingsRef.current.requestClose()
  }, [chatView, syncConversationRoute])

  const handleSettingsChange = useCallback(() => {
    onSettingsChange()
    void loadDefaultModel()
    void loadSkills()
    void refreshToolIndicator()
    setSidebarProfileRefreshKey((key) => key + 1)
  }, [loadDefaultModel, loadSkills, onSettingsChange, refreshToolIndicator])

  useEffect(() => {
    void ensurePopoutIds()
  }, [ensurePopoutIds])

  useTauriEvent(api.onConversationPopoutsChanged, (payload) => {
    const previous = popoutConversationIdsRef.current
    const next = new Set(payload.conversationIds)
    for (const id of previous) {
      if (!next.has(id)) poppedGeneratingRunsRef.current.delete(id)
    }
    replacePopoutIds(next)
    void navigation.reconcilePopouts(previous, next)
  }, [replacePopoutIds, navigation])

  useEffect(() => {
    if (!popoutNotice) return
    const timer = window.setTimeout(() => setPopoutNotice(null), 4000)
    return () => window.clearTimeout(timer)
  }, [popoutNotice])

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
    if (!conversationId || compactingConversationIds.has(conversationId)) return
    markConversationCompacting(conversationId, true)
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
        setContextError(typeof err === 'string' ? err : (err as Error).message || '上下文压缩失败')
      }
    } finally {
      // 清零不看「我还在不在这个会话」——切走后原来那个守卫永远不成立，标志会卡死。
      markConversationCompacting(conversationId, false)
    }
  }, [compactingConversationIds, markConversationCompacting, patchContextState, refreshSidebar])

  const handleClearContext = useCallback(async () => {
    const conversationId = currentConversationIdRef.current
    if (!conversationId) return
    setContextError('')
    try {
      const result = await chatApi.clearContext(conversationId)
      if (currentConversationIdRef.current === conversationId) {
        const latestId = latestClearBoundaryId(result.contextState)
        if (latestId) {
          setAnimateClearBoundaryId(latestId)
          window.setTimeout(() => {
            setAnimateClearBoundaryId((current) => (current === latestId ? null : current))
          }, 1800)
        }
        patchContextState(result.contextState)
        refreshSidebar()
      }
    } catch (err) {
      if (currentConversationIdRef.current === conversationId) {
        setContextError(typeof err === 'string' ? err : (err as Error).message || '清空上下文失败')
      }
    }
  }, [patchContextState, refreshSidebar])

  const finishStreamingRun = useCallback(
    async (payload: { reason?: string; conversationId?: string }) => {
      const conversationId = payload.conversationId ?? currentConversationIdRef.current
      const preservedPartial = payload.reason === 'error' && conversationId
        ? freezeStreamSnapshot(conversationId)
        : false
      // 兜底：run 结束时压缩必然已终止；防御后端遗漏终止事件把"压缩中"状态卡死。
      if (conversationId) markConversationCompacting(conversationId, false)
      if (payload.reason === 'error' && conversationId) {
        setStreamErrorForConversation(
          conversationId,
          streamErrorsRef.current[conversationId] || '回复生成失败，请稍后重试。',
        )
      }
      if (conversationId) {
        if (currentConversationIdRef.current === conversationId) {
          await reloadConversation(conversationId, { force: true })
        }
        refreshSidebar()
      }
      if (conversationId) {
        // in-flight 也一起清：终局事件是「这一轮结束了」的权威。
        // 走 send/regenerate 的 run 到这里时它们的 finally 已经清过（这里是幂等的空操作）；
        // 而**恢复的 run**（restoredFromSnapshot，窗口重载后后端回放正在跑的那轮）没有 invoke
        // 归属它，只有这条路径能清 —— 漏了它侧栏那颗转圈就永远停不下来。
        clearConversationLocalState(localState(), conversationId)
        clearConversationInFlight(conversationId)
        if (!preservedPartial) settleStreamingPreview(conversationId)
        syncGeneratingConversationIds()
      }
      if (conversationId && currentConversationRef.current?.id === conversationId) {
        setPendingToolConfirm(null)
        setPendingSessionConsent(null)
        setPendingUserPrompt(null)
      }
    },
    [clearConversationInFlight, freezeStreamSnapshot, localState, markConversationCompacting, refreshSidebar, reloadConversation, setStreamErrorForConversation, settleStreamingPreview, syncGeneratingConversationIds],
  )

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
      setPendingToolConfirm(null)
      setPendingSessionConsent(null)
      setPendingUserPrompt(null)
    }
    clearConversationLocalState(localState(), conversationId)
    settleStreamingPreview(conversationId)
    syncGeneratingConversationIds()
  }, [applyConversation, localState, settleStreamingPreview, syncGeneratingConversationIds])

  const settlementPorts = useMemo<Parameters<ReturnType<typeof createChatExecutionOwner>['finish']>[2]>(() => ({
      completeWithConversation: finishStreamingRunWithConversation,
      completeTerminal: finishStreamingRun,
      abandonPreview: (id) => {
        if (!freezeStreamSnapshot(id)) clearStreamSnapshot(id)
      },
      settleQueue: (id, conversation) => messageQueueRef.current.settleAfterRun(id, conversation),
    }), [clearStreamSnapshot, finishStreamingRun, finishStreamingRunWithConversation, freezeStreamSnapshot])

  const settleRun = useCallback((lease: ExecutionLease, persistedConversation: Conversation | null) => (
    executionOwner.finish(lease, persistedConversation, settlementPorts)
  ), [executionOwner, settlementPorts])

  useTauriEvent(api.onChatProtocolIssue, ({ issue, conversationId }) => {
    if (issue === 'version_mismatch') {
      setProtocolVersionMismatch(true)
    } else if (
      issue === 'resync_required'
      && conversationId
      && conversationId === currentConversationIdRef.current
      && !popoutConversationIdsRef.current.has(conversationId)
    ) {
      void reloadConversation(conversationId)
    }
  }, [reloadConversation])

  useTauriEvent(api.onChatStream, (payload) => {
      if (popoutConversationIdsRef.current.has(payload.conversationId)) {
        if (payload.type === 'run_started' && payload.runId) {
          let runs = poppedGeneratingRunsRef.current.get(payload.conversationId)
          if (!runs) {
            runs = new Set()
            poppedGeneratingRunsRef.current.set(payload.conversationId, runs)
          }
          runs.add(payload.runId)
          markConversationInFlight(payload.conversationId)
        } else if (isStreamTerminal(payload)) {
          const runs = poppedGeneratingRunsRef.current.get(payload.conversationId)
          if (runs && payload.runId) runs.delete(payload.runId)
          if (!runs || runs.size === 0) {
            poppedGeneratingRunsRef.current.delete(payload.conversationId)
            clearConversationInFlight(payload.conversationId)
          }
        }
        return
      }
      const wasInFlight = executionOwner.snapshot(payload.conversationId).inFlight
      // A start packet must establish its run/group identity before the
      // cancellation fence decides whether it belongs to the cancelled run.
      if (payload.type !== 'run_started' && !executionOwner.allowsStreamPayload(payload)) return
      if (!executionOwner.observe({
        kind: 'runEvent', conversationId: payload.conversationId,
        runId: payload.runId, started: payload.type === 'run_started',
        groupId: payload.type === 'run_started' ? payload.recovery?.groupId : undefined,
      })) return
      if (payload.type === 'run_started' && !executionOwner.allowsStreamPayload(payload)) return
      const terminal = isStreamTerminal(payload)
      const terminalPayload = {
        conversationId: payload.conversationId,
        runId: payload.runId,
        reason: streamTerminalReason(payload),
      }
      if (payload.type === 'run_started') {
        if (payload.restoredFromSnapshot) restoredRunIdsRef.current.add(payload.runId)
        // 不是本窗口发起的 run（后端自起的唤醒轮 / 别的窗口的 run）：此刻会话必然不在
        // in-flight（本窗口的 send/regenerate 在 invoke 前就标了）。这类 run 没有
        // sendMessage 的统一收尾可等，必须走恢复路径在终止帧上立即 finishStreamingRun
        // ——否则下面的 markConversationInFlight 会让终止分支把收尾推迟给一个永远
        // 不会返回的 invoke，转圈和停止键永远停不下来（实测：唤醒轮消息落地后卡住）。
        if (!wasInFlight) {
          restoredRunIdsRef.current.add(payload.runId)
        }
        const remainingApprovals = (pendingToolConfirmsRef.current[payload.conversationId] ?? [])
          .filter((item) => item.runId !== payload.runId)
        if (remainingApprovals.length > 0) {
          pendingToolConfirmsRef.current[payload.conversationId] = remainingApprovals
        } else {
          delete pendingToolConfirmsRef.current[payload.conversationId]
        }
        if (currentConversationIdRef.current === payload.conversationId) {
          setPendingToolConfirm(remainingApprovals[0] ?? null)
          setToolConfirmError('')
        }
        if (pendingSessionConsentsRef.current[payload.conversationId]?.runId === payload.runId) {
          delete pendingSessionConsentsRef.current[payload.conversationId]
          if (currentConversationIdRef.current === payload.conversationId) {
            setPendingSessionConsent(null)
            setSessionConsentError('')
          }
        }
        markConversationInFlight(payload.conversationId)
        previewOwner.receive(payload)
        syncGeneratingConversationIds()
        return
      }
      if (!previewOwner.summary(payload.conversationId) && !getActiveGroup(payload.conversationId)) {
        if (!executionOwner.snapshot(payload.conversationId).inFlight) {
          if (terminal) {
            void finishStreamingRun(terminalPayload)
          }
          return
        }
      }
      const projection = previewOwner.receive(payload)
      if (!projection.accepted) return
      if (projection.target === 'group') {
        if (terminal) {
          if (restoredRunIdsRef.current.delete(payload.runId)) {
            const group = getActiveGroup(payload.conversationId)
            if (group?.columns.every((item) => !item.streaming)) {
              endGroup(payload.conversationId)
              void finishStreamingRun(terminalPayload)
            }
          }
        }
        // 组的整体「done / 持久化」交给 sendMessage 返回后的统一收尾；这里不触发 finishStreamingRun。
        return
      }
      syncGeneratingConversationIds()
      if (terminal) {
        if (restoredRunIdsRef.current.delete(payload.runId)) {
          void finishStreamingRun(terminalPayload)
          return
        }
        // invoke 未完成前不要 reload；交给 executionOwner 延迟终态，避免与 send 写盘竞态。
        if (executionOwner.snapshot(payload.conversationId).inFlight) {
          if (!executionOwner.observe({ kind: 'deferTerminal', terminal: terminalPayload })) {
            void finishStreamingRun(terminalPayload)
          }
          return
        }
        void finishStreamingRun(terminalPayload)
      }
  }, [clearConversationInFlight, executionOwner, finishStreamingRun, markConversationInFlight, previewOwner, syncGeneratingConversationIds])

  useTauriEvent(api.onChatContext, (payload) => {
    const currentConversationId = currentConversationIdRef.current
    if (!currentConversationId || payload.conversationId !== currentConversationId) {
      return
    }
    // 生成过程中的活数：只有分子 + 分母，就地补进现有状态（分段/压缩计数/来源标签留给
    // 轮末的权威快照）。不能走 patchContextState —— 那条要求一份完整的上下文状态对象。
    if (payload.live) {
      setContextState((prev) => {
        const next = applyLiveContextUsage(prev, payload.live!)
        if (!next || next === prev) return prev
        setCurrentConversation((conversation) => conversation
          ? { ...conversation, context_state: next, contextState: next }
          : conversation)
        return next
      })
      return
    }
    if (!payload.contextState) return
    patchContextState(payload.contextState)
    setContextError('')
  }, [patchContextState])

  useTauriEvent(api.onChatCompaction, (payload) => {
    const conversationId = payload.conversationId
    if (!conversationId) return
    // 压缩状态按事件里的会话记，不看是不是当前会话：后台会话的 started/completed
    // 都要收进集合，否则切走再切回来会漏掉开始、或者永远等不到结束。
    if (payload.trigger !== 'manual') {
      markConversationCompacting(conversationId, payload.phase === 'started')
    }
    if (payload.phase === 'started') return
    // 下面这些改的是当前会话的展示状态（边界动画 / currentConversation），仍要按当前会话过滤。
    if (conversationId !== currentConversationIdRef.current) return
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
  }, [markConversationCompacting])

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

  useTauriEvent(api.onChatHook, (payload) => {
    const currentConversationId = currentConversationIdRef.current
    if (!currentConversationId || payload.conversationId !== currentConversationId) {
      return
    }
    setHookWarning(payload)
  }, [])

  useTauriEvent(api.onChatTool, (payload) => {
      if (['agent', 'agent_control', 'native__agent', 'native__agent_control'].includes(payload.name) && payload.status === 'success') refreshSubAgents(payload.conversationId)
      if (popoutConversationIdsRef.current.has(payload.conversationId)) return
      if (!executionOwner.allowsStreamPayload(payload)) return
      // 忽略 invoke 结束后的迟到 tool 事件，否则会重新 setStreaming(true) 卡死输入栏。
      if (!executionOwner.snapshot(payload.conversationId).inFlight) return
      if (!executionOwner.observe({ kind: 'runEvent', conversationId: payload.conversationId, runId: payload.runId })) return
      const result = previewOwner.projectDisplay({ kind: 'tool', payload })
      if (!result.accepted) return
      // 插话卡到了 = 那条「立刻引导」真的进了模型历史，现在才把它从队列里摘掉。
      // （在此之前它一直留着，好让「没赶上轮次边界」退化成运行结束后的自动发送。）
      if (result.confirmedQueueMessageId) {
        messageQueueRef.current.confirm(payload.conversationId, result.confirmedQueueMessageId)
      }
      syncGeneratingConversationIds()
  }, [executionOwner, previewOwner, syncGeneratingConversationIds])

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
      if (popoutConversationIdsRef.current.has(payload.conversationId)) return
      if (!executionOwner.allowsStreamPayload(payload)) return
      if (!executionOwner.snapshot(payload.conversationId).inFlight) return
      if (!executionOwner.observe({ kind: 'runEvent', conversationId: payload.conversationId, runId: payload.runId })) return
      if (!previewOwner.projectDisplay({ kind: 'userPrompt', payload }).accepted) return
      // 同时排进「输入框上方」那张面板的队列：消息流里的那条只是痕迹，真正作答在面板上。
      const queue = pendingUserPromptsRef.current[payload.conversationId] ?? []
      const queued = queue.some((item) => item.toolCallId === payload.toolCallId)
      pendingUserPromptsRef.current[payload.conversationId] = queued ? queue : [...queue, payload]
      if (currentConversationIdRef.current === payload.conversationId) {
        setPendingUserPrompt(pendingUserPromptsRef.current[payload.conversationId][0] ?? null)
      }
      syncGeneratingConversationIds()
  }, [executionOwner, previewOwner, syncGeneratingConversationIds])

  /** 面板用的工具记录：**必须记忆** —— 写在 JSX 里每渲染新建一个对象，会把卡片里
   *  「换了新询问就重置草稿」的 effect 变成每渲染都重置（用户选到一半的答案被清空）。 */
  const pendingUserPromptRecord = useMemo(
    () => (pendingUserPrompt ? userPromptEventToRecord(pendingUserPrompt) : null),
    [pendingUserPrompt],
  )

  /** 面板作答完（或那一轮结束了）就把它收起来。后端没有「已答复」事件 ——
   *  `resolve_user_prompt` 只清重放快照、不发事件，所以收起由前端自己负责。 */
  const dismissPendingUserPrompt = useCallback((conversationId: string, toolCallId?: string) => {
    const rest = (pendingUserPromptsRef.current[conversationId] ?? [])
      .filter((item) => toolCallId == null || item.toolCallId !== toolCallId)
    if (rest.length > 0) {
      pendingUserPromptsRef.current[conversationId] = rest
    } else {
      delete pendingUserPromptsRef.current[conversationId]
    }
    if (currentConversationIdRef.current === conversationId) {
      setPendingUserPrompt(rest[0] ?? null)
    }
  }, [])

  useTauriEvent(api.onChatToolConfirm, (payload) => {
    if (popoutConversationIdsRef.current.has(payload.conversationId)) return
    // 排队而不是覆盖：一条消息里并行调多个工具时，后端会同时挂着多条询问等答复
    // （按 request_id 路由）。覆盖会让用户没看见的那条静默超时 ⇒ 模型收到「用户拒绝」。
    const queue = pendingToolConfirmsRef.current[payload.conversationId] ?? []
    if (!queue.some((item) => item.toolCallId === payload.toolCallId)) {
      queue.push(payload)
    }
    pendingToolConfirmsRef.current[payload.conversationId] = queue
    syncGeneratingConversationIds()
    if (currentConversationIdRef.current === payload.conversationId) {
      setPendingToolConfirm(queue[0] ?? null)
      setToolConfirmError('')
      // 计划卡一出现就在右侧栏摊开整份计划 —— 卡片上那块小灰框读不完。
      if (isPlanApproval(payload) && payload.argumentsPreview?.trim()) {
        requestDockMarkdownPreview({ title: '计划', text: payload.argumentsPreview })
      }
    }
  }, [syncGeneratingConversationIds])

  useTauriEvent(api.onChatToolConfirmWithdraw, (payload) => {
    if (popoutConversationIdsRef.current.has(payload.conversationId)) return
    const rest = (pendingToolConfirmsRef.current[payload.conversationId] ?? [])
      .filter((item) => item.toolCallId !== payload.toolCallId)
    if (rest.length > 0) {
      pendingToolConfirmsRef.current[payload.conversationId] = rest
    } else {
      delete pendingToolConfirmsRef.current[payload.conversationId]
    }
    syncGeneratingConversationIds()
    if (currentConversationIdRef.current === payload.conversationId) {
      setPendingToolConfirm((current) =>
        current?.toolCallId === payload.toolCallId ? rest[0] ?? null : current)
      setToolConfirmError('')
    }
  }, [syncGeneratingConversationIds])

  const resolvePendingToolConfirm = useCallback(async (
    approved: boolean,
    always = false,
    permissionMode: string | null = null,
  ): Promise<boolean> => {
    const prompt = pendingToolConfirm
    if (!prompt || toolConfirmSubmissionsRef.current.has(prompt.toolCallId)) return false

    toolConfirmSubmissionsRef.current.add(prompt.toolCallId)
    setToolConfirmSubmittingId(prompt.toolCallId)
    setToolConfirmError('')
    try {
      await api.chatConfirmToolCall(prompt.toolCallId, approved, always, permissionMode)
      const rest = (pendingToolConfirmsRef.current[prompt.conversationId] ?? [])
        .filter((item) => item.toolCallId !== prompt.toolCallId)
      if (rest.length > 0) {
        pendingToolConfirmsRef.current[prompt.conversationId] = rest
      } else {
        delete pendingToolConfirmsRef.current[prompt.conversationId]
      }
      syncGeneratingConversationIds()
      if (currentConversationIdRef.current === prompt.conversationId) {
        setPendingToolConfirm(rest[0] ?? null)
        setToolConfirmError('')
      }
      return true
    } catch (error) {
      console.error('Failed to submit tool confirmation:', error)
      const isStillPending = (pendingToolConfirmsRef.current[prompt.conversationId] ?? [])
        .some((item) => item.toolCallId === prompt.toolCallId)
      if (currentConversationIdRef.current === prompt.conversationId && isStillPending) {
        setToolConfirmError(
          typeof error === 'string' ? error : (error as Error).message || '提交审批失败，请重试',
        )
      }
      return false
    } finally {
      toolConfirmSubmissionsRef.current.delete(prompt.toolCallId)
      setToolConfirmSubmittingId((current) => current === prompt.toolCallId ? null : current)
    }
  }, [pendingToolConfirm, syncGeneratingConversationIds])

  useTauriEvent(api.onChatSessionConsent, (payload) => {
    if (popoutConversationIdsRef.current.has(payload.conversationId)) return
    pendingSessionConsentsRef.current[payload.conversationId] = payload
    if (currentConversationIdRef.current === payload.conversationId) {
      setPendingSessionConsent(payload)
      setSessionConsentError('')
    }
  }, [])

  const resolvePendingSessionConsent = useCallback(async (granted: boolean): Promise<boolean> => {
    const prompt = pendingSessionConsent
    if (!prompt || sessionConsentSubmissionsRef.current.has(prompt.conversationId)) return false

    sessionConsentSubmissionsRef.current.add(prompt.conversationId)
    setSessionConsentSubmittingConversationId(prompt.conversationId)
    setSessionConsentError('')
    try {
      await api.chatRespondSessionConsent(prompt.conversationId, granted)
      if (pendingSessionConsentsRef.current[prompt.conversationId]?.runId === prompt.runId) {
        delete pendingSessionConsentsRef.current[prompt.conversationId]
      }
      if (currentConversationIdRef.current === prompt.conversationId) {
        setPendingSessionConsent(pendingSessionConsentsRef.current[prompt.conversationId] ?? null)
        setSessionConsentError('')
      }
      return true
    } catch (error) {
      console.error('Failed to submit session consent:', error)
      if (
        currentConversationIdRef.current === prompt.conversationId
        && pendingSessionConsentsRef.current[prompt.conversationId]?.runId === prompt.runId
      ) {
        setSessionConsentError(
          typeof error === 'string' ? error : (error as Error).message || '提交会话授权失败，请重试',
        )
      }
      return false
    } finally {
      sessionConsentSubmissionsRef.current.delete(prompt.conversationId)
      setSessionConsentSubmittingConversationId((current) => (
        current === prompt.conversationId ? null : current
      ))
    }
  }, [pendingSessionConsent])

  useEffect(() => {
    const conversationId = currentConversation?.id
    if (!conversationId) return
    // 已弹出的会话:主窗只渲染占位卡、不渲染消息,run 边沿事件足够;
    // sync 会把该会话的运行快照(可能数百 KB)白拉到主窗协议状态里。
    if (popoutConversationIdsRef.current.has(conversationId)) return
    void api.chatSyncState(conversationId).catch((error) => {
      console.error('Failed to synchronize chat protocol state:', error)
    })
  }, [currentConversation?.id, popoutConversationIds])

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

  const handleStartAssistantChat = useCallback(async (assistant: ChatAssistant) => {
    const startingConversationId = currentConversationIdRef.current
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
      refreshSidebar()
      if (currentConversationIdRef.current === startingConversationId) {
        currentConversationIdRef.current = conv.id
        applyConversation(conv)
        restoreStreamingPreview(conv.id)
        syncConversationRoute(conv.id)
        setStreamError('')
      }
    } catch (err) {
      console.error('Failed to start assistant conversation:', err)
      if (currentConversationIdRef.current === startingConversationId) {
        setStreamError(typeof err === 'string' ? err : (err as Error).message || '创建助手对话失败')
      }
    }
  }, [activeModel, activeProviderId, applyConversation, refreshSidebar, restoreStreamingPreview, selectedProject?.id, selectedProject?.name, selectedSet?.id, syncConversationRoute])

  const handleStartBuilderChat = useCallback(async () => {
    const startingConversationId = currentConversationIdRef.current
    setAssistantStreamStatsByMessageId({})
    try {
      const conv = await chatApi.createBuilderConversation(
        activeProviderId || undefined,
        activeModel || undefined,
        selectedProject?.id ?? null,
      )
      refreshSidebar()
      if (currentConversationIdRef.current === startingConversationId) {
        currentConversationIdRef.current = conv.id
        applyConversation(conv)
        restoreStreamingPreview(conv.id)
        syncConversationRoute(conv.id)
        setStreamError('')
      }
    } catch (err) {
      console.error('Failed to start builder conversation:', err)
      if (currentConversationIdRef.current === startingConversationId) {
        setStreamError(typeof err === 'string' ? err : (err as Error).message || '创建搭建对话失败')
      }
    }
  }, [activeModel, activeProviderId, applyConversation, refreshSidebar, restoreStreamingPreview, selectedProject?.id, syncConversationRoute])

  const handleApplyAssistant = useCallback(async (assistantId: string | null) => {
    if (!currentConversation) return
    const conversationId = currentConversation.id
    try {
      const updated = await chatApi.updateConversation(conversationId, {
        assistantId: assistantId ?? '',
      })
      applyConversationIfCurrent(conversationId, updated)
      refreshSidebar()
      if (assistantId) void refreshContextStats(updated.id)
    } catch (err) {
      console.error('Failed to update conversation assistant:', err)
      setStreamErrorForConversation(
        conversationId,
        typeof err === 'string' ? err : (err as Error).message || '助手切换失败',
      )
    }
  }, [applyConversationIfCurrent, currentConversation, refreshContextStats, refreshSidebar, setStreamErrorForConversation])

  // 底栏弹层选择专家：有会话则切换该会话专家，无会话则以该专家开新对话；null=清除。
  const handleSelectAssistant = useCallback(async (assistant: ChatAssistant | null) => {
    if (!assistant) {
      await handleApplyAssistant(null)
      return
    }
    if (currentConversation) await handleApplyAssistant(assistant.id)
    else await handleStartAssistantChat(assistant)
  }, [currentConversation, handleApplyAssistant, handleStartAssistantChat])

  const ensureConversationForAgentPlan = useCallback(async () => {
    if (currentConversation) return currentConversation
    const startingConversationId = currentConversationIdRef.current
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
    if (currentConversationIdRef.current === startingConversationId) {
      currentConversationIdRef.current = conversation.id
      applyConversation(conversation)
      syncConversationRoute(conversation.id)
    }
    return conversation
  }, [activeModel, activeProviderId, applyConversation, currentConversation, draftAgentRuntime, refreshSidebar, selectedProject?.id, selectedProject?.name, selectedSet?.id, syncConversationRoute])

  const handleAgentPlanModeChange = useCallback(async (mode: AgentPlanMode) => {
    const startingConversationId = currentConversationIdRef.current
    let targetConversationId = currentConversation?.id ?? null
    try {
      const conversation = await ensureConversationForAgentPlan()
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
      } else if (currentConversationIdRef.current === startingConversationId) {
        setStreamError(typeof err === 'string' ? err : (err as Error).message || 'Plan 模式切换失败')
      }
    }
  }, [applyConversationIfCurrent, currentConversation?.id, ensureConversationForAgentPlan, refreshContextStats, refreshSidebar, setStreamErrorForConversation])

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
      if (currentConversationIdRef.current === event.startingConversationId) {
        currentConversationIdRef.current = event.conversation.id
        applyConversation(event.conversation)
        syncConversationRoute(event.conversation.id)
      }
      return
    }
    if (event.kind === 'updated') {
      applyConversationIfCurrent(event.conversation.id, event.conversation)
      return
    }
    if (event.kind === 'rejected') {
      if (event.conversationId) {
        setStreamErrorForConversation(event.conversationId, event.error.message)
      } else if (currentConversationIdRef.current === event.startingConversationId) {
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
        settleOptimisticConversationListItem(
          setOptimisticSidebarConversations, conversationId, outcome.conversation,
        )
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
    settleOptimisticConversationListItem(
      setOptimisticSidebarConversations, conversationId, keptConversation,
    )
    if (keptConversation) refreshSidebar()
    setStreamErrorForConversation(conversationId, outcome.error.message || '发送失败')
    if (!freezeStreamSnapshot(conversationId)) clearStreamSnapshot(conversationId)
  }, [
    applyAssistantStreamStats, applyConversation, applyConversationIfCurrent,
    clearStreamSnapshot, freezeStreamSnapshot, refreshSidebar,
    setStreamErrorForConversation, syncConversationRoute, syncGeneratingConversationIds,
  ])

  const sendController = useMemo(() => createChatSendController({
    executionOwner,
    previewOwner,
    persistence: chatApi,
    settlementPorts,
    presentation: {
      currentConversationId: () => currentConversationIdRef.current,
      present: presentSendEvent,
    },
  }), [executionOwner, previewOwner, settlementPorts, presentSendEvent])

  const handleSendMessage = useCallback(async (
    content: string,
    attachments: PendingAttachment[] = [],
    options: SendMessageOptions = {},
  ) => {
    const attachmentSkillId = usesChatRuntime
      ? null
      : options.forceNewConversation
        ? inferSingleAttachmentSkillId(attachments, enabledSkills)
        : effectiveSkillId ?? inferSingleAttachmentSkillId(attachments, enabledSkills)
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
    const startingConversationId = currentConversationIdRef.current
    try {
      const conversation = await chatApi.importExternalConversation(
        messages,
        attachmentPaths,
        activeProviderId || undefined,
        activeModel || undefined,
        selectedProject?.id ?? null,
      )
      refreshSidebar()
      if (currentConversationIdRef.current === startingConversationId) {
        currentConversationIdRef.current = conversation.id
        applyConversation(conversation)
        syncConversationRoute(conversation.id)
      }
      return true
    } catch (err) {
      console.error('Failed to import external conversation:', err)
      if (currentConversationIdRef.current === startingConversationId) {
        setStreamError(typeof err === 'string' ? err : (err as Error).message || '导入对话失败')
      }
      return false
    }
  }, [activeModel, activeProviderId, applyConversation, refreshSidebar, selectedProject?.id, syncConversationRoute])
  const importExternalConversationRef = useRef(importExternalConversation)
  importExternalConversationRef.current = importExternalConversation

  const { drainExternalSends, hasPendingDrainRequest } = useExternalSendQueue({
    onEnterConversationView: () => setChatView('conversation'),
    onImportConversation: (messages, attachmentPaths) =>
      importExternalConversationRef.current(messages, attachmentPaths),
    onSendMessage: (content, attachments, options) =>
      handleSendMessageRef.current(content, attachments, options),
    onError: setStreamError,
  })

  // 运行中的消息队列（Codex 式排队 + 立刻引导）。同上用 ref 转发 handleSendMessage，
  // 保持 drain 的身份稳定（它被 handleSendMessage 自己的 finally 调用，不能互相拖依赖）。
  const messageQueue = useMessageQueue({
    onSendMessage: (content, attachments, options) =>
      handleSendMessageRef.current(content, attachments, options),
    onRestoreToComposer: (message) => insertTextIntoComposer(message.content),
    onPendingChange: (conversationId, pending) => {
      void chatApi.setGoalUserQueuePending(conversationId, pending)
    },
  })
  const messageQueueRef = useRef(messageQueue)
  messageQueueRef.current = messageQueue

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
    const message = messageQueueRef.current.enqueue(conversation.id, content, attachments)
    if (message && canFollowUpCurrentConversation) {
      void messageQueueRef.current.followUp(conversation, message.id)
    }
  }, [canFollowUpCurrentConversation])

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
          const queued = messageQueueRef.current.enqueue(conversation.id, text, [])
          if (!queued) throw new Error('答复未能加入消息队列，请重试')
          if (!generatingConversationIdsRef.current.has(conversation.id)) {
            void messageQueueRef.current.drain(conversation)
          }
        }
        setClosedAsyncQuestions((previous) => ({
          ...previous, [conversationId]: [...(previous[conversationId] ?? []), toolId],
        }))
      },
    }
  }, [closedAsyncQuestions, currentConversation])

  const handleSteerQueuedMessage = useCallback((messageId: string) => {
    const conversationId = currentConversationIdRef.current
    if (!conversationId) return
    void messageQueueRef.current.steer(conversationId, messageId)
  }, [])

  const handleRemoveQueuedMessage = useCallback((messageId: string) => {
    const conversationId = currentConversationIdRef.current
    if (!conversationId) return
    messageQueueRef.current.remove(conversationId, messageId)
  }, [])

  const handleRestoreQueuedMessage = useCallback((messageId: string) => {
    const conversationId = currentConversationIdRef.current
    if (!conversationId) return
    messageQueueRef.current.restoreToComposer(conversationId, messageId)
  }, [])

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

  useEffect(() => {
    if (!streamCoarse.streaming && hasPendingDrainRequest()) {
      void drainExternalSends()
    }
  }, [drainExternalSends, hasPendingDrainRequest, streamCoarse.streaming])

  const handleUpdateMessage = useCallback(
    async (messageId: string, content: string) => {
      const conv = currentConversationRef.current
      if (!conv) return
      try {
        const updated = await chatApi.updateMessage(conv.id, messageId, content)
        applyConversationIfCurrent(conv.id, updated)
        refreshSidebar()
      } catch (err) {
        console.error('Failed to update message:', err)
        setStreamErrorForConversation(
          conv.id,
          typeof err === 'string' ? err : (err as Error).message || '保存失败',
        )
      }
    },
    [applyConversationIfCurrent, refreshSidebar, setStreamErrorForConversation],
  )

  const handleDeleteMessage = useCallback(
    async (messageId: string) => {
      const conv = currentConversationRef.current
      if (!conv) return
      if (!window.confirm('确定删除这条消息吗？')) return
      try {
        const updated = await chatApi.deleteMessage(conv.id, messageId)
        if (applyConversationIfCurrent(conv.id, updated)) {
          setAssistantStreamStatsByMessageId((prev) => {
            const next = { ...prev }
            delete next[messageId]
            return next
          })
        }
        refreshSidebar()
      } catch (err) {
        console.error('Failed to delete message:', err)
        setStreamErrorForConversation(
          conv.id,
          typeof err === 'string' ? err : (err as Error).message || '删除失败',
        )
      }
    },
    [applyConversationIfCurrent, refreshSidebar, setStreamErrorForConversation],
  )

  // 一键 rewind（「回到这里」）：截掉这条提问及其之后的所有消息，原文塞回输入框，用户改完再自己发。
  // 破坏性且不可撤销 → 先 confirm（与删除消息同一把关）。
  const handleRewindMessage = useCallback(
    async (messageId: string) => {
      const conv = currentConversationRef.current
      if (!conv) return
      if (!window.confirm('回到这里？这条提问及其之后的所有消息会被删除，原文放回输入框。')) return
      try {
        const { conversation, content } = await chatApi.rewindToMessage(conv.id, messageId)
        const applied = applyConversationIfCurrent(conv.id, conversation)
        if (applied) {
          setAssistantStreamStatsByMessageId({})
          setStreamError('')
          insertTextIntoComposer(content)
        }
        refreshSidebar()
        // 上下文用量后台补算（后端 rewind 故意不算，见那边注释）：几秒的 MCP 列表不该挡住 UI。
        void refreshContextStats(conversation.id)
      } catch (err) {
        console.error('Failed to rewind conversation:', err)
        setStreamErrorForConversation(
          conv.id,
          typeof err === 'string' ? err : (err as Error).message || '回到这里失败',
        )
      }
    },
    [applyConversationIfCurrent, refreshContextStats, refreshSidebar, setStreamErrorForConversation],
  )

  // 对话分支（方案 B）：在某条消息处建分支——把该消息及之前的消息复制进新对话，
  // 立即打开新对话（不自动发送）。源对话只读、不受影响。
  const handleForkMessage = useCallback(
    async (messageId: string) => {
      const conv = currentConversationRef.current
      if (!conv) return
      const startingConversationId = conv.id
      try {
        const forked = await chatApi.forkConversation(conv.id, messageId)
        refreshSidebar()
        if (currentConversationIdRef.current === startingConversationId) {
          setAssistantStreamStatsByMessageId({})
          currentConversationIdRef.current = forked.id
          applyConversation(forked)
          restoreStreamingPreview(forked.id)
          syncConversationRoute(forked.id)
          setStreamError('')
        }
      } catch (err) {
        console.error('Failed to fork conversation:', err)
        setStreamErrorForConversation(
          conv.id,
          typeof err === 'string' ? err : (err as Error).message || '建分支失败',
        )
      }
    },
    [applyConversation, refreshSidebar, restoreStreamingPreview, setStreamErrorForConversation, syncConversationRoute],
  )

  const handleSaveMessageToNote = useCallback(
    async (messageId: string) => {
      const conv = currentConversationRef.current
      if (!conv) return false
      const message = conv.messages.find((m) => m.id === messageId)
      if (!message) return false
      const content = message.content?.trim() || ''
      if (!content) return false

      const firstLine = content
        .split('\n')
        .map((line) => line.trim())
        .find((line) => line.length > 0)
      const title = firstLine
        ? firstLine
            .replace(/^#+\s*/, '')
            .replace(/\*\*|__|\*|_|`>/g, '')
            .slice(0, 40)
            .trim() || '对话笔记'
        : '对话笔记'
      try {
        await api.notesCreate(title, content, '', 'chat')
        setStreamError('')
        return true
      } catch (err) {
        console.error('Failed to save message to note:', err)
        setStreamError(err instanceof Error ? err.message : String(err) || '存为笔记失败')
        return false
      }
    },
    [],
  )

  // 多答组「选中条」（任务 06-30 / D5）：标记某组进下一轮历史的列。默认第一列；用户点选改。
  const handleSetGroupSelection = useCallback(
    async (groupId: string, messageId: string) => {
      const conv = currentConversationRef.current
      if (!conv) return
      try {
        const updated = await chatApi.setGroupSelection(conv.id, groupId, messageId)
        applyConversationMeta(updated)
      } catch (err) {
        console.error('Failed to set group selection:', err)
        setStreamErrorForConversation(
          conv.id,
          typeof err === 'string' ? err : (err as Error).message || '选中失败',
        )
      }
    },
    [applyConversationMeta, setStreamErrorForConversation],
  )

  const handleRegenerateMessage = useCallback(
    async (messageId: string, newContent?: string) => {
      const conv = currentConversationRef.current
      if (!conv) return

      const conversationId = conv.id
      // Busy 拒绝（AC3）：入口已在 MessageList 按 streaming/frozen 收起，这里是兜底。
      // 带编辑内容时静默 return 会无声丢掉用户改的文字，必须给出提示（与 handleSend 同文案）。
      if (executionOwner.snapshot(conversationId).inFlight) {
        setStreamErrorForConversation(conversationId, '该对话正在生成中，请稍后再试')
        return
      }

      const messageIndex = conv.messages.findIndex(
        (message) => message.id === messageId,
      )
      if (messageIndex < 0) return

      // 助手消息：截到它之前重生成。用户消息：保留它（编辑时先替换内容）、只丢其后内容再重试。
      const keepTarget = conv.messages[messageIndex].role === 'user'
      const cutFrom = keepTarget ? messageIndex + 1 : messageIndex
      // 空白-only 的编辑内容按「未编辑」处理（纯重生成）：绝不能把 Some("") 发给后端——
      // 乐观截断已经执行，后端再报「消息内容不能为空」会留下截断了却没重生成的线程。
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
      const startedAt = Date.now()
      const lease = executionOwner.begin({ conversationId, kind: 'regenerate', startedAt })
      if (!lease) return
      previewOwner.begin(conversationId, startedAt)
      syncGeneratingConversationIds()

      if (currentConversationIdRef.current === conversationId) {
        setStreamErrorForConversation(conversationId, '')
      }

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
          typeof err === 'string' ? err : (err as Error).message || '重新生成失败',
        )
        if (!freezeStreamSnapshot(conversationId)) clearStreamSnapshot(conversationId)
        if (currentConversationIdRef.current === conversationId) {
          void reloadConversation(conversationId)
        }
      } finally {
        await settleRun(lease, persistedConversation)
        syncGeneratingConversationIds()
      }
    },
    [applyAssistantStreamStats, applyConversation, clearStreamSnapshot, executionOwner, freezeStreamSnapshot, previewOwner, refreshSidebar, reloadConversation, setStreamErrorForConversation, settleRun, syncGeneratingConversationIds],
  )

  const handleReplyWithModel = useCallback(
    async (messageId: string, providerId: string, model: string) => {
      const conv = currentConversationRef.current
      if (!conv) return
      const conversationId = conv.id
      if (executionOwner.snapshot(conversationId).inFlight) {
        setStreamErrorForConversation(conversationId, '该对话正在生成中，请稍后再试')
        return
      }
      const span = assistantTurnSpan(conv.messages, messageId)
      if (!span || span.end !== conv.messages.length - 1) {
        setStreamErrorForConversation(conversationId, '只能对最后一轮回答换模型')
        return
      }
      const groupId = span.groupId || `grp_${crypto.randomUUID()}`
      const sessionProvider = conv.provider_id ?? ''
      const sessionModel = conv.model ?? ''
      const lease = executionOwner.begin({
        conversationId, kind: 'replyWithModel', startedAt: Date.now(),
        group: { groupId, arms: [
          ...span.siblings.map((message) => ({
            providerId: message.provider_id ?? message.providerId ?? sessionProvider,
            model: message.model ?? sessionModel,
            messageId: message.id,
            streaming: false,
            content: message.content,
            reasoning: message.reasoning,
            toolCalls: message.tool_calls ?? message.toolCalls ?? [],
            segments: message.segments ?? [],
          })),
          { providerId, model },
        ] },
      })
      if (!lease) return
      previewOwner.begin(conversationId, Date.now(), 'group')
      syncGeneratingConversationIds()
      if (currentConversationIdRef.current === conversationId) {
        setStreamErrorForConversation(conversationId, '')
      }
      let persistedConversation: Conversation | null = null
      try {
        const updated = await chatApi.replyWithModel(
          conversationId,
          messageId,
          providerId,
          model,
          groupId,
        )
        persistedConversation = updated
        if (currentConversationIdRef.current === conversationId) {
          applyAssistantStreamStats(updated)
          applyConversation(updated)
          refreshSidebar()
        } else {
          refreshSidebar()
        }
      } catch (err) {
        console.error('Failed to reply with model:', err)
        setStreamErrorForConversation(
          conversationId,
          typeof err === 'string' ? err : (err as Error).message || '换模型回答失败',
        )
        if (currentConversationIdRef.current === conversationId) {
          void reloadConversation(conversationId)
        }
      } finally {
        await settleRun(lease, persistedConversation)
        syncGeneratingConversationIds()
      }
    },
    [applyAssistantStreamStats, applyConversation, executionOwner, previewOwner, refreshSidebar, reloadConversation, setStreamErrorForConversation, settleRun, syncGeneratingConversationIds],
  )

  const handleRuntimeChange = useCallback(async (runtime: AgentRuntimeConfig) => {
    setDraftAgentRuntime(runtime)
    saveLastAgentRuntime(runtime)
    if (!currentConversation) return
    const conversationId = currentConversation.id
    try {
      const goal = currentConversation.goal_state ?? currentConversation.goalState
      if (goal && !['completed', 'cancelled', 'paused'].includes(goal.status)) {
        const paused = await chatApi.pauseGoal(conversationId)
        applyConversationIfCurrent(conversationId, paused)
      }
      const updated = await chatApi.setAgentRuntime(conversationId, runtime)
      applyConversationIfCurrent(conversationId, updated)
    } catch (err) {
      console.error('Failed to change agent runtime:', err)
      setStreamErrorForConversation(
        conversationId,
        typeof err === 'string' ? err : (err as Error).message || 'Agent 切换失败',
      )
    }
  }, [applyConversationIfCurrent, currentConversation, setDraftAgentRuntime, setStreamErrorForConversation])

  const handleExternalModelChange = useCallback(async (model: string, reasoning?: string | null) => {
    // Route through handleRuntimeChange so the draft updates even before a conversation exists
    // (the draft is applied when the conversation is created on first send).
    const next = withExternalModel(activeAgentRuntime, model, reasoning)
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

  const handleExternalPresetChange = useCallback(async (preset: string) => {
    const next: AgentRuntimeConfig = {
      ...activeAgentRuntime,
      kind: 'external',
      externalAgentPreset: preset,
    }
    await handleRuntimeChange(next)
  }, [activeAgentRuntime, handleRuntimeChange])

  const persistApprovedExternalSandbox = useCallback(async (
    conversationId: string,
    runtime: AgentRuntimeConfig,
    sandbox: string,
  ) => {
    const next: AgentRuntimeConfig = {
      ...runtime,
      kind: 'external',
      externalSandbox: sandbox,
    }
    try {
      const updated = await chatApi.setAgentRuntime(conversationId, next)
      if (applyConversationIfCurrent(conversationId, updated)) {
        setDraftAgentRuntime(next)
        saveLastAgentRuntime(next)
      }
    } catch (error) {
      console.error('Failed to persist the post-approval permission mode:', error)
      setStreamErrorForConversation(
        conversationId,
        typeof error === 'string' ? error : (error as Error).message || '权限模式保存失败',
      )
    }
  }, [applyConversationIfCurrent, setDraftAgentRuntime, setStreamErrorForConversation])

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

  const runGoalMutation = useCallback(async (
    mutation: (conversationId: string) => Promise<Conversation>,
    continueWhenActive = false,
  ) => {
    const conversationId = currentConversationIdRef.current
    if (!conversationId) return
    try {
      const updated = await mutation(conversationId)
      applyConversationIfCurrent(conversationId, updated)
      const goal = updated.goal_state ?? updated.goalState
      if (continueWhenActive && goal && (goal.status === 'active' || goal.status === 'verifying')) {
        void chatApi.continueGoal(conversationId).then((result) => {
          applyConversationIfCurrent(conversationId, result)
        }).catch((error) => {
          setStreamErrorForConversation(conversationId, error instanceof Error ? error.message : String(error))
        })
      }
    } catch (error) {
      setStreamErrorForConversation(
        conversationId,
        typeof error === 'string' ? error : (error as Error).message || 'Goal 操作失败',
      )
      throw error
    }
  }, [applyConversationIfCurrent, setStreamErrorForConversation])

  const handleEditGoal = useCallback((objective: string) => runGoalMutation(
    (conversationId) => chatApi.editGoal(conversationId, objective),
    true,
  ), [runGoalMutation])
  const handlePauseGoal = useCallback(() => runGoalMutation(chatApi.pauseGoal), [runGoalMutation])
  const handleResumeGoal = useCallback(() => runGoalMutation(chatApi.resumeGoal, true), [runGoalMutation])
  const handleCancelGoal = useCallback(() => runGoalMutation(chatApi.cancelGoal), [runGoalMutation])

  const handleModelChange = useCallback(async (providerId: string, model: string) => {
    setDraftProviderModel(providerId, model)
    saveLastModel(providerId, model)
    void persistLastChatModelToSettings(providerId, model)

    if (!currentConversation) return
    const conversationId = currentConversation.id

    try {
      const updatedConv = await chatApi.updateConversation(conversationId, {
        providerId,
        model,
      })
      applyConversationMeta(updatedConv)
    } catch (err) {
      console.error('Failed to change model:', err)
      setStreamErrorForConversation(
        conversationId,
        typeof err === 'string' ? err : (err as Error).message || '模型切换失败',
      )
    }
  }, [applyConversationMeta, currentConversation, setDraftProviderModel, setStreamErrorForConversation])

  const handleThinkingLevelChange = useCallback(async (level: ThinkingLevel | null) => {
    setDraftThinkingLevel(level)
    saveLastThinkingLevel(level) // 记住为全局默认，不再回落到 high
    if (!currentConversation) return
    const conversationId = currentConversation.id
    try {
      const updatedConv = await chatApi.updateConversation(conversationId, {
        thinkingLevel: level,
      })
      applyConversationMeta(updatedConv)
    } catch (err) {
      console.error('Failed to change thinking level:', err)
      setStreamErrorForConversation(
        conversationId,
        typeof err === 'string' ? err : (err as Error).message || '思考等级切换失败',
      )
    }
  }, [applyConversationMeta, currentConversation, setDraftThinkingLevel, setStreamErrorForConversation])

  // 会话级三态联网搜索（任务 07-23）：设置模式,持久化到会话(欢迎页先存草稿),
  // 并记住为全局默认——之后所有新会话/未显式设置的会话自动沿用(与思考等级同款)。
  const handleSetWebSearchMode = useCallback(async (mode: WebSearchMode) => {
    setDraftWebSearchMode(mode)
    saveLastWebSearchMode(mode)
    if (!currentConversation) return
    const conversationId = currentConversation.id
    try {
      const updatedConv = await chatApi.updateConversation(conversationId, {
        webSearchMode: mode,
      })
      applyConversationMeta(updatedConv)
    } catch (err) {
      console.error('Failed to change web search mode:', err)
      setStreamErrorForConversation(
        conversationId,
        typeof err === 'string' ? err : (err as Error).message || '联网搜索模式切换失败',
      )
    }
  }, [applyConversationMeta, currentConversation, setDraftWebSearchMode, setStreamErrorForConversation])

  // 多模型一问多答（任务 06-30 / D2）：变更多答模型集，持久化到会话（欢迎页先存草稿）。
  // 上限 4 由 UI 侧约束；这里直落 chatApi.updateConversation({ replyModels })。
  const handleChangeReplyModels = useCallback(async (models: ModelRef[]) => {
    setDraftReplyModels(models)
    if (!currentConversation) return
    const conversationId = currentConversation.id
    try {
      const updatedConv = await chatApi.updateConversation(conversationId, {
        replyModels: models,
      })
      applyConversationMeta(updatedConv)
    } catch (err) {
      console.error('Failed to update reply models:', err)
      setStreamErrorForConversation(
        conversationId,
        typeof err === 'string' ? err : (err as Error).message || '多答模型更新失败',
      )
    }
  }, [applyConversationMeta, currentConversation, setDraftReplyModels, setStreamErrorForConversation])

  const handleChangeKnowledgeBaseIds = useCallback(async (ids: string[]) => {
    // the draft is applied when the conversation is created on first send.
    setDraftKnowledgeBaseIds(ids)
    if (!currentConversation) return
    const conversationId = currentConversation.id
    try {
      const updatedConv = await chatApi.updateConversation(conversationId, {
        knowledgeBaseIds: ids,
      })
      applyConversationMeta(updatedConv)
    } catch (err) {
      console.error('Failed to update knowledge bases:', err)
    }
  }, [applyConversationMeta, currentConversation, setDraftKnowledgeBaseIds])

  const handleChangeAdditionalDirectories = useCallback(async (directories: AdditionalDirectory[]) => {
    setDraftAdditionalDirectories(directories)
    if (!currentConversation) return
    const conversationId = currentConversation.id
    try {
      const updatedConv = await chatApi.updateConversation(conversationId, {
        additionalDirectories: directories,
      })
      applyConversationMeta(updatedConv)
    } catch (err) {
      console.error('Failed to update additional directories:', err)
      setStreamErrorForConversation(
        conversationId,
        typeof err === 'string' ? err : (err as Error).message || '附加目录更新失败',
      )
    }
  }, [applyConversationMeta, currentConversation, setDraftAdditionalDirectories, setStreamErrorForConversation])

  const handleToggleForceKnowledgeSearch = useCallback(async () => {
    const next = !(currentConversation
      ? (currentConversation.force_knowledge_search ?? currentConversation.forceKnowledgeSearch ?? false)
      : draftForceKnowledgeSearch)
    setDraftForceKnowledgeSearch(next)
    if (!currentConversation) return
    const conversationId = currentConversation.id
    try {
      const updatedConv = await chatApi.updateConversation(conversationId, {
        forceKnowledgeSearch: next,
      })
      applyConversationMeta(updatedConv)
    } catch (err) {
      console.error('Failed to update force knowledge search:', err)
    }
  }, [applyConversationMeta, currentConversation, draftForceKnowledgeSearch, setDraftForceKnowledgeSearch])

  const handleCancelStream = useCallback(async () => {
    const conversationId = currentConversationIdRef.current
    if (
      !conversationId
      || getStreamCoarse().cancelling
      || !(executionOwner.snapshot(conversationId).inFlight || previewOwner.isStreaming(conversationId))
    ) {
      return
    }

    const permit = executionOwner.requestCancellation(
      conversationId,
      previewOwner.summary(conversationId)?.runId ?? null,
    )
    if (!permit) return
    setStreamCoarse({ cancelling: true })
    freezeCancelledRunLocally(conversationId)
    let succeeded = false
    let failure: unknown
    try {
      await chatApi.cancelStream(conversationId)
      succeeded = true
    } catch (err) {
      failure = err
    } finally {
      const stillCurrent = executionOwner.completeCancellation(permit, succeeded)
      if (stillCurrent) {
        if (!succeeded) {
          console.error('Failed to cancel chat stream:', failure)
          previewOwner.resume(conversationId)
          syncGeneratingConversationIds()
          setStreamErrorForConversation(
            conversationId,
            typeof failure === 'string' ? failure : (failure as Error | null | undefined)?.message || '停止生成失败',
          )
        }
        if (currentConversationIdRef.current === conversationId) setStreamCoarse({ cancelling: false })
      }
    }
  }, [executionOwner, freezeCancelledRunLocally, previewOwner, setStreamErrorForConversation, syncGeneratingConversationIds])

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

  const setSidebarCollapsedPersisted = useCallback((collapsed: boolean) => {
    const finish = measureChatSurface(
      'sidebar-collapse',
      document.querySelector('.chat-window-shell'),
      collapsed ? 'collapsed' : 'expanded',
    )
    setSidebarCollapsed(collapsed)
    rememberChatSidebarCollapsed(collapsed)
    if (typeof requestAnimationFrame === 'function') {
      requestAnimationFrame(() => requestAnimationFrame(finish))
    } else {
      finish()
    }
  }, [])

  const handleSidebarWidthChange = useCallback((nextWidth: number) => {
    setSidebarWidth(nextWidth)
    rememberSidebarWidth(nextWidth)
  }, [])

  useLayoutEffect(() => {
    const shell = document.querySelector('.chat-window-shell')
    if (shell instanceof HTMLElement) {
      shell.style.setProperty('--chat-sidebar-width', `${sidebarWidth}px`)
    }
  }, [sidebarWidth])

  // ---------- Right Dock ----------
  const [dockOpen, setDockOpen] = useState(() => getRememberedDockOpen())
  const [dockWidth, setDockWidth] = useState(() => getRememberedDockWidth())
  const [dockTab, setDockTab] = useState<DockTab>(() => getRememberedDockTab())
  const [dockWorkdir, setDockWorkdir] = useState('')
  const [treeExpanded, setTreeExpanded] = useState<string[]>([])
  const [dockReveal, setDockReveal] = useState<DockRevealRequest>(null)
  const [subAgentRequest, setSubAgentRequest] = useState<{ conversationId: string; agentId: string; nonce: number } | null>(null)
  const [dockPreview, setDockPreview] = useState<DockPreviewRequest>(null)
  // 工作目录跟随当前会话 / 选中项目 / agent runtime 变化，由后端 dock_resolve_cwd 解析
  // （外部 agent 与内置 runtime 的实际写入目录不同，runtime 切换必须重解析）。
  useEffect(() => {
    const conversationId = currentConversation?.id ?? null
    const projectId = selectedProject?.id ?? null
    if (!conversationId && !projectId) {
      setDockWorkdir('')
      return
    }
    let cancelled = false
    dockApi
      .resolveCwd(conversationId, projectId)
      .then((cwd) => {
        if (!cancelled) setDockWorkdir(cwd)
      })
      .catch(() => {
        if (!cancelled) setDockWorkdir('')
      })
    return () => {
      cancelled = true
    }
  }, [currentConversation?.id, selectedProject?.id, activeAgentRuntime.kind])

  useEffect(() => {
    const prev = skillProjectCwdRef.current
    skillProjectCwdRef.current = dockWorkdir
    if (prev !== dockWorkdir) void loadSkills()
  }, [dockWorkdir, loadSkills])

  // 文件树展开状态按 workdir 持久化，workdir 切换时重新载入。
  useEffect(() => {
    setTreeExpanded(dockWorkdir ? getRememberedTreeExpanded(dockWorkdir) : [])
  }, [dockWorkdir])

  const handleToggleDock = useCallback(() => {
    setDockOpen((prev) => {
      rememberDockOpen(!prev)
      return !prev
    })
  }, [])

  const handleCloseDock = useCallback(() => {
    setDockOpen(false)
    rememberDockOpen(false)
  }, [])

  // 输入栏 Git 胶囊「在 Git 面板中打开」：展开 Dock 并切到 Git 页。
  const handleOpenDockGit = useCallback(() => {
    setDockTab('git')
    rememberDockTab('git')
    setDockOpen(true)
    rememberDockOpen(true)
  }, [])

  // 标题栏后台任务状态灯：展开 Dock 并切到任务页。
  const handleOpenDockTasks = useCallback(() => {
    setDockTab('tasks')
    rememberDockTab('tasks')
    setDockOpen(true)
    rememberDockOpen(true)
  }, [])

  useEffect(() => onDockSubAgentRequest(target => {
    if (target.conversationId !== currentConversationIdRef.current) return
    handleOpenDockTasks()
    setSubAgentRequest(previous => ({ ...target, nonce: (previous?.nonce ?? 0) + 1 }))
  }), [handleOpenDockTasks])

  const handleDockWidthChange = useCallback((nextWidth: number) => {
    setDockWidth(nextWidth)
    rememberDockWidth(nextWidth)
  }, [])

  const handleDockTabChange = useCallback((tab: DockTab) => {
    setDockTab(tab)
    rememberDockTab(tab)
  }, [])

  const handleTreeExpandedChange = useCallback(
    (paths: string[]) => {
      setTreeExpanded(paths)
      if (dockWorkdir) rememberTreeExpanded(dockWorkdir, paths)
    },
    [dockWorkdir],
  )

  // Git 面板「在文件树中定位」：切到文件 tab 并展开定位。
  const handleDockRevealInTree = useCallback((path: string) => {
    setDockTab('files')
    rememberDockTab('files')
    setDockOpen(true)
    rememberDockOpen(true)
    setDockReveal((prev) => ({ path, nonce: (prev?.nonce ?? 0) + 1 }))
  }, [])

  // 工具卡片点文件名 → dock 查看器预览。workdir 内的路径同时在树里定位；
  // workdir 外的绝对路径（如写到桌面的文件）用其所在目录作查看器根。
  useEffect(
    () =>
      onDockPreviewRequest((rawPath) => {
        const normalize = (value: string) => value.replace(/\\/g, '/').replace(/^\/\/\?\//, '')
        const target = normalize(rawPath.trim())
        if (!target) return
        const wd = normalize(dockWorkdir)
        const isAbsolute = /^(?:[a-zA-Z]:)?\//.test(target)
        let request: { workdir: string; path: string } | null = null
        let revealRel: string | null = null
        if (isAbsolute) {
          if (wd && target.toLowerCase().startsWith(`${wd.toLowerCase()}/`)) {
            revealRel = target.slice(wd.length + 1)
            request = { workdir: dockWorkdir, path: revealRel }
          } else {
            const idx = target.lastIndexOf('/')
            if (idx > 0) request = { workdir: target.slice(0, idx), path: target.slice(idx + 1) }
          }
        } else if (dockWorkdir) {
          revealRel = target.replace(/^\.\//, '')
          request = { workdir: dockWorkdir, path: revealRel }
        }
        if (!request) return
        setDockTab('files')
        rememberDockTab('files')
        setDockOpen(true)
        rememberDockOpen(true)
        if (revealRel) setDockReveal((prev) => ({ path: revealRel, nonce: (prev?.nonce ?? 0) + 1 }))
        const next = request
        setDockPreview((prev) => ({ kind: 'file', ...next, nonce: (prev?.nonce ?? 0) + 1 }))
      }),
    [dockWorkdir],
  )

  // 工具卡片点 +N -N 徽标 → dock 侧栏渲染整份带色 diff。
  useEffect(
    () =>
      onDockDiffPreviewRequest((payload) => {
        setDockTab('files')
        rememberDockTab('files')
        setDockOpen(true)
        rememberDockOpen(true)
        setDockPreview((prev) => ({ kind: 'diff', ...payload, nonce: (prev?.nonce ?? 0) + 1 }))
      }),
    [],
  )

  // claude 交计划（ExitPlanMode）→ dock 侧栏渲染整份计划。审批卡里那块 `max-h-40` 的
  // 灰框只够扫一眼，而「批不批这个计划」是要读完才能决定的。
  useEffect(
    () =>
      onDockMarkdownPreviewRequest((payload) => {
        setDockTab('files')
        rememberDockTab('files')
        setDockOpen(true)
        rememberDockOpen(true)
        setDockPreview((prev) => ({ kind: 'markdown', ...payload, nonce: (prev?.nonce ?? 0) + 1 }))
      }),
    [],
  )

  // 文件树「插入 @ 引用」：经 composerInsert 文本信道注入输入框正文。
  const handleInsertFileMention = useCallback((path: string) => {
    insertTextIntoComposer(`@${path} `)
  }, [])

  const handleCollapseSidebar = useCallback(() => {
    setSidebarCollapsedPersisted(true)
  }, [setSidebarCollapsedPersisted])

  useEffect(() => {
    if (!isTauriRuntime()) return
    let cancelled = false
    void (async () => {
      const { getCurrentWindow } = await import('@tauri-apps/api/window')
      const { LogicalSize } = await import('@tauri-apps/api/dpi')
      const min = sidebarCollapsed
        ? CHAT_MIN_SIZE_COLLAPSED
        : {
            width: CHAT_MIN_SIZE_COLLAPSED.width + sidebarWidth,
            height: CHAT_MIN_SIZE_COLLAPSED.height,
          }
      const win = getCurrentWindow()
      // 最大化/全屏时不要动 min-size 或 size：Windows 上 setMinSize 会触发重排、把最大化状态取消掉
      // （表现为切换侧边栏后窗口退出最大化）。尺寸约束对铺满屏幕的窗口也没意义，等恢复到可调窗口再应用。
      if ((await win.isMaximized()) || (await win.isFullscreen())) return
      if (cancelled) return
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
  }, [sidebarCollapsed, sidebarWidth])

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
      if (popoutConversationIdsRef.current.has(id)) {
        void chatApi.focusConversationPopout(id)
        occupyConversationInMain(id, conversation ?? currentConversationRef.current)
        return
      }
      void handleSelectConversation(id, {
        messageCount: conversation?.message_count,
        focusMessageId: focusMessageId || undefined,
      })
    }, { restoreCurrentRoute: false })
  }, [handleSelectConversation, occupyConversationInMain, runAfterLeavingSettings])

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
    setOptimisticSidebarConversations((prev) => {
      const next = prev.filter((item) => generatingConversationIdsRef.current.has(item.id))
      return next.length === prev.length ? prev : next
    })
  }, [])

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
      await chatApi.openConversationPopout(conversationId)
      addPopoutId(conversationId)
      if (currentConversationIdRef.current === conversationId) {
        occupyConversationInMain(conversationId, currentConversationRef.current)
      }
    } catch (err) {
      const message = typeof err === 'string' ? err : (err as Error).message || i18n[uiLang].chatPopoutLimit
      setPopoutNotice(message)
    }
  }, [addPopoutId, occupyConversationInMain, uiLang])

  // 会话页顶栏控件。非 mac 渲染进全宽标题栏带（单行 chrome），mac 仍留在主区 52px 顶栏。
  // 抽成变量而非组件：依赖十余个 Chat 局部状态与回调，拆组件只会换来一长串 props。
  const conversationTitlebarControls = useMemo(() => (
    <>
      <div className="flex min-w-0 items-center gap-1">
        <div className="shrink-0" data-tauri-drag-region="false">
          <RuntimePicker
            agentRuntime={activeAgentRuntime}
            onRuntimeChange={handleRuntimeChange}
            conversationId={currentConversation?.id}
            locked={
              // 一 agent 一对话：有消息后锁死 kind/agent（内置 Kivio 与本地 CLI 一律）。
              // 拉出独立窗口后主窗卸掉了消息，仍按「已有对话」锁死。
              !!currentConversation && (
                (currentConversation.messages?.length ?? 0) > 0
                || popoutConversationIds.has(currentConversation.id)
              )
            }
          />
        </div>
        <div className="min-w-0 max-w-full shrink" data-tauri-drag-region="false">
          {usesExternalRuntime ? (
            <ExternalModelSelector
              agentRuntime={activeAgentRuntime}
              onModelChange={handleExternalModelChange}
              conversationId={currentConversation?.id}
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
          <div className="shrink-0 chat-thinking-pill-wrap" data-tauri-drag-region="false">
            <ThinkingLevelSelector
              currentProviderId={activeProviderId}
              currentModel={activeModel}
              value={
                currentConversation
                  ? (currentConversation.thinking_level
                      ?? currentConversation.thinkingLevel
                      ?? draftThinkingLevel)
                  : draftThinkingLevel
              }
              onChange={handleThinkingLevelChange}
            />
          </div>
        )}
        <div className="shrink-0" data-tauri-drag-region="false">
          <PermissionPicker
            agentRuntime={activeAgentRuntime}
            approvalPolicy={approvalPolicy}
            onApprovalPolicyChange={handleApprovalPolicyChange}
          />
        </div>
        {!usesChatRuntime && (
          <div className="shrink-0" data-tauri-drag-region="false">
            <BackgroundJobsIndicator
              conversationId={currentConversation?.id ?? null}
              onOpen={handleOpenDockTasks}
            />
          </div>
        )}
      </div>
      <div className="min-w-5 flex-1" data-tauri-drag-region />
      <div className="flex min-w-0 shrink items-center justify-end gap-1">
        {currentConversation && (
          <div className="shrink-0" data-tauri-drag-region="false">
            <IconButton
              label={i18n[uiLang].chatOpenInNewWindow}
              size="sm"
              variant="ghost"
              onClick={() => void handleOpenConversationPopout(currentConversation.id)}
            >
              <SquareArrowOutUpRight size={15} />
            </IconButton>
          </div>
        )}
        {!usesChatRuntime && (
          <div className="shrink-0" data-tauri-drag-region="false">
            <IconButton
              label={i18n[uiLang].dockToggle}
              size="sm"
              variant="ghost"
              className={dockOpen ? 'bg-black/5 text-neutral-800 dark:bg-white/10 dark:text-neutral-100' : ''}
              onClick={handleToggleDock}
            >
              <PanelRight size={15} />
            </IconButton>
          </div>
        )}
      </div>
    </>
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
    (pendingToolConfirm || pendingSessionConsent || pendingUserPrompt) ? (
    <div className="shrink-0 px-6">
      <div className="mx-auto w-full max-w-4xl">
        {pendingUserPrompt && pendingUserPromptRecord && (
          <AskUserBlock
            variant="docked"
            toolCall={pendingUserPromptRecord}
            onResolved={() => dismissPendingUserPrompt(
              pendingUserPrompt.conversationId,
              pendingUserPrompt.toolCallId,
            )}
          />
        )}
        {pendingToolConfirm && (
          <ApprovalCard
            title={toolApprovalTitle(pendingToolConfirm)}
            subtitle={`${pendingToolConfirm.source}${pendingToolConfirm.serverId ? ` · ${pendingToolConfirm.serverId}` : ''}`}
            detail={pendingToolConfirm.argumentsPreview}
            error={toolConfirmError}
            actions={isPlanApproval(pendingToolConfirm)
              ? [
                {
                  label: '拒绝 / 让它改',
                  disabled: toolConfirmSubmittingId === pendingToolConfirm.toolCallId,
                  onSelect: () => { void resolvePendingToolConfirm(false) },
                },
                ...PLAN_APPROVAL_ACTIONS.map((action, index) => ({
                  label: action.label,
                  primary: index === PLAN_APPROVAL_ACTIONS.length - 1,
                  hint: index === PLAN_APPROVAL_ACTIONS.length - 1 ? 'Ctrl+↵' : undefined,
                  disabled: toolConfirmSubmittingId === pendingToolConfirm.toolCallId,
                  onSelect: () => {
                    void resolvePendingToolConfirm(true, false, action.mode).then((accepted) => {
                      if (accepted) {
                        return persistApprovedExternalSandbox(
                          pendingToolConfirm.conversationId,
                          activeAgentRuntime,
                          action.mode,
                        )
                      }
                    })
                  },
                })),
              ]
              : isEnterPlanApproval(pendingToolConfirm)
                ? [
                  {
                    label: '不用，直接做',
                    disabled: toolConfirmSubmittingId === pendingToolConfirm.toolCallId,
                    onSelect: () => { void resolvePendingToolConfirm(false) },
                  },
                  {
                    label: '总是允许',
                    disabled: toolConfirmSubmittingId === pendingToolConfirm.toolCallId,
                    onSelect: () => {
                      void resolvePendingToolConfirm(true, true).then((accepted) => {
                        if (accepted) {
                          return persistApprovedExternalSandbox(
                            pendingToolConfirm.conversationId,
                            activeAgentRuntime,
                            'plan',
                          )
                        }
                      })
                    },
                  },
                  {
                    label: '进入计划模式',
                    primary: true,
                    hint: 'Ctrl+↵',
                    disabled: toolConfirmSubmittingId === pendingToolConfirm.toolCallId,
                    onSelect: () => {
                      void resolvePendingToolConfirm(true).then((accepted) => {
                        if (accepted) {
                          return persistApprovedExternalSandbox(
                            pendingToolConfirm.conversationId,
                            activeAgentRuntime,
                            'plan',
                          )
                        }
                      })
                    },
                  },
                ]
                : [
                  {
                    label: '拒绝',
                    disabled: toolConfirmSubmittingId === pendingToolConfirm.toolCallId,
                    onSelect: () => { void resolvePendingToolConfirm(false) },
                  },
                  {
                    label: '总是允许',
                    disabled: toolConfirmSubmittingId === pendingToolConfirm.toolCallId,
                    onSelect: () => { void resolvePendingToolConfirm(true, true) },
                  },
                  {
                    label: '允许一次',
                    primary: true,
                    hint: 'Ctrl+↵',
                    disabled: toolConfirmSubmittingId === pendingToolConfirm.toolCallId,
                    onSelect: () => { void resolvePendingToolConfirm(true) },
                  },
                ]}
          />
        )}
        {pendingSessionConsent && (
          <ApprovalCard
            title="允许本次会话使用文件和命令工具？"
            subtitle="授权后，本会话内 Kivio 可读写、删除磁盘上的任意文件并执行任意终端命令（包括项目目录之外）。仅本次会话有效，重启后需重新授权。"
            error={sessionConsentError}
            actions={[
              {
                label: '拒绝',
                disabled: sessionConsentSubmittingConversationId === pendingSessionConsent.conversationId,
                onSelect: () => { void resolvePendingSessionConsent(false) },
              },
              {
                label: '允许本次会话',
                primary: true,
                hint: 'Ctrl+↵',
                disabled: sessionConsentSubmittingConversationId === pendingSessionConsent.conversationId,
                onSelect: () => { void resolvePendingSessionConsent(true) },
              },
            ]}
          />
        )}
      </div>
    </div>
    ) : null
  ), [
    activeAgentRuntime,
    dismissPendingUserPrompt,
    pendingSessionConsent,
    pendingToolConfirm,
    pendingUserPrompt,
    pendingUserPromptRecord,
    persistApprovedExternalSandbox,
    resolvePendingSessionConsent,
    resolvePendingToolConfirm,
    sessionConsentError,
    sessionConsentSubmittingConversationId,
    toolConfirmError,
    toolConfirmSubmittingId,
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
            sessionLibrary={{
              currentConversationId: currentConversation?.id,
              generatingConversationIds,
              onSelectConversation: handleSidebarSelectConversation,
              onConversationDeleted: handleSidebarConversationDeleted,
              onForceDropConversation: handleSidebarForceDropConversation,
              onConversationsChanged: refreshSidebar,
            }}
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
              if (currentConversation?.id) void chatApi.closeConversationPopout(currentConversation.id)
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
            subAgentRequest={subAgentRequest}
            open={dockOpen}
            width={dockWidth}
            activeTab={dockTab}
            workdir={dockWorkdir}
            lang={uiLang}
            conversationId={currentConversation?.id ?? null}
            treeExpanded={treeExpanded}
            revealRequest={dockReveal}
            previewRequest={dockPreview}
            onToggleTab={handleDockTabChange}
            onWidthChange={handleDockWidthChange}
            onClose={handleCloseDock}
            onTreeExpandedChange={handleTreeExpandedChange}
            onInsertMention={handleInsertFileMention}
            onRevealInTree={handleDockRevealInTree}
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
