export type ChatRouteKind =
  | 'root'
  | 'conversation'
  | 'settings'
  | 'assistants'
  | 'skill'
  | 'plugins'
  | 'sessions'
  | 'automations'
  | 'mcp'
  | 'knowledge'
  | 'notes'
  | 'onboarding'
  | 'popout'
  | 'other'

const CENTER_SEGMENTS = new Set<Exclude<ChatRouteKind, 'root' | 'conversation' | 'other'>>([
  'settings',
  'assistants',
  'skill',
  'plugins',
  'sessions',
  'automations',
  'mcp',
  'knowledge',
  'notes',
  'onboarding',
  'popout',
])

export function pathFromHash(hash: string): string {
  return hash.replace(/^#/, '').split('?')[0]
}

export function isChatPath(path: string): boolean {
  return path === 'chat' || path.startsWith('chat/')
}

export function isChatSettingsPath(path: string): boolean {
  return chatRouteKind(path) === 'settings'
}

export function isChatOnboardingPath(path: string): boolean {
  return chatRouteKind(path) === 'onboarding'
}

export function isChatPopoutPath(path: string): boolean {
  return chatRouteKind(path) === 'popout'
}

export function chatRouteKind(path: string): ChatRouteKind {
  if (path === 'chat') return 'root'
  if (!path.startsWith('chat/')) return 'other'
  const segment = path.slice('chat/'.length).split('/')[0]
  return CENTER_SEGMENTS.has(segment as Exclude<ChatRouteKind, 'root' | 'conversation' | 'other'>)
    ? segment as Exclude<ChatRouteKind, 'root' | 'conversation' | 'other'>
    : 'conversation'
}

export function decodeChatRouteId(prefix: string, path: string): string | null {
  if (!path.startsWith(prefix)) return null
  const encoded = path.slice(prefix.length)
  if (!encoded || encoded.includes('/')) return null
  try {
    return decodeURIComponent(encoded)
  } catch {
    return null
  }
}

/** Encode one opaque id into a route path; callers decide whether to add the hash marker. */
export function encodeChatRouteId(prefix: string, id: string): string {
  return `${prefix}${encodeURIComponent(id)}`
}

export function decodeConversationRouteId(path: string): string | null {
  if (chatRouteKind(path) !== 'conversation') return null
  return decodeChatRouteId('chat/', path)
}

export function isRememberableChatRoute(path: string): boolean {
  const kind = chatRouteKind(path)
  if (kind === 'conversation') return decodeConversationRouteId(path) !== null
  return kind === 'assistants'
    || kind === 'skill'
    || kind === 'plugins'
    || kind === 'sessions'
    || kind === 'automations'
    || kind === 'mcp'
    || kind === 'knowledge'
    || kind === 'notes'
}
