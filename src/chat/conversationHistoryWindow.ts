import type { ChatMessage, Conversation } from './types'

export const EMPTY_HISTORY_DIRECTORY: NonNullable<Conversation['history_directory']> = []

export interface ConversationHistoryPage {
  revision: number
  start: number
  end: number
  total: number
  messages: ChatMessage[]
}

/** Match the native window edge so multi-answer arms do not start mid-group. */
export function historyWindowStart(messages: ChatMessage[], end: number): number {
  let start = Math.max(0, end - 60)
  if (start === 0 || start >= end || messages[start].role !== 'assistant') return start
  const group = messages[start].group_id ?? messages[start].groupId
  if (group) {
    while (start > Math.max(0, end - 64)
      && (messages[start - 1].group_id ?? messages[start - 1].groupId) === group) start -= 1
  }
  if (start > 0 && messages[start].role === 'assistant' && messages[start - 1].role === 'user') start -= 1
  return start
}

/** Pages only extend the same snapshot. Edits, truncations and background
 * completions change revision and must be reloaded instead of spliced in. */
export function prependConversationHistoryPage(
  current: Conversation,
  page: ConversationHistoryPage,
): Conversation | null {
  if (current.history_start == null || current.history_total == null) return null
  if (page.revision !== current.revision || page.total !== current.history_total
    || page.end !== current.history_start || page.start < 0
    || page.messages.length !== page.end - page.start) return null
  return {
    ...current,
    messages: [...page.messages, ...current.messages],
    history_start: page.start,
  }
}

export function isPartialConversation(conversation: Conversation | null): boolean {
  return Boolean(conversation && (conversation.history_start ?? 0) > 0)
}
