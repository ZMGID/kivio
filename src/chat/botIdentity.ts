/** Product identity survives model changes, compaction and runtime sessions. */
export const BOT_CONVERSATION_PREFIX = 'conv_bot_'

export function botAssistantId(conversationId?: string | null): string | null {
  return conversationId?.startsWith(BOT_CONVERSATION_PREFIX)
    ? conversationId.slice(BOT_CONVERSATION_PREFIX.length) || null
    : null
}

export function botConversationId(assistantId: string): string {
  return `${BOT_CONVERSATION_PREFIX}${assistantId}`
}
