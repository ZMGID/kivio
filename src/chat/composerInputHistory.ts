import { peerMessageOf } from './bot/peerMessage'
import type { ChatMessage } from './types'

/** Composer Up-arrow history: the user's own typed turns, not colleague deliveries. */
export function composerInputHistory(messages: readonly ChatMessage[] | null | undefined): string[] {
  if (!messages?.length) return []
  const history: string[] = []
  for (const message of messages) {
    if (message.role !== 'user' || peerMessageOf(message)) continue
    history.push(message.content)
  }
  return history
}
