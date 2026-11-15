/** Short-lived reading intent. Geometry is resolved by MessageList against its
 * current rows; a saved pixel offset is only a fallback when the row is gone. */
export interface ChatReadingPosition {
  following: boolean
  rowKey: string | null
  rowRevision?: string | null
  rowOffset: number
  scrollTop: number
  layoutKey: string
}

const MAX_POSITIONS = 24
const positions = new Map<string, ChatReadingPosition>()

export function rememberChatReadingPosition(conversationId: string, position: ChatReadingPosition): void {
  positions.delete(conversationId)
  positions.set(conversationId, position)
  if (positions.size > MAX_POSITIONS) positions.delete(positions.keys().next().value!)
}

export function recallChatReadingPosition(conversationId: string): ChatReadingPosition | null {
  const position = positions.get(conversationId)
  if (!position) return null
  positions.delete(conversationId)
  positions.set(conversationId, position)
  return position
}

/** Row identity of a multi-answer group; a single message row uses its message id. */
export function readingGroupRowKey(groupId: string): string {
  return `group-${groupId}`
}

/** A first window is enough unless the reader left at a stored row it does not
 * hold. The live row is restored from the stream preview, never from history. */
export function readingPositionNeedsFullHistory(
  position: ChatReadingPosition | null,
  loaded: readonly { id: string; group_id?: string | null; groupId?: string | null }[],
  liveMessageId?: string | null,
): boolean {
  if (!position || position.following) return false
  const { rowKey } = position
  if (!rowKey) return true
  if (rowKey === liveMessageId) return false
  return !loaded.some((message) => {
    const groupId = message.group_id ?? message.groupId
    return message.id === rowKey || (groupId != null && readingGroupRowKey(groupId) === rowKey)
  })
}

export function forgetChatReadingPosition(conversationId: string): void {
  positions.delete(conversationId)
}

export function clearChatReadingPositions(): void {
  positions.clear()
}
