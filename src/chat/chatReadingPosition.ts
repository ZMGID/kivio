/** Short-lived reading intent. Geometry is resolved by MessageList against its
 * current rows; a saved pixel offset is only a fallback when the row is gone. */
export interface ChatReadingPosition {
  following: boolean
  rowKey: string | null
  rowRevision?: string | null
  rowOffset: number
  scrollTop: number
  layoutKey: string
  block?: ChatReadingBlock
}

export interface ChatReadingBlock {
  segmentId: string | null
  messageId: string | null
  excerpt: string
  index: number
  top: number
  inWork: boolean
  work?: { key: string; offset: number }
}

const READING_BLOCKS = '.chat-markdown :is(p, li, pre, h1, h2, h3, h4, h5, h6, table, blockquote),'
  + ' [data-chat-work-block] :is(pre, button[data-chat-disclosure])'
const readingBlocks = (scope: HTMLElement) => [...scope.querySelectorAll<HTMLElement>(READING_BLOCKS)]
const blockExcerpt = (block: HTMLElement) => block.textContent?.trim().slice(0, 96) ?? ''

export function captureLiveReadingPosition(viewport: HTMLElement, layoutKey: string): ChatReadingPosition | null {
  const row = viewport.querySelector<HTMLElement>('[data-chat-live-reading-row]')
  const rowKey = row?.dataset.chatReadingKey
  if (!row || !rowKey) return null
  const view = viewport.getBoundingClientRect()
  const rect = row.getBoundingClientRect()
  if (rect.top > view.top + 1 || rect.bottom <= view.top) return null
  const element = readingBlocks(row).find((block) => {
    const bounds = block.getBoundingClientRect()
    return bounds.bottom > view.top && bounds.top < view.bottom && Boolean(blockExcerpt(block))
  })
  const segment = element?.closest<HTMLElement>('[data-chat-reading-segment]')
  const message = element?.closest<HTMLElement>('[data-message-id]')
  const work = element?.closest<HTMLElement>('[data-chat-work-block]')
  const workKey = work?.dataset.chatWorkBlock
  return {
    following: false,
    rowKey,
    rowOffset: view.top - rect.top,
    scrollTop: viewport.scrollTop,
    layoutKey,
    block: element ? {
      segmentId: segment?.dataset.chatReadingSegment ?? null,
      messageId: message?.dataset.messageId ?? null,
      excerpt: blockExcerpt(element),
      index: readingBlocks(segment ?? work ?? message ?? row).indexOf(element),
      top: element.getBoundingClientRect().top - view.top,
      inWork: Boolean(element.closest('[data-chat-process-group]')),
      work: work && workKey ? { key: workKey, offset: view.top - work.getBoundingClientRect().top } : undefined,
    } : undefined,
  }
}

export function resolveChatReadingBlock(row: HTMLElement, block: ChatReadingBlock): {
  element: HTMLElement
  offset: number
  pending: boolean
} | undefined {
  const message = block.messageId && row.dataset.messageId !== block.messageId
    ? row.querySelector<HTMLElement>(`[data-message-id="${CSS.escape(block.messageId)}"]`) ?? row
    : row
  const segment = block.segmentId
    ? message.querySelector<HTMLElement>(`[data-chat-reading-segment="${CSS.escape(block.segmentId)}"]`)
    : null
  const work = block.work
    ? message.querySelector<HTMLElement>(`[data-chat-work-block="${CSS.escape(block.work.key)}"]`)
    : null
  const blocks = readingBlocks(segment ?? work ?? message)
  const matching = (candidate: HTMLElement) => {
    const excerpt = blockExcerpt(candidate)
    return candidate.getClientRects().length > 0
      && (excerpt.startsWith(block.excerpt) || (excerpt.length > 0 && block.excerpt.startsWith(excerpt)))
  }
  const indexed = blocks[block.index]
  const element = indexed && matching(indexed) ? indexed : blocks.find(matching)
  if (element) return { element, offset: -block.top, pending: false }
  if (work && block.work) return {
    element: work,
    offset: Math.min(block.work.offset, Math.max(0, work.getBoundingClientRect().height - 1)),
    pending: true,
  }
  return undefined
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
