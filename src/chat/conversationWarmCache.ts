import type { Conversation } from './types'

const TTL_MS = 5 * 60_000
// Fast local index reads should commit once, without mounting an intermediate view.
const PREVIEW_DELAY_MS = 32
const MAX_ENTRIES = 4
const MAX_BYTES = 24 * 1024 * 1024

/** Extra validity beyond the stored revision, e.g. a snapshot taken during a run. */
type StillCurrent = () => boolean
type Entry = {
  conversation: Conversation; bytes: number; expiresAt: number
  timer: ReturnType<typeof setTimeout>; stillCurrent?: StillCurrent
}

/** Owns loaded display windows between short navigation round trips.
 * Only a revision-checked hit may become the active conversation. */
export function createConversationWarmCache(now = Date.now) {
  const entries = new Map<string, Entry>()
  const pending = new Map<string, { conversation: Conversation; cancel: () => void; stillCurrent?: StillCurrent }>()
  let usedBytes = 0

  const remove = (id: string) => {
    const entry = entries.get(id)
    if (!entry) return
    clearTimeout(entry.timer)
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

  const remember = (conversation: Conversation, stillCurrent?: StillCurrent) => {
    cancelPending(conversation.id)
    remove(conversation.id)
    // A complete JSON snapshot is the transport representation. Measuring it
    // here counts nested tool cards and metadata that text-only estimates miss.
    const bytes = JSON.stringify(conversation).length * 2
    if (bytes > MAX_BYTES) return
    const id = conversation.id
    entries.set(id, {
      conversation, bytes, expiresAt: now() + TTL_MS, timer: setTimeout(() => remove(id), TTL_MS), stillCurrent,
    })
    usedBytes += bytes
    prune()
  }

  return {
    remember,
    rememberSoon(conversation: Conversation, stillCurrent?: StillCurrent) {
      cancelPending(conversation.id)
      const cancel = scheduleAfterPaint(() => {
        if (pending.get(conversation.id)?.conversation !== conversation) return
        pending.delete(conversation.id)
        remember(conversation, stillCurrent)
      })
      pending.set(conversation.id, { conversation, cancel, stillCurrent })
      while (pending.size > MAX_ENTRIES) cancelPending(pending.keys().next().value!)
    },
    async get(
      id: string,
      revision: (id: string) => Promise<number | null>,
      preview?: (conversation: Conversation) => void,
    ): Promise<Conversation | null> {
      prune()
      // A rapid A → B → A return may precede the idle cache bookkeeping.
      // Reuse that already-loaded window instead of loading the full history.
      const held = pending.get(id) ?? entries.get(id)
      const conversation = held?.conversation
      if (!conversation) return null
      if (held.stillCurrent && !held.stillCurrent()) {
        cancelPending(id)
        remove(id)
        return null
      }
      const validating = revision(id)
      const previewTimer = preview && setTimeout(() => {
        const current = pending.get(id) ?? entries.get(id)
        if (current?.conversation === conversation && (!current.stillCurrent || current.stillCurrent())) {
          preview(conversation)
        }
      }, PREVIEW_DELAY_MS)
      let currentRevision: number | null
      try {
        currentRevision = await validating
      } finally {
        clearTimeout(previewTimer)
      }
      const current = pending.get(id) ?? entries.get(id)
      if (current?.conversation !== conversation) return null
      if (
        currentRevision === null || currentRevision !== conversation.revision
        || (current.stillCurrent && !current.stillCurrent())
      ) {
        cancelPending(id)
        remove(id)
        return null
      }
      const entry = entries.get(id)
      if (entry) {
        entries.delete(id)
        entries.set(id, entry)
      }
      return conversation
    },
    forget(id: string) { cancelPending(id); remove(id) },
    clear() {
      for (const id of pending.keys()) cancelPending(id)
      for (const id of entries.keys()) remove(id)
      usedBytes = 0
    },
    stats() { prune(); return { entries: entries.size, bytes: usedBytes } },
  }
}
