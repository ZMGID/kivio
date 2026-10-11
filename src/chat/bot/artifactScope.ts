export function artifactsForConversation<T extends { conversationId: string }>(
  items: readonly T[],
  conversationId: string | null | undefined,
): T[] {
  if (!conversationId) return [...items]
  return items.filter((item) => item.conversationId === conversationId)
}
