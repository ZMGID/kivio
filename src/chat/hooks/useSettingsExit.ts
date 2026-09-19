import { useCallback, useEffect, useRef, useState, type Dispatch, type MutableRefObject, type RefObject, type SetStateAction } from 'react'
import type { SettingsShellHandle } from '../../settings/public/shell'
import { completeSettingsExit, type PendingSettingsAction } from '../settingsExit'

/** 退场下滑动画时长，与 Settings 入场容器的 CSS 对齐。 */
const SETTINGS_EXIT_MS = 220

type ChatView =
  | 'conversation' | 'settings' | 'assistants' | 'skill'
  | 'mcp' | 'knowledge' | 'notes' | 'automations' | 'onboarding'

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

  const closeSettings = useCallback(() => {
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
      onReturnedToConversation()
    }, SETTINGS_EXIT_MS)
  }, [currentConversationIdRef, onReturnedToConversation, setChatView, syncConversationRoute])

  // 中心页（技能/MCP/专家）没有自己的返回按钮，离开靠侧栏选会话/新建等任意路径。
  // 统一在「回到会话视图」这个转变点刷新技能列表与工具指示器，
  // 保证中心页里的启停/增删在回到聊天后立即生效（替代原各页 onClose 的刷新职责）。
  const prevChatViewRef = useRef(chatView)
  useEffect(() => {
    const prev = prevChatViewRef.current
    prevChatViewRef.current = chatView
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
  }, [chatView, currentConversationIdRef, setChatView, settingsRef, syncConversationRoute])

  return { settingsExiting, closeSettings, runAfterLeavingSettings }
}
