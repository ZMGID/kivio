import { useCallback, useEffect, useRef, type Dispatch, type MutableRefObject, type SetStateAction } from 'react'
import {
  conversationHash,
  getRouteConversationId,
  hashPath,
  isChatAssistantCenterPath,
  isChatAutomationsPath,
  isChatSchedulesPath,
  isChatKnowledgeCenterPath,
  isChatMcpCenterPath,
  isChatNotesPath,
  isChatArtifactsPath,
  isChatMediaPath,
  isChatOnboardingRoute,
  isChatPluginCenterPath,
  isChatSessionCenterPath,
  isChatSettingsPath,
  isChatSkillCenterPath,
  setHash,
  type ChatExtensionsNavItem,
} from '../chatRoutes'
import { getRememberedTasksTab, rememberTasksTab } from '../persistence'

import type { ChatView } from '../routeCodec'
import { pathFromHash } from '../routeCodec'

interface UseChatRoutingParams {
  onViewChange: (view: ChatView) => void
  /** 会话路由命中且需要加载时调用（已是当前会话则不会触发，见 loadFromRoute 注释）。 */
  onLoadConversation: (conversationId: string) => void
  /** 路由指向空会话时的重置动作。 */
  onResetConversation: () => void
  /** 进入非会话目标时立即使旧会话导航代次失效；不取消后台运行。 */
  onLeaveConversation: () => void
  /** 读当前会话 id，用于跳过「刚 apply 完又被路由重载一遍」的双读。 */
  currentConversationIdRef: React.MutableRefObject<string | null>
  /** 旧 `#chat/sessions` 入口：对话库已迁入设置，重定向到设置 → 对话库。 */
  onOpenSessionsSettings?: () => void
  /** 设置页初始 tab；openEmbeddedSettings 写入。 */
  setSettingsInitialTab: (tab: SettingsOpenTab) => void
  /** 扩展 nav 选中项；openExtensionsItem 写入。 */
  setExtensionsNavItem: Dispatch<SetStateAction<ChatExtensionsNavItem | null>>
  /** Scheduled editor permission; checked before hash listeners can replace or unmount Chat. */
  tasksLeaveGuardRef?: MutableRefObject<(() => Promise<boolean>) | null>
  requestTasksLeave?: () => Promise<boolean>
}

type SettingsOpenTab = 'chat' | 'connectors' | 'sessions' | 'usage'

/**
 * 聊天窗口的 hash 路由。
 *
 * 对外写 view / 会话加载，以及中心页 opener（setView + syncXxxRoute）。
 * 不持有自己的 state；settings tab 与扩展 nav 的 setter 由页面注入。
 *
 * 时序保持与搬迁前一致：挂载时立刻 loadFromRoute() 一次，再订阅 hashchange。
 */
export function useChatRouting({
  onViewChange,
  onLoadConversation,
  onResetConversation,
  onLeaveConversation,
  currentConversationIdRef,
  onOpenSessionsSettings,
  setSettingsInitialTab,
  setExtensionsNavItem,
  tasksLeaveGuardRef,
  requestTasksLeave,
}: UseChatRoutingParams) {
  const acceptedHashRef = useRef(window.location.hash)
  const syncConversationRoute = useCallback((conversationId: string | null) => {
    if (!conversationId) onLeaveConversation()
    setHash(conversationHash(conversationId))
  }, [onLeaveConversation])

  const syncNonConversationRoute = useCallback((hash: string) => {
    onLeaveConversation()
    setHash(hash)
  }, [onLeaveConversation])
  const syncSettingsRoute = useCallback(() => syncNonConversationRoute('#chat/settings'), [syncNonConversationRoute])
  const syncOnboardingRoute = useCallback(() => syncNonConversationRoute('#chat/onboarding'), [syncNonConversationRoute])
  const syncAssistantCenterRoute = useCallback(() => syncNonConversationRoute('#chat/assistants'), [syncNonConversationRoute])
  const syncSkillCenterRoute = useCallback(() => syncNonConversationRoute('#chat/skill'), [syncNonConversationRoute])
  const syncMcpCenterRoute = useCallback(() => syncNonConversationRoute('#chat/mcp'), [syncNonConversationRoute])
  const syncKnowledgeCenterRoute = useCallback(() => syncNonConversationRoute('#chat/knowledge'), [syncNonConversationRoute])
  const syncNotesRoute = useCallback(() => syncNonConversationRoute('#chat/notes'), [syncNonConversationRoute])

  useEffect(() => {
    const loadFromRoute = () => {
      acceptedHashRef.current = window.location.hash
      const path = hashPath()
      if (isChatOnboardingRoute(path)) {
        onLeaveConversation()
        onViewChange('onboarding')
        return
      }
      if (isChatSettingsPath(path)) {
        onLeaveConversation()
        onViewChange('settings')
        return
      }
      if (isChatAssistantCenterPath(path)) {
        onLeaveConversation()
        onViewChange('assistants')
        return
      }
      if (isChatSkillCenterPath(path)) {
        onLeaveConversation()
        onViewChange('skill')
        return
      }
      if (isChatMcpCenterPath(path)) {
        onLeaveConversation()
        onViewChange('mcp')
        return
      }
      if (isChatKnowledgeCenterPath(path)) {
        onLeaveConversation()
        onViewChange('knowledge')
        return
      }
      if (isChatNotesPath(path)) {
        onLeaveConversation()
        onViewChange('notes')
        return
      }
      if (isChatMediaPath(path)) {
        onLeaveConversation()
        onViewChange('media')
        return
      }
      if (isChatArtifactsPath(path)) {
        onLeaveConversation()
        onViewChange('artifacts')
        return
      }
      if (isChatAutomationsPath(path)) {
        rememberTasksTab('automations')
        onLeaveConversation()
        onViewChange('automations')
        return
      }
      if (isChatSchedulesPath(path)) {
        rememberTasksTab('schedules')
        onLeaveConversation()
        onViewChange('schedules')
        return
      }
      // 对话库已迁入设置；旧链接 `#chat/sessions` 重定向
      if (isChatSessionCenterPath(path)) {
        onLeaveConversation()
        onOpenSessionsSettings?.()
        return
      }
      if (isChatPluginCenterPath(path)) {
        onLeaveConversation()
        onViewChange('plugins')
        return
      }
      const conversationId = getRouteConversationId()
      if (!conversationId) {
        onLeaveConversation()
        onViewChange('conversation')
        onResetConversation()
        return
      }
      onViewChange('conversation')
      // 已是当前会话：说明这次 hash 变化来自点击/创建/分支等「先加载并 apply、再同步路由」的
      // 路径，数据刚落进 state，此处再 force 重载只会让同一对话白读一遍盘（双重 IPC）。
      // 真正的路由导航（前进/后退/启动恢复/外部改 hash）ref 必然不同，照常加载。
      if (currentConversationIdRef.current === conversationId) return
      onLoadConversation(conversationId)
    }
    loadFromRoute()
    let disposed = false
    let pendingTasksHash: string | null = null
    const handleHashChange = (event: HashChangeEvent) => {
      const previousHash = acceptedHashRef.current
      const nextHash = window.location.hash
      if (nextHash === previousHash && pendingTasksHash !== null) {
        event.stopImmediatePropagation()
        return
      }
      if (
        tasksLeaveGuardRef?.current
        && requestTasksLeave
        && isChatSchedulesPath(pathFromHash(previousHash))
        && !isChatSchedulesPath(pathFromHash(nextHash))
      ) {
        // Capture runs before App's mode listener. Keep the editor mounted while
        // confirmation is pending, including browser back and external hashes.
        event.stopImmediatePropagation()
        window.history.replaceState(window.history.state, '', previousHash || window.location.pathname + window.location.search)
        const alreadyPending = pendingTasksHash !== null
        pendingTasksHash = nextHash
        if (alreadyPending) return
        void requestTasksLeave().then((allowed) => {
          if (disposed) return
          const targetHash = pendingTasksHash
          pendingTasksHash = null
          if (!allowed || targetHash === null) return
          window.history.replaceState(window.history.state, '', targetHash || window.location.pathname + window.location.search)
          acceptedHashRef.current = targetHash
          // replaceState has no hashchange; notify the existing App route owner
          // only after permission, without re-entering this leave decision.
          window.dispatchEvent(new HashChangeEvent('hashchange', {
            oldURL: new URL(previousHash, window.location.href).href,
            newURL: window.location.href,
          }))
        })
        return
      }
      loadFromRoute()
    }
    window.addEventListener('hashchange', handleHashChange, true)
    return () => {
      disposed = true
      window.removeEventListener('hashchange', handleHashChange, true)
    }
  }, [
    currentConversationIdRef,
    onLoadConversation,
    onLeaveConversation,
    onOpenSessionsSettings,
    onResetConversation,
    onViewChange,
    requestTasksLeave,
    tasksLeaveGuardRef,
  ])

  const openEmbeddedSettings = useCallback((tab: SettingsOpenTab = 'chat') => {
    setSettingsInitialTab(tab)
    onViewChange('settings')
    syncSettingsRoute()
  }, [onViewChange, setSettingsInitialTab, syncSettingsRoute])

  const openChatSettings = useCallback(() => {
    openEmbeddedSettings('chat')
  }, [openEmbeddedSettings])

  const openAssistantCenter = useCallback(() => {
    onViewChange('assistants')
    syncAssistantCenterRoute()
  }, [onViewChange, syncAssistantCenterRoute])

  const openSkillCenter = useCallback(() => {
    onViewChange('skill')
    syncSkillCenterRoute()
  }, [onViewChange, syncSkillCenterRoute])

  const openMcpCenter = useCallback(() => {
    onViewChange('mcp')
    syncMcpCenterRoute()
  }, [onViewChange, syncMcpCenterRoute])

  const openKnowledgeCenter = useCallback(() => {
    onViewChange('knowledge')
    syncKnowledgeCenterRoute()
  }, [onViewChange, syncKnowledgeCenterRoute])

  const openNotesCenter = useCallback(() => {
    onViewChange('notes')
    syncNotesRoute()
  }, [onViewChange, syncNotesRoute])

  const openTasksCenter = useCallback(() => {
    const tab = getRememberedTasksTab()
    onViewChange(tab)
    syncNonConversationRoute(`#chat/${tab}`)
  }, [onViewChange, syncNonConversationRoute])

  const openExtensionsItem = useCallback((item: ChatExtensionsNavItem) => {
    setExtensionsNavItem(item)
    if (item === 'tasks') {
      openTasksCenter()
      return
    }
    if (item === 'media') {
      onViewChange('media')
      syncNonConversationRoute('#chat/media')
      return
    }
    if (item === 'artifacts') {
      onViewChange('artifacts')
      syncNonConversationRoute('#chat/artifacts')
      return
    }
    if (item === 'plugins') {
      onViewChange('plugins')
      syncNonConversationRoute('#chat/plugins')
      return
    }
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
  }, [
    openAssistantCenter, openSkillCenter, openMcpCenter, openKnowledgeCenter,
    openNotesCenter, openTasksCenter, setExtensionsNavItem, onViewChange, syncNonConversationRoute,
  ])

  return {
    syncConversationRoute,
    syncSettingsRoute,
    syncOnboardingRoute,
    syncAssistantCenterRoute,
    syncSkillCenterRoute,
    syncMcpCenterRoute,
    syncKnowledgeCenterRoute,
    syncNotesRoute,
    openEmbeddedSettings,
    openChatSettings,
    openAssistantCenter,
    openSkillCenter,
    openMcpCenter,
    openKnowledgeCenter,
    openNotesCenter,
    openTasksCenter,
    openExtensionsItem,
  }
}
