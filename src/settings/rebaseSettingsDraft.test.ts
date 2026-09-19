import { describe, expect, it } from 'vitest'
import type { ChatMcpServer, Settings } from '../api/tauri'
import {
  acceptSettingsSave,
  createSettingsEditorState,
  updateSettingsEditorDraft,
  receiveSettingsSnapshot,
} from './rebaseSettingsDraft'

function settings(partial: Record<string, unknown>): Settings {
  return {
    theme: 'light',
    settingsLanguage: 'zh',
    favoriteModels: [],
    chatProviderId: 'p1',
    chatModel: 'm1',
    defaultModels: {
      chat: { providerId: 'p1', model: 'm1' },
      vision: { providerId: '', model: '' },
      videoAnalysis: { providerId: '', model: '' },
      titleSummary: { providerId: '', model: '' },
      compression: { providerId: '', model: '' },
      imageGeneration: { providerId: '', model: '' },
      promptOptimize: { providerId: '', model: '' },
      advisor: { providerId: '', model: '' },
    },
    chatTools: {
      enabled: false,
      servers: [],
      skillScanPaths: [],
      disabledSkillIds: [],
      maxToolRounds: null,
      toolTimeoutMs: 60_000,
      approvalPolicy: 'auto',
      nativeTools: { skillRuntime: false, runCommand: false, webSearch: false },
    },
    lens: { enabled: true, hotkey: '', webSearch: { enabled: false, provider: 'tavily' } },
    screenshotTranslation: { cardWidth: 480 },
    providers: [],
    ...partial,
  } as unknown as Settings
}

function server(partial: Partial<ChatMcpServer> & Pick<ChatMcpServer, 'id'>): ChatMcpServer {
  return {
    name: partial.id,
    enabled: true,
    transport: 'stdio',
    url: '',
    command: 'bin',
    args: [],
    env: {},
    headers: {},
    enabledTools: [],
    ...partial,
  }
}

describe('settings editor canonical state', () => {
  describe.each(['providers', 'chatTools.servers'] as const)('%s keyed merge', (path) => {
    const withRows = (rows: ChatMcpServer[]) => path === 'providers'
      ? settings({ providers: rows })
      : settings({ chatTools: { ...settings({}).chatTools, servers: rows } })
    const rowsOf = (value: Settings) => path === 'providers' ? value.providers : value.chatTools.servers
    const merge = (base: ChatMcpServer[], local: ChatMcpServer[], remote: ChatMcpServer[]) => (
      receiveSettingsSnapshot(
        updateSettingsEditorDraft(createSettingsEditorState(withRows(base)), withRows(local)),
        withRows(remote),
      )
    )

    it('preserves additions from both sides alongside a remote edit', () => {
      const original = server({ id: 'one' })
      const remoteEdit = { ...original, name: 'Remote' }
      const localNew = server({ id: 'local-new' })
      const remoteNew = server({ id: 'remote-new' })
      const next = merge([original], [original, localNew], [remoteEdit, remoteNew])

      expect(rowsOf(next.draft)).toEqual([remoteEdit, remoteNew, localNew])
      expect(next.conflicts).toEqual([])
    })

    it.each(['local', 'remote'])('does not resurrect an unchanged entity deleted by %s', (side) => {
      const original = server({ id: 'one' })
      const added = server({ id: 'new' })
      const next = side === 'local'
        ? merge([original], [], [original, added])
        : merge([original], [original, added], [])

      expect(rowsOf(next.draft)).toEqual([added])
      expect(next.conflicts).toEqual([])
    })

    it('reports different additions with the same id as a conflict', () => {
      const local = server({ id: 'new', name: 'Local' })
      const remote = server({ id: 'new', name: 'Remote' })
      const next = merge([], [local], [remote])

      expect(rowsOf(next.draft)).toEqual([local])
      expect(next.conflicts).toEqual([{ path: `${path}.new`, base: undefined, local, remote }])
    })

    it.each(['local', 'remote'])('retains a %s deletion conflict through unrelated snapshots', (side) => {
      const original = server({ id: 'one' })
      const edited = { ...original, name: 'Edited' }
      const remote = withRows(side === 'local' ? [edited] : [])
      const conflicted = merge([original], side === 'local' ? [] : [edited], rowsOf(remote) as ChatMcpServer[])
      const next = receiveSettingsSnapshot(conflicted, { ...remote, favoriteModels: ['m1'] })

      expect(rowsOf(next.draft)).toEqual(side === 'local' ? [] : [edited])
      expect(next.conflicts).toEqual(conflicted.conflicts)
      expect(next.conflicts).toHaveLength(1)
    })
  })

  it('retains unresolved conflicts, refreshes remote values, and clears them on convergence', () => {
    const edited = updateSettingsEditorDraft(createSettingsEditorState(settings({})), settings({ theme: 'dark' }))
    const conflicted = receiveSettingsSnapshot(edited, settings({ theme: 'system' }))
    const unrelated = receiveSettingsSnapshot(conflicted, settings({ theme: 'system', favoriteModels: ['m1'] }))
    expect(unrelated.conflicts).toEqual(conflicted.conflicts)
    expect(unrelated.draft.favoriteModels).toEqual(['m1'])

    const changedAgain = receiveSettingsSnapshot(unrelated, settings({ theme: 'light', favoriteModels: ['m1'] }))
    expect(changedAgain.conflicts).toHaveLength(1)
    expect(changedAgain.conflicts[0]).toMatchObject({ path: 'theme', local: 'dark', remote: 'light' })
    expect(updateSettingsEditorDraft(changedAgain, { ...changedAgain.draft, theme: 'system' }).conflicts).toEqual([])

    const converged = receiveSettingsSnapshot(changedAgain, settings({ theme: 'dark', favoriteModels: ['m1'] }))
    expect(converged.conflicts).toEqual([])
    expect(converged.draft).toEqual(converged.acknowledgedDraft)
  })

  it('advances the canonical baseline across consecutive external snapshots', () => {
    const initial = settings({ favoriteModels: [] })
    const first = settings({ favoriteModels: ['one'] })
    const second = settings({ favoriteModels: ['one', 'two'] })

    const afterFirst = receiveSettingsSnapshot(createSettingsEditorState(initial), first)
    const afterSecond = receiveSettingsSnapshot(afterFirst, second)

    expect(afterSecond.canonical.favoriteModels).toEqual(['one', 'two'])
    expect(afterSecond.acknowledgedDraft.favoriteModels).toEqual(['one', 'two'])
    expect(afterSecond.draft.favoriteModels).toEqual(['one', 'two'])
    expect(afterSecond.conflicts).toEqual([])
  })

  it('accepts canonical corrections while retaining acknowledged placeholder input', () => {
    const submitted = settings({
      retryAttempts: 999,
      providers: [{ id: 'draft-provider', apiKeys: [''] }],
    })
    const canonical = settings({ retryAttempts: 10, providers: [] })

    const next = acceptSettingsSave(
      submitted,
      canonical,
      submitted,
    )

    expect(next.canonical.retryAttempts).toBe(10)
    expect(next.draft.retryAttempts).toBe(10)
    expect(next.draft.providers).toEqual([{ id: 'draft-provider', apiKeys: [''] }])
    expect(next.acknowledgedDraft).toEqual(next.draft)
  })

  it('merges different provider entities and reports a same-field conflict', () => {
    const initial = settings({
      providers: [
        { id: 'one', name: 'One', apiKeys: ['a'] },
        { id: 'two', name: 'Two', apiKeys: ['b'] },
      ],
    })
    const local = settings({
      providers: [
        { id: 'one', name: 'Local One', apiKeys: ['a'] },
        { id: 'two', name: 'Two', apiKeys: ['b'] },
      ],
    })
    const remoteDifferentEntity = settings({
      providers: [
        { id: 'one', name: 'One', apiKeys: ['a'] },
        { id: 'two', name: 'Remote Two', apiKeys: ['b'] },
      ],
    })

    const merged = receiveSettingsSnapshot(
      createSettingsEditorState(initial, local),
      remoteDifferentEntity,
    )
    expect(merged.draft.providers.find((provider) => provider.id === 'one')?.name).toBe('Local One')
    expect(merged.draft.providers.find((provider) => provider.id === 'two')?.name).toBe('Remote Two')
    expect(merged.conflicts).toEqual([])

    const remoteSameField = settings({
      providers: [
        { id: 'one', name: 'Remote One', apiKeys: ['a'] },
        { id: 'two', name: 'Two', apiKeys: ['b'] },
      ],
    })
    const conflicted = receiveSettingsSnapshot(
      createSettingsEditorState(initial, local),
      remoteSameField,
    )
    expect(conflicted.draft.providers.find((provider) => provider.id === 'one')?.name).toBe('Local One')
    expect(conflicted.conflicts.map((conflict) => conflict.path)).toContain('providers.one.name')
  })

  it('merges different fields of the same provider without replacing the entity', () => {
    const initial = settings({
      providers: [{ id: 'one', name: 'One', apiKeys: ['old'], enabledModels: ['m1'] }],
    })
    const local = settings({
      providers: [{ id: 'one', name: 'Local', apiKeys: ['old'], enabledModels: ['m1'] }],
    })
    const remote = settings({
      providers: [{ id: 'one', name: 'One', apiKeys: ['new'], enabledModels: ['m1'] }],
    })

    const next = receiveSettingsSnapshot(createSettingsEditorState(initial, local), remote)
    expect(next.draft.providers).toEqual([
      { id: 'one', name: 'Local', apiKeys: ['new'], enabledModels: ['m1'] },
    ])
    expect(next.conflicts).toEqual([])
  })

  it('keeps a locally edited entity and surfaces a concurrent remote deletion', () => {
    const initial = settings({ providers: [{ id: 'one', name: 'One', apiKeys: ['old'] }] })
    const local = settings({ providers: [{ id: 'one', name: 'Local', apiKeys: ['old'] }] })
    const remote = settings({ providers: [] })

    const next = receiveSettingsSnapshot(createSettingsEditorState(initial, local), remote)
    expect(next.draft.providers).toEqual([{ id: 'one', name: 'Local', apiKeys: ['old'] }])
    expect(next.conflicts.map((conflict) => conflict.path)).toContain('providers.one')
  })

  it('replays edits made while a save is in flight onto the canonical response', () => {
    const submitted = settings({ theme: 'dark', retryAttempts: 2 })
    const latest = settings({ theme: 'dark', retryAttempts: 5 })
    const saved = settings({ theme: 'dark', retryAttempts: 2 })

    const next = acceptSettingsSave(
      submitted,
      saved,
      latest,
    )
    expect(next.canonical).toEqual(saved)
    expect(next.acknowledgedDraft).toEqual(submitted)
    expect(next.draft.retryAttempts).toBe(5)
    expect(next.conflicts).toEqual([])
  })

  it('keeps a conflict until the user changes the conflicting value', () => {
    const initial = settings({ theme: 'light' })
    const local = settings({ theme: 'dark' })
    const conflicted = receiveSettingsSnapshot(
      createSettingsEditorState(initial, local),
      settings({ theme: 'system' }),
    )
    expect(updateSettingsEditorDraft(conflicted, local).conflicts).toHaveLength(1)
    expect(updateSettingsEditorDraft(conflicted, settings({ theme: 'system' })).conflicts).toEqual([])
  })

  it('always adopts plugin-managed MCP state from the canonical snapshot', () => {
    const plugin = server({ id: 'plugin-cua-driver', connectorId: 'plugin:cua-driver', enabled: true })
    const user = server({ id: 'mine', enabled: true })
    const initial = settings({ chatTools: { ...settings({}).chatTools, servers: [plugin, user] } })
    const local = settings({
      chatTools: {
        ...initial.chatTools,
        servers: [plugin, { ...user, enabled: false }],
      },
    })
    const fresh = settings({
      chatTools: {
        ...initial.chatTools,
        servers: [{ ...plugin, enabled: false }, user],
      },
    })
    const next = receiveSettingsSnapshot(createSettingsEditorState(initial, local), fresh)
    expect(next.draft.chatTools.servers.find((row) => row.id === 'plugin-cua-driver')?.enabled).toBe(false)
    expect(next.draft.chatTools.servers.find((row) => row.id === 'mine')?.enabled).toBe(false)
    expect(next.conflicts).toEqual([])
  })
})
