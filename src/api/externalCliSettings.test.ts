import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
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
    expect(invoke).toHaveBeenCalledWith('chat_detect_external_agent_models', expect.objectContaining({ force: false }))
  })

  it('shows the snapshot before a slow refresh finishes', async () => {
    const snapshot = { models: [{ id: 'saved', label: 'Saved' }], refreshing: true }
    let finish: (value: unknown) => void = () => {}
    vi.mocked(invoke).mockResolvedValueOnce(snapshot).mockImplementationOnce(() => new Promise((resolve) => { finish = resolve }))
    const onSnapshot = vi.fn()
    const result = externalCliSettingsApi.detectExternalAgentModels('codex', 'project', false, onSnapshot)
    await vi.waitFor(() => expect(onSnapshot).toHaveBeenCalledWith(expect.objectContaining(snapshot)))
    expect(invoke).toHaveBeenNthCalledWith(1, 'chat_detect_external_agent_models', expect.objectContaining({ snapshotOnly: true }))
    finish({ models: [{ id: 'new', label: 'New' }], source: 'probed' })
    expect((await result).models[0].id).toBe('new')
  })

  it('does not probe again when the snapshot is fresh', async () => {
    vi.mocked(invoke).mockResolvedValue({ models: [{ id: 'cached', label: 'Cached' }], refreshing: false })
    expect((await externalCliSettingsApi.detectExternalAgentModels('codex', null, false, vi.fn())).models[0].id).toBe('cached')
    expect(invoke).toHaveBeenCalledTimes(1)
  })

  it('executes the displayed plan and reads only selected cc-switch records', async () => {
    await externalCliSettingsApi.externalCliInstall('claude', 'displayed-plan')
    expect(invoke).toHaveBeenCalledWith('chat_external_cli_install', {
      agentId: 'claude', planId: 'displayed-plan',
    })
    const selections = [{ agentId: 'claude', providerId: 'selected' }]
    await externalCliSettingsApi.externalCliImportCcSwitch(selections)
    expect(invoke).toHaveBeenCalledWith('chat_external_cli_import_cc_switch', { selections })
  })

  it('normalizes missing agents from a native snapshot', async () => {
    vi.mocked(invoke).mockResolvedValue({ success: true })
    expect(await externalCliSettingsApi.detectExternalAgents(true)).toEqual([])
  })

  it('does not subscribe to agent updates in browser preview', async () => {
    vi.mocked(isTauriRuntime).mockReturnValue(false)
    const unlisten = await onExternalAgentsUpdated(vi.fn())
    expect(listen).not.toHaveBeenCalled()
    unlisten()
  })
})
