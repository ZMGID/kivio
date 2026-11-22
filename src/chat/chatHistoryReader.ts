import type { Conversation } from './types'
import { isPartialConversation, mergeConversationHistoryPage, type ConversationHistoryPage } from './conversationHistoryWindow'

/** A page or anchor read belongs to the view that requested it. Both desktop
 * windows validate its revision and discard late replies after navigation. */
export function createChatHistoryReader(ports: {
  captureView: () => () => boolean
  currentConversation: () => Conversation | null
  currentConversationId: () => string | null
  knownConversationRevision?: (id: string) => number
  readHistoryPage: (id: string, before: number, after?: number) => Promise<ConversationHistoryPage>
  readWindow: (id: string, anchor?: string) => Promise<Conversation>
  showHistoryPage: (conversation: Conversation) => void
  showHistoryTarget: (conversation: Conversation, messageId: string) => void
  reportHistoryError: (id: string, error: string | null) => void
  reportConversationChanged?: (id: string) => void
}) {
  const readAndShowConversation = async (
    conversationId: string,
    read: (conversationId: string) => Promise<Conversation>,
    isCurrent: () => boolean,
    show: (conversation: Conversation) => void,
  ): Promise<'shown' | 'superseded' | 'changed'> => {
    for (let attempt = 0; attempt < 3; attempt++) {
      const conversation = await read(conversationId)
      if (!isCurrent()) return 'superseded'
      if ((conversation.revision ?? 0) >= (ports.knownConversationRevision?.(conversationId) ?? 0)) {
        show(conversation)
        return 'shown'
      }
    }
    return 'changed'
  }

  const historyPages = new Map<string, () => boolean>()
  const loadHistoryPage = async (edge: 'older' | 'gap-start' | 'gap-end' = 'older') => {
    const current = ports.currentConversation()
    if (!current || !isPartialConversation(current)) return
    const gap = current.history_gap
    const before = edge === 'older' ? current.history_start : gap?.end
    if (!before) return
    const pending = historyPages.get(current.id)
    if (pending?.()) return
    const lease = ports.captureView()
    const canCommit = () => lease()
      && ports.currentConversationId() === current.id
    historyPages.set(current.id, lease)
    ports.reportHistoryError(current.id, null)
    try {
      const page = await ports.readHistoryPage(current.id, before, edge === 'gap-start' ? gap?.start : undefined)
      if (!canCommit()) return
      const latest = ports.currentConversation()
      if (!latest || latest.id !== current.id || !isPartialConversation(latest)) return
      if (latest.history_gap !== gap || latest.history_start !== current.history_start) return
      const merged = mergeConversationHistoryPage(latest, page)
      if (merged && merged.revision >= (ports.knownConversationRevision?.(current.id) ?? 0)) {
        ports.showHistoryPage(merged)
        return
      }
      const split = gap ? gap.start - (current.history_start ?? 0) : 0
      const index = edge === 'gap-start' ? Math.max(0, split - 1) : edge === 'gap-end' ? split : 0
      const anchor = current.messages[index]?.id
      const outcome = await readAndShowConversation(current.id, id => ports.readWindow(id, anchor),
        () => canCommit() && ports.currentConversation()?.messages === latest.messages, ports.showHistoryPage)
      if (outcome === 'changed') ports.reportConversationChanged?.(current.id)
    } catch (error) {
      if (canCommit()) {
        console.error('Failed to load older conversation history:', error)
        ports.reportHistoryError(current.id, edge === 'older' ? '加载更早消息失败，请重试。' : '加载历史消息失败，请重试。')
      }
    } finally {
      if (historyPages.get(current.id) === lease) historyPages.delete(current.id)
    }
  }
  let historyFocusSequence = 0
  const focusHistoryMessage = async (conversationId: string, messageId: string, signal: AbortSignal, anchorMessageId = messageId) => {
    const sequence = ++historyFocusSequence
    const currentView = ports.captureView()
    const isCurrent = () => !signal.aborted && sequence === historyFocusSequence
      && currentView()
      && ports.currentConversationId() === conversationId
    if (!isCurrent()) return
    ports.reportHistoryError(conversationId, null)
    try {
      const outcome = await readAndShowConversation(conversationId, id => ports.readWindow(id, anchorMessageId), isCurrent,
        (conversation) => ports.showHistoryTarget(conversation, messageId))
      if (outcome === 'changed' && isCurrent()) ports.reportConversationChanged?.(conversationId)
    } catch (error) {
      if (!isCurrent()) return
      console.error('Failed to load historical navigation target:', error)
      ports.reportHistoryError(conversationId, '打开历史消息失败，请重试。')
    }
  }
  return { readAndShowConversation, loadHistoryPage, focusHistoryMessage }
}
