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

export function forgetChatReadingPosition(conversationId: string): void {
  positions.delete(conversationId)
}

export function clearChatReadingPositions(): void {
  positions.clear()
}
