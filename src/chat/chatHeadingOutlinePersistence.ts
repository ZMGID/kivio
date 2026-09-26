const STORAGE_KEY = 'kivio-chat-heading-outline-levels'
const MAX_REMEMBERED_CONVERSATIONS = 50

type StoredLevels = Record<string, boolean>

function readStoredLevels(): StoredLevels {
  try {
    const raw = window.localStorage?.getItem(STORAGE_KEY)
    if (!raw) return {}
    const parsed: unknown = JSON.parse(raw)
    if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) return {}

    const levels: StoredLevels = {}
    for (const [conversationId, value] of Object.entries(parsed as Record<string, unknown>)) {
      if (typeof value === 'boolean' && conversationId) levels[conversationId] = value
    }
    return levels
  } catch {
    return {}
  }
}

export function getRememberedChatHeadingOutlineExpanded(conversationId: string | null | undefined): boolean {
  if (!conversationId) return false
  return readStoredLevels()[conversationId] ?? false
}

export function rememberChatHeadingOutlineExpanded(conversationId: string | null | undefined, expanded: boolean): void {
  if (!conversationId) return
  try {
    const levels = readStoredLevels()
    delete levels[conversationId]
    levels[conversationId] = expanded
    const keys = Object.keys(levels)
    while (keys.length > MAX_REMEMBERED_CONVERSATIONS) {
      const oldest = keys.shift()
      if (oldest === undefined) break
      delete levels[oldest]
    }
    window.localStorage?.setItem(STORAGE_KEY, JSON.stringify(levels))
  } catch {
    // Chat remains usable when storage is unavailable or quota is exhausted.
  }
}

export function clearRememberedChatHeadingOutlineState(): void {
  try {
    window.localStorage?.removeItem(STORAGE_KEY)
  } catch {
    // Ignore restricted storage in tests and previews.
  }
}
