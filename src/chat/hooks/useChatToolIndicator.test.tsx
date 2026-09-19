import { act, renderHook } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { api } from '../../api/tauri'
import { getSettingsCached, refreshSettings, subscribeSettings, updateSettingsCached } from '../../api/settingsCache'
import { DEFAULT_APPROVAL_POLICY, useChatToolIndicator } from './useChatToolIndicator'

vi.mock('../../api/tauri', () => ({
  isTauriRuntime: () => true,
  api: {
    chatMcpListTools: vi.fn(),
    onMcpServerState: vi.fn(),
  },
}))
vi.mock('../../api/settingsCache', () => ({
  getSettingsCached: vi.fn(),
  refreshSettings: vi.fn(),
  subscribeSettings: vi.fn(),
  updateSettingsCached: vi.fn(),
}))

const mockList = vi.mocked(api.chatMcpListTools)
const mockServerState = vi.mocked(api.onMcpServerState)
const mockGet = vi.mocked(getSettingsCached)
const mockRefresh = vi.mocked(refreshSettings)
const mockSubscribe = vi.mocked(subscribeSettings)
const mockUpdate = vi.mocked(updateSettingsCached)

type Settings = Awaited<ReturnType<typeof getSettingsCached>>

let settings: Settings
let serverStateListener: ((payload: { state: { kind: string } }) => void) | null
let settingsListener: ((next: Settings) => void) | null

function makeSettings(overrides: Record<string, unknown> = {}): Settings {
  return {
    providers: [
      { id: 'p1', apiFormat: 'openai_responses', baseUrl: 'https://a', request: { oauth: { provider: 'codex' } } },
      { id: 'p2', apiFormat: 'anthropic', baseUrl: 'https://b', request: {} },
    ],
    chatTools: {
      enabled: true,
      approvalPolicy: 'always_ask',
      disabledSkillIds: ['s1'],
      nativeTools: { webSearch: false, readFile: true, skillRuntime: true },
      servers: [
        { id: 'srv', enabled: false },
        { id: 'plugin-office', enabled: true },
      ],
    },
    ...overrides,
  } as unknown as Settings
}

beforeEach(() => {
  vi.useFakeTimers()
  settings = makeSettings()
  serverStateListener = null
  settingsListener = null
  mockGet.mockReset()
  mockGet.mockImplementation(async () => settings)
  mockRefresh.mockReset()
  mockRefresh.mockImplementation(async () => settings)
  mockUpdate.mockReset()
  mockUpdate.mockImplementation(async (mutate) => {
    settings = mutate(settings)
    return settings
  })
  mockList.mockReset()
  mockList.mockResolvedValue({ success: true, tools: [{ id: 't1' }, { id: 't2' }], discoveryPending: false } as never)
  mockServerState.mockReset()
  mockServerState.mockImplementation(async (listener) => {
    serverStateListener = listener as never
    return () => { serverStateListener = null }
  })
  mockSubscribe.mockReset()
  mockSubscribe.mockImplementation((listener) => {
    settingsListener = listener as never
    return () => { settingsListener = null }
  })
})

async function flush() {
  await act(async () => {
    vi.runAllTimers()
    await Promise.resolve()
  })
  await act(async () => {})
}

describe('useChatToolIndicator: snapshot', () => {
  it('starts pending, then loads settings capability tables and the passive tool catalog on idle', async () => {
    const onSettingsChange = vi.fn()
    const { result } = renderHook(() => useChatToolIndicator({ onSettingsChange }))
    expect(result.current.toolDiscoveryPending).toBe(true)
    expect(mockList).not.toHaveBeenCalled()

    await flush()

    expect(mockList).toHaveBeenCalledWith(true)
    expect(result.current).toMatchObject({
      toolDiscoveryPending: false,
      enabledToolCount: 2,
      toolsRequested: true,
      approvalPolicy: 'always_ask',
      disabledSkillIds: ['s1'],
      webSearchEnabled: false,
      providerApiFormats: { p1: 'openai_responses', p2: 'anthropic' },
      providerBaseUrls: { p1: 'https://a', p2: 'https://b' },
      providerOAuthTypes: { p1: 'codex', p2: '' },
    })
    expect(result.current.mcpServers.map((server) => server.id)).toEqual(['srv', 'plugin-office'])
  })

  it('does not list tools when no tool source is enabled', async () => {
    settings = makeSettings({
      chatTools: { enabled: false, servers: [], nativeTools: { skillRuntime: false } },
    })
    const { result } = renderHook(() => useChatToolIndicator({ onSettingsChange: vi.fn() }))
    await flush()
    expect(mockList).not.toHaveBeenCalled()
    expect(result.current).toMatchObject({ toolsRequested: false, toolDiscoveryPending: false, enabledToolCount: null })
  })

  it('keeps discovery pending when the cached catalog is incomplete', async () => {
    mockList.mockResolvedValue({ success: true, tools: [{ id: 't1' }], discoveryPending: true } as never)
    const { result } = renderHook(() => useChatToolIndicator({ onSettingsChange: vi.fn() }))
    await flush()
    expect(result.current).toMatchObject({ toolDiscoveryPending: true, enabledToolCount: null })
    expect(result.current.enabledTools).toHaveLength(1)
  })

  it('reports a disabled reason when listing fails and resets to defaults on a thrown error', async () => {
    mockList.mockResolvedValue({ success: false, tools: [], error: 'MCP down' } as never)
    const { result } = renderHook(() => useChatToolIndicator({ onSettingsChange: vi.fn() }))
    await flush()
    expect(result.current.toolsDisabledReason).toBe('MCP down')

    mockGet.mockRejectedValueOnce(new Error('settings unreadable'))
    await act(async () => { await result.current.refresh() })
    expect(result.current).toMatchObject({
      toolsDisabledReason: 'settings unreadable',
      toolsRequested: false,
      approvalPolicy: DEFAULT_APPROVAL_POLICY,
    })
  })

  it('keeps the disabledSkillIds reference stable when unchanged', async () => {
    const { result } = renderHook(() => useChatToolIndicator({ onSettingsChange: vi.fn() }))
    await flush()
    const first = result.current.disabledSkillIds
    await act(async () => { await result.current.refresh() })
    expect(result.current.disabledSkillIds).toBe(first)
  })
})

describe('useChatToolIndicator: reactions', () => {
  it('re-reads on MCP server state changes other than "connecting"', async () => {
    renderHook(() => useChatToolIndicator({ onSettingsChange: vi.fn() }))
    await flush()
    expect(mockList).toHaveBeenCalledTimes(1)
    await act(async () => { serverStateListener?.({ state: { kind: 'connecting' } }) })
    expect(mockList).toHaveBeenCalledTimes(1)
    await act(async () => { serverStateListener?.({ state: { kind: 'connected' } }) })
    expect(mockList).toHaveBeenCalledTimes(2)
  })

  it('mirrors server list changes from the settings subscription immediately', async () => {
    const { result } = renderHook(() => useChatToolIndicator({ onSettingsChange: vi.fn() }))
    await flush()
    act(() => {
      settingsListener?.(makeSettings({ chatTools: { servers: [{ id: 'new-only', enabled: true }] } }))
    })
    expect(result.current.mcpServers.map((server) => server.id)).toEqual(['new-only'])
  })
})

describe('useChatToolIndicator: writes', () => {
  it('sets approval policy optimistically, persists it and notifies the host', async () => {
    const onSettingsChange = vi.fn()
    const { result } = renderHook(() => useChatToolIndicator({ onSettingsChange }))
    await flush()
    await act(async () => { await result.current.setApprovalPolicy('auto') })
    expect(result.current.approvalPolicy).toBe('auto')
    expect(settings.chatTools.approvalPolicy).toBe('auto')
    expect(onSettingsChange).toHaveBeenCalledTimes(1)
  })

  it('re-reads the authoritative policy when persistence fails', async () => {
    const onSettingsChange = vi.fn()
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    const { result } = renderHook(() => useChatToolIndicator({ onSettingsChange }))
    await flush()
    mockUpdate.mockRejectedValueOnce(new Error('disk'))
    await act(async () => { await result.current.setApprovalPolicy('auto') })
    expect(onSettingsChange).not.toHaveBeenCalled()
    expect(result.current.approvalPolicy).toBe('always_ask')
    error.mockRestore()
  })

  it('toggles from the visible state with one fresh merge and refreshes the catalog', async () => {
    const onSettingsChange = vi.fn()
    const { result } = renderHook(() => useChatToolIndicator({ onSettingsChange }))
    await flush()
    settings = {
      ...settings,
      chatTools: {
        ...settings.chatTools,
        servers: settings.chatTools.servers.map((server) => server.id === 'srv'
          ? { ...server, auth: { kind: 'oauth', accessToken: 'new-token' } }
          : server),
      },
    }
    await act(async () => { await result.current.toggleMcpServer('srv') })
    expect(mockRefresh).not.toHaveBeenCalled()
    expect(mockUpdate).toHaveBeenCalledTimes(1)
    expect(settings.chatTools.servers.find((server: { id: string }) => server.id === 'srv')?.enabled).toBe(true)
    expect(settings.chatTools.servers.find((server: { id: string }) => server.id === 'srv')?.auth).toEqual({ kind: 'oauth', accessToken: 'new-token' })
    expect(result.current.mcpServers.find((server) => server.id === 'srv')?.enabled).toBe(true)
    expect(onSettingsChange).toHaveBeenCalledTimes(1)
  })

  it('refuses to toggle plugin-managed servers', async () => {
    const onSettingsChange = vi.fn()
    const { result } = renderHook(() => useChatToolIndicator({ onSettingsChange }))
    await flush()
    await act(async () => { await result.current.toggleMcpServer('plugin-office') })
    expect(mockUpdate).not.toHaveBeenCalled()
    expect(onSettingsChange).not.toHaveBeenCalled()
  })
})
