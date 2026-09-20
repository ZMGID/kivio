import routeContract from './routeContract.json'

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
  | 'artifacts'
  | 'onboarding'
  | 'popout'
  | 'other'

type ChatCenterRouteKind = Exclude<ChatRouteKind, 'root' | 'conversation' | 'other'>
export type ChatView = Exclude<ChatRouteKind, 'root' | 'plugins' | 'sessions' | 'popout' | 'other'>

/**
 * Route vocabulary is declared once in routeContract.json and consumed by both this codec and
 * Rust's persisted-route validator. The cases in that contract are the cross-language fixture.
 */
const CENTER_SEGMENTS = new Set<string>(routeContract.centerSegments)
const REMEMBERABLE_CENTER_SEGMENTS = new Set<string>(routeContract.rememberableCenterSegments)

function isCenterRouteKind(segment: string): segment is ChatCenterRouteKind {
  return CENTER_SEGMENTS.has(segment)
}

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
  if (isCenterRouteKind(segment)) return segment
  return decodeChatRouteId('chat/', path) === null ? 'other' : 'conversation'
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
  return REMEMBERABLE_CENTER_SEGMENTS.has(kind)
}

/** Persisted/legacy routes additionally allow the chat root used before a conversation exists. */
export function isRestorableChatRoute(route: string): boolean {
  const path = pathFromHash(route)
  return path === 'chat' || isRememberableChatRoute(path)
}
