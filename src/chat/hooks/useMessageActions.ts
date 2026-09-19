import { useCallback, type Dispatch, type MutableRefObject, type SetStateAction } from 'react'
import { api } from '../../api/tauri'
import { chatApi } from '../api'
import type { createChatNavigationController } from '../chatNavigationController'
import { insertTextIntoComposer } from '../composerInsert'
import type { AssistantStreamStats } from '../MessageList'
import { setCoarse as setStreamCoarse } from '../streamingStore'
import type { Conversation } from '../types'

type ChatNavigationController = ReturnType<typeof createChatNavigationController>

export interface UseMessageActionsOptions {
  /** 读最新会话用 ref：消息操作 handler 身份不随会话对象换引用而变（否则打穿气泡 memo）。 */
  currentConversationRef: MutableRefObject<Conversation | null>
  navigation: Pick<
    ChatNavigationController,
    'beginConversationCreation' | 'isConversationCreationCurrent' | 'commitCreatedConversation'
  >
  applyConversationIfCurrent: (expectedId: string, conversation: Conversation) => boolean
  applyConversationMeta: (updated: Conversation) => void
  setStreamErrorForConversation: (conversationId: string, message: string) => void
  setAssistantStreamStatsByMessageId: Dispatch<SetStateAction<Record<string, AssistantStreamStats>>>
  refreshSidebar: () => void
  refreshContextStats: (conversationId: string) => Promise<void>
}

function errorMessage(err: unknown, fallback: string): string {
  return typeof err === 'string' ? err : (err as Error).message || fallback
}

/** 只改当前视图的错误条，不写 per-conversation 记录（沿用原 `setStreamError(useState)` 语义）。 */
function setStreamError(error: string): void {
  setStreamCoarse({ streamError: error })
}

/** 消息第一行去掉标题井号 / 强调标记后截 40 字作笔记标题；空则回落默认名。 */
export function noteTitleFromContent(content: string, fallback = '对话笔记'): string {
  const firstLine = content
    .split('\n')
    .map((line) => line.trim())
    .find((line) => line.length > 0)
  if (!firstLine) return fallback
  return firstLine
    .replace(/^#+\s*/, '')
    .replace(/\*\*|__|\*|_|`>/g, '')
    .slice(0, 40)
    .trim() || fallback
}

/** 单条消息上的操作：编辑 / 删除 / 回到这里 / 建分支 / 存笔记 / 多答组选中。 */
export function useMessageActions({
  currentConversationRef,
  navigation,
  applyConversationIfCurrent,
  applyConversationMeta,
  setStreamErrorForConversation,
  setAssistantStreamStatsByMessageId,
  refreshSidebar,
  refreshContextStats,
}: UseMessageActionsOptions) {
  const updateMessage = useCallback(async (messageId: string, content: string) => {
    const conv = currentConversationRef.current
    if (!conv) return
    try {
      const updated = await chatApi.updateMessage(conv.id, messageId, content)
      applyConversationIfCurrent(conv.id, updated)
      refreshSidebar()
    } catch (err) {
      console.error('Failed to update message:', err)
      setStreamErrorForConversation(conv.id, errorMessage(err, '保存失败'))
    }
  }, [applyConversationIfCurrent, currentConversationRef, refreshSidebar, setStreamErrorForConversation])

  const deleteMessage = useCallback(async (messageId: string) => {
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
      setStreamErrorForConversation(conv.id, errorMessage(err, '删除失败'))
    }
  }, [
    applyConversationIfCurrent, currentConversationRef, refreshSidebar,
    setAssistantStreamStatsByMessageId, setStreamErrorForConversation,
  ])

  // 一键 rewind（「回到这里」）：截掉这条提问及其之后的所有消息，原文塞回输入框，用户改完再自己发。
  // 破坏性且不可撤销 → 先 confirm（与删除消息同一把关）。
  const rewindToMessage = useCallback(async (messageId: string) => {
    const conv = currentConversationRef.current
    if (!conv) return
    if (!window.confirm('回到这里？这条提问及其之后的所有消息会被删除，原文放回输入框。')) return
    try {
      const { conversation, content } = await chatApi.rewindToMessage(conv.id, messageId)
      if (applyConversationIfCurrent(conv.id, conversation)) {
        setAssistantStreamStatsByMessageId({})
        setStreamError('')
        insertTextIntoComposer(content)
      }
      refreshSidebar()
      // 上下文用量后台补算（后端 rewind 故意不算，见那边注释）：几秒的 MCP 列表不该挡住 UI。
      void refreshContextStats(conversation.id)
    } catch (err) {
      console.error('Failed to rewind conversation:', err)
      setStreamErrorForConversation(conv.id, errorMessage(err, '回到这里失败'))
    }
  }, [
    applyConversationIfCurrent, currentConversationRef, refreshContextStats, refreshSidebar,
    setAssistantStreamStatsByMessageId, setStreamErrorForConversation,
  ])

  // 对话分支：把该消息及之前的消息复制进新对话，立即打开新对话（不自动发送）。源对话只读、不受影响。
  const forkAtMessage = useCallback(async (messageId: string) => {
    const conv = currentConversationRef.current
    if (!conv) return
    const creation = navigation.beginConversationCreation()
    try {
      const forked = await chatApi.forkConversation(conv.id, messageId)
      refreshSidebar()
      if (navigation.isConversationCreationCurrent(creation)) {
        setAssistantStreamStatsByMessageId({})
        navigation.commitCreatedConversation(creation, forked)
        setStreamError('')
      }
    } catch (err) {
      console.error('Failed to fork conversation:', err)
      setStreamErrorForConversation(conv.id, errorMessage(err, '建分支失败'))
    }
  }, [currentConversationRef, navigation, refreshSidebar, setAssistantStreamStatsByMessageId, setStreamErrorForConversation])

  const saveMessageToNote = useCallback(async (messageId: string): Promise<boolean> => {
    const conv = currentConversationRef.current
    if (!conv) return false
    const content = conv.messages.find((m) => m.id === messageId)?.content?.trim() || ''
    if (!content) return false
    try {
      await api.notesCreate(noteTitleFromContent(content), content, '', 'chat')
      setStreamError('')
      return true
    } catch (err) {
      console.error('Failed to save message to note:', err)
      setStreamError(err instanceof Error ? err.message : String(err) || '存为笔记失败')
      return false
    }
  }, [currentConversationRef])

  // 多答组「选中条」：标记某组进下一轮历史的列。默认第一列；用户点选改。
  const setGroupSelection = useCallback(async (groupId: string, messageId: string) => {
    const conv = currentConversationRef.current
    if (!conv) return
    try {
      const updated = await chatApi.setGroupSelection(conv.id, groupId, messageId)
      applyConversationMeta(updated)
    } catch (err) {
      console.error('Failed to set group selection:', err)
      setStreamErrorForConversation(conv.id, errorMessage(err, '选中失败'))
    }
  }, [applyConversationMeta, currentConversationRef, setStreamErrorForConversation])

  return { updateMessage, deleteMessage, rewindToMessage, forkAtMessage, saveMessageToNote, setGroupSelection }
}
