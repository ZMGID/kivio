import { useCallback, useEffect, useRef, useState, type Dispatch, type MutableRefObject, type RefObject, type SetStateAction } from 'react'
import type { SettingsShellHandle } from '../../settings/public/shell'

/** 退场下滑动画时长，与 Settings 入场容器的 CSS 对齐。 */
const SETTINGS_EXIT_MS = 220

type ChatView = import('../routeCodec').ChatView

interface PendingSettingsAction {
  action: () => void
  /** 目标动作会自行写路由时，不先恢复旧会话路由。 */
  restoreCurrentRoute: boolean
}

export interface UseSettingsExitOptions {
  chatView: ChatView
  setChatView: Dispatch<SetStateAction<ChatView>>
  settingsRef: RefObject<SettingsShellHandle | null>
  currentConversationIdRef: MutableRefObject<string | null>
  syncConversationRoute: (conversationId: string | null) => void
  /** 从技能 / MCP / 专家 / 知识库 / 设置回到会话时刷新技能与工具指示器。 */
  onReturnedToConversation: () => void
}

/**
 * 设置页退场与「先关设置再导航」的排队。settingsRef 仍归页面（传给 SettingsShell）。
 */
export function useSettingsExit({
  chatView,
  setChatView,
  settingsRef,
  currentConversationIdRef,
  syncConversationRoute,
  onReturnedToConversation,
}: UseSettingsExitOptions) {
  const [settingsExiting, setSettingsExiting] = useState(false)
  const pendingAfterSettingsCloseRef = useRef<PendingSettingsAction | null>(null)
  const exitTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const prevChatViewRef = useRef(chatView)

  const finishExit = useCallback(() => {
    const pending = pendingAfterSettingsCloseRef.current
    pendingAfterSettingsCloseRef.current = null
    setSettingsExiting(false)
    setChatView('conversation')
    if (!pending || pending.restoreCurrentRoute) {
      syncConversationRoute(currentConversationIdRef.current)
    }
    pending?.action()
  }, [currentConversationIdRef, setChatView, syncConversationRoute])

  const closeSettings = useCallback(() => {
    if (prevChatViewRef.current !== 'settings' || exitTimerRef.current !== null) return
    // 保存确认后只启动一次退场；保活页面仅切换可见性。
    setSettingsExiting(true)
    exitTimerRef.current = setTimeout(() => {
      exitTimerRef.current = null
      finishExit()
    }, SETTINGS_EXIT_MS)
  }, [finishExit])

  useEffect(() => () => {
    if (exitTimerRef.current !== null) clearTimeout(exitTimerRef.current)
    exitTimerRef.current = null
    pendingAfterSettingsCloseRef.current = null
  }, [])

  // 中心页（技能/MCP/专家）没有自己的返回按钮，离开靠侧栏选会话/新建等任意路径。
  // 统一在「回到会话视图」这个转变点刷新技能列表与工具指示器，
  // 保证中心页里的启停/增删在回到聊天后立即生效（替代原各页 onClose 的刷新职责）。
  useEffect(() => {
    const prev = prevChatViewRef.current
    prevChatViewRef.current = chatView
    if (chatView !== 'settings') {
      if (exitTimerRef.current !== null) clearTimeout(exitTimerRef.current)
      exitTimerRef.current = null
      pendingAfterSettingsCloseRef.current = null
      setSettingsExiting(false)
    }
    if (chatView !== 'conversation' || prev === chatView) return
    if (prev === 'skill' || prev === 'mcp' || prev === 'assistants' || prev === 'knowledge' || prev === 'settings') {
      onReturnedToConversation()
    }
  }, [chatView, onReturnedToConversation])

  const runAfterLeavingSettings = useCallback((
    action: () => void,
    options?: { restoreCurrentRoute?: boolean },
  ) => {
    if (chatView !== 'settings') {
      action()
      return
    }
    pendingAfterSettingsCloseRef.current = {
      action,
      restoreCurrentRoute: options?.restoreCurrentRoute ?? true,
    }
    // The queued navigation fires only from SettingsShell.onClose after its
    // canonical draft flush succeeds. On failure, keep settings and the action
    // in place so the user can repair/retry instead of losing the draft.
    if (settingsRef.current) settingsRef.current.requestClose()
    else finishExit()
  }, [chatView, finishExit, settingsRef])

  return { settingsExiting, closeSettings, runAfterLeavingSettings }
}
