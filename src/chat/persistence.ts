import type { Window } from '@tauri-apps/api/window'
export const CHAT_DEFAULT_SIZE = { width: 1280, height: 800 }
/** 侧栏收起时可缩到的最小尺寸 */
export const CHAT_MIN_SIZE_COLLAPSED = { width: 400, height: 400 }
/** 侧栏展开时整窗最小尺寸（260px 侧栏 + 主内容区） */
export const CHAT_MIN_SIZE_EXPANDED = { width: 660, height: 400 }
export const CHAT_MIN_SIZE = CHAT_MIN_SIZE_COLLAPSED
export function getChatPlatformWindowSize(
  size: { width: number; height: number },
): { width: number; height: number } {
  return size
}

export type ChatWindowGeometry = {
  width: number
  height: number
  x?: number
  y?: number
}

const CHAT_LAST_ROUTE_KEY = 'kivio-chat-last-route'
const CHAT_SIDEBAR_COLLAPSED_KEY = 'kivio-chat-sidebar-collapsed'
const CHAT_WINDOW_GEOMETRY_KEY = 'kivio-chat-window-geometry'
/** @deprecated 旧版仅持久化尺寸；读取时自动迁移到 geometry key */
const CHAT_WINDOW_SIZE_KEY = 'kivio-chat-window-size'

export function hashPath(): string {
  return window.location.hash.replace('#', '').split('?')[0]
}

export function isChatPath(path: string): boolean {
  return path === 'chat' || path.startsWith('chat/')
}

export function isChatSettingsPath(path: string): boolean {
  return path === 'chat/settings' || path.startsWith('chat/settings/')
}

function getLocalStorageItem(key: string): string | null {
  try {
    return window.localStorage?.getItem(key) ?? null
  } catch {
    return null
  }
}

function setLocalStorageItem(key: string, value: string) {
  try {
    window.localStorage?.setItem(key, value)
  } catch {
    // Storage can be unavailable in restricted previews. Chat still works without persistence.
  }
}

function removeLocalStorageItem(key: string) {
  try {
    window.localStorage?.removeItem(key)
  } catch {
    // Ignore storage errors; persistence is best-effort only.
  }
}

export function normalizeStoredChatRoute(value: string | null): string | null {
  if (!value) return null
  const route = value.startsWith('#') ? value : `#${value}`
  const path = route.replace('#', '').split('?')[0]
  if (!isChatPath(path) || isChatSettingsPath(path)) return null
  return route
}

export function rememberCurrentChatRoute() {
  const path = hashPath()
  if (!path.startsWith('chat/') || isChatSettingsPath(path)) return
  setLocalStorageItem(CHAT_LAST_ROUTE_KEY, window.location.hash || '#chat')
}

export function getRememberedChatRoute(): string | null {
  return normalizeStoredChatRoute(getLocalStorageItem(CHAT_LAST_ROUTE_KEY))
}

export function forgetRememberedChatRoute() {
  removeLocalStorageItem(CHAT_LAST_ROUTE_KEY)
}

export function getRememberedChatSidebarCollapsed(): boolean {
  return getLocalStorageItem(CHAT_SIDEBAR_COLLAPSED_KEY) === '1'
}

export function rememberChatSidebarCollapsed(collapsed: boolean) {
  setLocalStorageItem(CHAT_SIDEBAR_COLLAPSED_KEY, collapsed ? '1' : '0')
}

function normalizeChatWindowGeometry(
  parsed: Partial<ChatWindowGeometry>,
): ChatWindowGeometry | null {
  const width = Number(parsed.width)
  const height = Number(parsed.height)
  if (!Number.isFinite(width) || !Number.isFinite(height)) return null
  const x = Number(parsed.x)
  const y = Number(parsed.y)
  const min = getChatPlatformWindowSize(CHAT_MIN_SIZE)
  const next: ChatWindowGeometry = {
    width: Math.max(min.width, Math.round(width)),
    height: Math.max(min.height, Math.round(height)),
  }
  if (Number.isFinite(x) && Number.isFinite(y)) {
    next.x = Math.round(x)
    next.y = Math.round(y)
  }
  return next
}

export function getRememberedChatGeometry(): ChatWindowGeometry {
  try {
    const rawGeometry = getLocalStorageItem(CHAT_WINDOW_GEOMETRY_KEY)
    if (rawGeometry) {
      const parsed = JSON.parse(rawGeometry) as Partial<ChatWindowGeometry>
      const normalized = normalizeChatWindowGeometry(parsed)
      if (normalized) return normalized
    }
    const rawSize = getLocalStorageItem(CHAT_WINDOW_SIZE_KEY)
    if (rawSize) {
      const parsed = JSON.parse(rawSize) as Partial<{ width: number; height: number }>
      const normalized = normalizeChatWindowGeometry(parsed)
      if (normalized) return normalized
    }
  } catch {
    // fall through
  }
  return getChatPlatformWindowSize(CHAT_DEFAULT_SIZE)
}

export function getRememberedChatSize(): { width: number; height: number } {
  const { width, height } = getRememberedChatGeometry()
  return { width, height }
}

export function rememberChatGeometry(geometry: ChatWindowGeometry) {
  const normalized = normalizeChatWindowGeometry(geometry)
  if (!normalized) return
  setLocalStorageItem(CHAT_WINDOW_GEOMETRY_KEY, JSON.stringify(normalized))
}

export function rememberChatSize(width: number, height: number) {
  const current = getRememberedChatGeometry()
  rememberChatGeometry({ ...current, width, height })
}

/** 在 show 之前恢复上次窗口尺寸与位置，避免先闪默认 1280×800 再跳变。 */
export async function restoreChatWindowGeometry(win: Window): Promise<void> {
  if (await win.isMaximized()) return

  const { LogicalPosition, LogicalSize } = await import('@tauri-apps/api/window')
  const geo = getRememberedChatGeometry()
  await win.setSize(new LogicalSize(geo.width, geo.height))
  if (Number.isFinite(geo.x) && Number.isFinite(geo.y)) {
    await win.setPosition(new LogicalPosition(geo.x!, geo.y!))
  } else {
    await win.center()
  }
}

export async function snapshotChatWindowGeometry(win: Window): Promise<ChatWindowGeometry | null> {
  try {
    const scaleFactor = await win.scaleFactor()
    const [size, position] = await Promise.all([win.innerSize(), win.outerPosition()])
    const logicalSize = size.toLogical(scaleFactor)
    const logicalPosition = position.toLogical(scaleFactor)
    return normalizeChatWindowGeometry({
      width: logicalSize.width,
      height: logicalSize.height,
      x: logicalPosition.x,
      y: logicalPosition.y,
    })
  } catch {
    return null
  }
}
