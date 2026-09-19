import type { ChatMessage, PendingAttachment } from './types'

/** Accepted-but-not-yet-persisted user messages are per conversation, not per
 * current page. A token makes a late completion unable to clear a newer send. */
export function createOptimisticUserPresentation() {
  const pending = new Map<string, { token: number; message: ChatMessage; baselineMatches: number }>()
  const listeners = new Set<() => void>()
  let sequence = 0
  let revision = 0

  const publish = () => {
    revision += 1
    listeners.forEach((listener) => listener())
  }

  return {
    subscribe: (listener: () => void) => {
      listeners.add(listener)
      return () => { listeners.delete(listener) }
    },
    getRevision: () => revision,
    begin: (
      conversationId: string,
      content: string,
      attachments: PendingAttachment[],
      now = Date.now(),
      stored: ChatMessage[] = [],
    ) => {
      const token = ++sequence
      const message: ChatMessage = {
        id: `pending-user-${now}-${token}`,
        role: 'user',
        content,
        attachments: attachments.map(({ id, type, name, path }) => ({ id, type, name, path })),
        timestamp: Math.floor(now / 1000),
      }
      const baselineMatches = stored.filter((item) => item.role === 'user' && item.content === content).length
      pending.set(conversationId, { token, message, baselineMatches })
      publish()
      return { token, message }
    },
    settle: (conversationId: string, token: number) => {
      if (pending.get(conversationId)?.token !== token) return
      pending.delete(conversationId)
      publish()
    },
    clear: (conversationId: string) => {
      if (!pending.delete(conversationId)) return
      publish()
    },
    overlay: (conversationId: string | null | undefined, stored: ChatMessage[]): ChatMessage[] => {
      const claimed = conversationId ? pending.get(conversationId) : null
      if (!claimed) return stored
      const { message, baselineMatches } = claimed
      const alreadyStored = stored.some((item) => item.id === message.id)
        || stored.filter((item) => item.role === 'user' && item.content === message.content).length > baselineMatches
      return alreadyStored ? stored : [...stored, message]
    },
  }
}
