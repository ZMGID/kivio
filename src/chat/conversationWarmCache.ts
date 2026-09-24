import type { Conversation } from './types'

const TTL_MS = 30_000
const MAX_ENTRIES = 4
const MAX_BYTES = 24 * 1024 * 1024

type Entry = { conversation: Conversation; bytes: number; expiresAt: number }

/** Owns loaded display windows between short navigation round trips.
 * Every hit must be checked against the repository index revision first. */
export function createConversationWarmCache(now = Date.now) {
  const entries = new Map<string, Entry>()
  const pending = new Map<string, { conversation: Conversation; cancel: () => void }>()
  let usedBytes = 0

  const remove = (id: string) => {
    const entry = entries.get(id)
    if (!entry) return
    usedBytes -= entry.bytes
    entries.delete(id)
  }
  const prune = () => {
    for (const [id, entry] of entries) if (entry.expiresAt <= now()) remove(id)
    while (entries.size > MAX_ENTRIES || usedBytes > MAX_BYTES) remove(entries.keys().next().value!)
  }

  const cancelPending = (id: string) => {
    pending.get(id)?.cancel()
    pending.delete(id)
  }

  const scheduleAfterPaint = (work: () => void): (() => void) => {
    if (typeof window === 'undefined') {
      const timer = setTimeout(work, 0)
      return () => clearTimeout(timer)
    }
    if (typeof window.requestIdleCallback === 'function') {
      const id = window.requestIdleCallback(work, { timeout: 1_000 })
      return () => window.cancelIdleCallback(id)
    }
    let timer: number | null = null
    const frame = window.requestAnimationFrame(() => { timer = window.setTimeout(work, 0) })
    return () => {
      window.cancelAnimationFrame(frame)
      if (timer !== null) window.clearTimeout(timer)
    }
  }

  const remember = (conversation: Conversation) => {
    cancelPending(conversation.id)
    remove(conversation.id)
    // A complete JSON snapshot is the transport representation. Measuring it
    // here counts nested tool cards and metadata that text-only estimates miss.
    const bytes = JSON.stringify(conversation).length * 2
    if (bytes > MAX_BYTES) return
    entries.set(conversation.id, { conversation, bytes, expiresAt: now() + TTL_MS })
    usedBytes += bytes
    prune()
  }

  return {
    remember,
    rememberSoon(conversation: Conversation) {
      cancelPending(conversation.id)
      const cancel = scheduleAfterPaint(() => {
        if (pending.get(conversation.id)?.conversation !== conversation) return
        pending.delete(conversation.id)
        remember(conversation)
      })
      pending.set(conversation.id, { conversation, cancel })
      while (pending.size > MAX_ENTRIES) cancelPending(pending.keys().next().value!)
    },
    async get(id: string, revision: (id: string) => Promise<number | null>): Promise<Conversation | null> {
      prune()
      const entry = entries.get(id)
      if (!entry) return null
      const currentRevision = await revision(id)
      if (entries.get(id) !== entry) return null
      if (currentRevision === null || currentRevision !== entry.conversation.revision) {
        remove(id)
        return null
      }
      entries.delete(id)
      entries.set(id, entry)
      return entry.conversation
    },
    forget(id: string) { cancelPending(id); remove(id) },
    clear() {
      for (const id of pending.keys()) cancelPending(id)
      entries.clear()
      usedBytes = 0
    },
    stats() { prune(); return { entries: entries.size, bytes: usedBytes } },
  }
}
