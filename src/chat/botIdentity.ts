/** Product identity survives model changes, compaction and runtime sessions. */
export const BOT_CONVERSATION_PREFIX = 'conv_bot_'
export const MEMBER_CONVERSATION_PREFIX = 'conv_bot_member_'

/** Group execution thread: `conv_bot_member_{assistantId}@{roomId}`. Not a contact. */
export function isDedicatedMemberConversation(conversationId?: string | null): boolean {
  return Boolean(conversationId?.startsWith(MEMBER_CONVERSATION_PREFIX))
}

export function memberThreadIdentity(conversationId?: string | null): { assistantId: string; roomId: string } | null {
  if (!isDedicatedMemberConversation(conversationId) || !conversationId) return null
  const rest = conversationId.slice(MEMBER_CONVERSATION_PREFIX.length)
  const mark = rest.indexOf('@')
  if (mark <= 0 || mark >= rest.length - 1) return null
  return { assistantId: rest.slice(0, mark), roomId: rest.slice(mark + 1) }
}

/** Private contact chat only. Member execution threads return null. */
export function botAssistantId(conversationId?: string | null): string | null {
  if (!conversationId || isDedicatedMemberConversation(conversationId)) return null
  if (!conversationId.startsWith(BOT_CONVERSATION_PREFIX)) return null
  return conversationId.slice(BOT_CONVERSATION_PREFIX.length) || null
}

/** Stable assistant id for a private chat or that Bot's group execution thread. */
export function botContactId(conversationId?: string | null): string | null {
  return botAssistantId(conversationId) ?? memberThreadIdentity(conversationId)?.assistantId ?? null
}

export function isBotOwnedConversation(conversationId?: string | null): boolean {
  return botContactId(conversationId) != null
}

export function botConversationId(assistantId: string): string {
  return `${BOT_CONVERSATION_PREFIX}${assistantId}`
}
