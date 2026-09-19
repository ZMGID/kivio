/**
 * @vitest-environment jsdom
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  getRememberedDockTab,
  getRememberedSidebarWidth,
  hashPath,
  isChatPath,
  clampSidebarWidth,
  normalizeStoredChatRoute,
  rememberDockTab,
  rememberSidebarWidth,
  SIDEBAR_DEFAULT_WIDTH,
  SIDEBAR_MAX_WIDTH,
  SIDEBAR_MIN_WIDTH,
} from './persistence'
import { decodeConversationRouteId } from './routeCodec'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(undefined) }))

describe('hashPath', () => {
  it('strips hash prefix and query string', () => {
    window.location.hash = '#chat/settings?tab=general'
    expect(hashPath()).toBe('chat/settings')
  })
})

describe('isChatPath', () => {
  it('matches chat routes', () => {
    expect(isChatPath('chat')).toBe(true)
    expect(isChatPath('chat/conv-1')).toBe(true)
    expect(isChatPath('settings')).toBe(false)
  })
})


describe('normalizeStoredChatRoute', () => {
  it('accepts conversation routes and normalizes missing hash', () => {
    expect(normalizeStoredChatRoute('#chat/conv-1')).toBe('#chat/conv-1')
    expect(normalizeStoredChatRoute('chat/conv-1')).toBe('#chat/conv-1')
  })

  it.each([
    'chat',
    'chat/assistants',
    'chat/skill/item',
    'chat/plugins',
    'chat/sessions',
    'chat/automations/a-1',
    'chat/mcp',
    'chat/knowledge',
    'chat/notes/n-1',
  ])('preserves the existing rememberable center route %s', (path) => {
    expect(normalizeStoredChatRoute(path)).toBe(`#${path}`)
  })

  it('rejects settings / onboarding / non-chat values', () => {
    expect(normalizeStoredChatRoute('#chat/settings')).toBeNull()
    expect(normalizeStoredChatRoute('#chat/settings?tab=general')).toBeNull()
    expect(normalizeStoredChatRoute('#chat/onboarding')).toBeNull()
    expect(normalizeStoredChatRoute('#lens')).toBeNull()
    expect(normalizeStoredChatRoute(null)).toBeNull()
  })

  it.each([
    ['chat/conv-1', 'conv-1', '#chat/conv-1'],
    ['chat/a%2Fb', 'a/b', '#chat/a%2Fb'],
    ['chat/%E0%A4%A', null, null],
    ['chat/a/b', null, null],
    ['chat/', null, null],
    ['chat/settings', null, null],
  ])('keeps navigation decoding and recovery policy aligned for %s', (path, decoded, remembered) => {
    expect(decodeConversationRouteId(path)).toBe(decoded)
    expect(normalizeStoredChatRoute(`#${path}`)).toBe(remembered)
  })
})

describe('right dock tab persistence', () => {
  it('falls back to files for the removed trajectory / Pi sessions keys', () => {
    window.localStorage.clear()
    window.localStorage.setItem('kivio-chat-dock-tab', 'trajectory')
    expect(getRememberedDockTab()).toBe('files')
    window.localStorage.setItem('kivio-chat-dock-tab', 'piSessions')
    expect(getRememberedDockTab()).toBe('files')
    rememberDockTab('git')
    expect(getRememberedDockTab()).toBe('git')
  })
})

describe('last route memory (Rust-persisted, auto-migrates from localStorage)', () => {
  let routes: typeof import('./persistence')
  beforeEach(async () => {
    window.localStorage.clear()
    vi.resetModules()
    routes = await import('./persistence')
  })

  it('remembers the current conversation route in the in-memory cache', () => {
    window.location.hash = '#chat/conv-a'
    routes.rememberCurrentChatRoute()
    expect(routes.getRememberedChatRoute()).toBe('#chat/conv-a')
  })

  it('does not remember the list / settings / onboarding routes', () => {
    window.location.hash = '#chat'
    routes.rememberCurrentChatRoute()
    expect(routes.getRememberedChatRoute()).toBeNull()

    window.location.hash = '#chat/settings'
    routes.rememberCurrentChatRoute()
    expect(routes.getRememberedChatRoute()).toBeNull()
  })

  it('auto-migrates legacy localStorage on first getRememberedChatRoute call', async () => {
    window.localStorage.setItem('kivio-chat-last-route', '#chat/conv-legacy')
    const route = routes.getRememberedChatRoute()
    expect(route).toBe('#chat/conv-legacy')
    await vi.waitFor(() => expect(window.localStorage.getItem('kivio-chat-last-route')).toBeNull())
    
    // 第二次调用应返回缓存值，不再读 localStorage
    expect(routes.getRememberedChatRoute()).toBe('#chat/conv-legacy')
  })


  it('falls back to the legacy localStorage value only when the cache is empty', () => {
    window.localStorage.setItem('kivio-chat-last-route', '#chat/conv-legacy')
    expect(routes.getRememberedChatRoute()).toBe('#chat/conv-legacy')
  })
})

describe('sidebar width persistence', () => {
  beforeEach(() => {
    window.localStorage.clear()
  })

  it('defaults to 240 and clamps to [200, 400]', () => {
    expect(getRememberedSidebarWidth()).toBe(SIDEBAR_DEFAULT_WIDTH)
    rememberSidebarWidth(180)
    expect(getRememberedSidebarWidth()).toBe(SIDEBAR_MIN_WIDTH)
    rememberSidebarWidth(480)
    expect(getRememberedSidebarWidth()).toBe(SIDEBAR_MAX_WIDTH)
    rememberSidebarWidth(320)
    expect(getRememberedSidebarWidth()).toBe(320)
  })

  it('does not let the sidebar steal the main pane’s minimum width', () => {
    expect(clampSidebarWidth(360, 500)).toBe(SIDEBAR_MIN_WIDTH)
    expect(clampSidebarWidth(360, 900)).toBe(360)
  })
})
