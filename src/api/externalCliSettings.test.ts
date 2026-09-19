import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { isTauriRuntime } from './tauri'
import { externalCliSettingsApi, onExternalAgentsUpdated } from './externalCliSettings'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('./tauri', () => ({ isTauriRuntime: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }))

describe('external CLI settings transport', () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset()
    vi.mocked(isTauriRuntime).mockReset().mockReturnValue(true)
  })

  it('keeps browser preview fallbacks local without calling native commands', async () => {
    vi.mocked(isTauriRuntime).mockReturnValue(false)

    expect(await externalCliSettingsApi.externalCliScanCcSwitch()).toEqual({ providers: [], skipped: 0 })
    expect(await externalCliSettingsApi.dshPluginInventory()).toEqual([])
    expect(await externalCliSettingsApi.piExtensionsInventory()).toBeNull()
    expect(invoke).not.toHaveBeenCalled()
  })

  it('forwards DSH plugin patches through the single native transport owner', async () => {
    const patch = { shell: { timeoutMs: 5000 } }
    const snapshot = { settingsPath: '/tmp/settings.yaml' }
    vi.mocked(invoke).mockResolvedValue(snapshot)

    expect(await externalCliSettingsApi.dshPluginSettingsSave(patch)).toBe(snapshot)
    expect(invoke).toHaveBeenCalledWith('chat_dsh_plugin_settings_save', { patch })
  })

  it('normalizes an older model-probe response without a source field', async () => {
    vi.mocked(invoke).mockResolvedValue({ success: true, models: [{ id: 'm', label: 'M' }] })

    expect(await externalCliSettingsApi.detectExternalAgentModels('claude')).toEqual({
      models: [{ id: 'm', label: 'M' }],
      reasoningOptions: [],
      reasoningByModel: {},
      source: 'probed',
      probeError: undefined,
      currentModel: null,
      currentReasoning: null,
    })
    expect(invoke).toHaveBeenCalledWith('chat_detect_external_agent_models', {
      agentId: 'claude', conversationId: undefined, force: false,
    })
  })

  it('normalizes missing agents from a native snapshot', async () => {
    vi.mocked(invoke).mockResolvedValue({ success: true })
    expect(await externalCliSettingsApi.detectExternalAgents(true)).toEqual([])
    expect(invoke).toHaveBeenCalledWith('chat_detect_external_agents', {
      forceRefresh: true, conversationId: undefined,
    })
  })

  it('does not subscribe to agent updates in browser preview', async () => {
    vi.mocked(isTauriRuntime).mockReturnValue(false)
    const handler = vi.fn()
    const unlisten = await onExternalAgentsUpdated(handler)
    unlisten()
    expect(handler).not.toHaveBeenCalled()
  })
})
