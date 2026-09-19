import { describe, expect, it, vi } from 'vitest'
import type { Settings, SettingsSnapshot } from '../api/tauri'
import { SettingsEditorController, type SettingsEditorPort } from './SettingsEditorController'

function settings(theme: Settings['theme'] = 'light'): Settings {
  return {
    theme,
    settingsLanguage: 'zh',
    favoriteModels: [],
    providers: [],
    chatTools: { servers: [] },
  } as unknown as Settings
}

function snapshot(value: Settings, revision: number): SettingsSnapshot {
  return { settings: value, version: { epoch: 'boot', revision } }
}

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}

function port(initial: SettingsSnapshot, save: SettingsEditorPort['save']): SettingsEditorPort {
  return {
    peek: () => initial,
    load: async () => initial,
    refresh: async () => initial,
    save,
    subscribe: () => () => {},
  }
}

describe('SettingsEditorController', () => {
  it('ordinary close waits for edits made during an in-flight save', async () => {
    const first = deferred<SettingsSnapshot>()
    const second = deferred<SettingsSnapshot>()
    const save = vi.fn()
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise)
    const controller = new SettingsEditorController(port(snapshot(settings(), 1), save))
    controller.start()
    controller.edit((draft) => ({ ...draft, theme: 'dark' }))
    const pending = controller.flush()
    controller.edit((draft) => ({ ...draft, theme: 'system' }))
    const closed = vi.fn()
    const closing = controller.requestClose(closed)

    expect(closed).not.toHaveBeenCalled()
    first.resolve(snapshot(settings('dark'), 2))
    await Promise.resolve()
    expect(closed).not.toHaveBeenCalled()
    second.resolve(snapshot(settings('system'), 3))
    await Promise.all([pending, closing])
    expect(closed).toHaveBeenCalledOnce()
    expect(controller.snapshot.settings?.theme).toBe('system')
    expect(controller.snapshot.hasUnsavedChanges).toBe(false)
    controller.dispose()
  })

  it('navigation close returns immediately while the pending edit continues saving', async () => {
    const pendingSave = deferred<SettingsSnapshot>()
    const controller = new SettingsEditorController(port(
      snapshot(settings(), 1),
      () => pendingSave.promise,
    ))
    controller.start()
    controller.edit((draft) => ({ ...draft, theme: 'dark' }))
    const closed = vi.fn()
    const finished = controller.requestClose(closed, { waitForSave: false })

    expect(closed).toHaveBeenCalledOnce()
    expect(controller.snapshot.hasUnsavedChanges).toBe(true)
    pendingSave.resolve(snapshot(settings('dark'), 2))
    await finished
    expect(controller.snapshot.hasUnsavedChanges).toBe(false)
    controller.dispose()
  })

  it('accepts canonical corrections while preserving an unfinished provider row', async () => {
    const initial = snapshot(settings(), 1)
    const save = vi.fn(async () => snapshot({
      ...settings('dark'),
      providers: [],
      retryAttempts: 8,
    } as Settings, 2))
    const controller = new SettingsEditorController(port(initial, save))
    controller.start()
    controller.edit((draft) => ({
      ...draft,
      theme: 'dark',
      retryAttempts: 999,
      providers: [{ id: 'unfinished', apiKeys: [''] }],
    } as Settings))

    expect(await controller.flush()).toBe(true)
    expect(controller.snapshot.settings?.retryAttempts).toBe(8)
    expect(controller.snapshot.settings?.providers).toEqual([{ id: 'unfinished', apiKeys: [''] }])
    expect(controller.snapshot.hasUnsavedChanges).toBe(false)
    controller.dispose()
  })

  it('rebases consecutive external favorites without sending a stale full save', () => {
    const initial = snapshot(settings(), 1)
    let receive!: (value: SettingsSnapshot) => void
    const save = vi.fn(async () => initial)
    const controller = new SettingsEditorController({
      ...port(initial, save),
      subscribe: (listener) => { receive = listener; return () => {} },
    })
    controller.start()
    receive(snapshot({ ...settings(), favoriteModels: ['one'] }, 2))
    receive(snapshot({ ...settings(), favoriteModels: ['one', 'two'] }, 3))

    expect(controller.snapshot.settings?.favoriteModels).toEqual(['one', 'two'])
    expect(controller.snapshot.hasUnsavedChanges).toBe(false)
    expect(save).not.toHaveBeenCalled()
    controller.dispose()
  })

  it('keeps same-field conflict visible and never overwrites the remote value', async () => {
    const initial = snapshot(settings(), 1)
    let receive!: (value: SettingsSnapshot) => void
    const save = vi.fn(async () => initial)
    const controller = new SettingsEditorController({
      ...port(initial, save),
      subscribe: (listener) => { receive = listener; return () => {} },
    })
    controller.start()
    controller.edit((draft) => ({ ...draft, theme: 'dark' }))
    receive(snapshot(settings('system'), 2))

    expect(controller.snapshot.settings?.theme).toBe('dark')
    expect(controller.snapshot.conflicts.map((conflict) => conflict.path)).toEqual(['theme'])
    expect(await controller.flush()).toBe(false)
    expect(save).not.toHaveBeenCalled()
    controller.dispose()
  })

  it('preserves a failed edit and retries it on a later flush', async () => {
    const initial = snapshot(settings(), 1)
    const save = vi.fn()
      .mockRejectedValueOnce(new Error('disk unavailable'))
      .mockResolvedValueOnce(snapshot(settings('dark'), 2))
    const controller = new SettingsEditorController(port(initial, save))
    controller.start()
    controller.edit((draft) => ({ ...draft, theme: 'dark' }))

    expect(await controller.flush()).toBe(false)
    expect(controller.snapshot.settings?.theme).toBe('dark')
    expect(controller.snapshot.hasUnsavedChanges).toBe(true)
    expect(controller.snapshot.saveError).toContain('disk unavailable')
    expect(await controller.flush()).toBe(true)
    expect(controller.snapshot.hasUnsavedChanges).toBe(false)
    controller.dispose()
  })

  it('keeps ordinary close open on failed flush so the draft can be retried', async () => {
    const initial = snapshot(settings(), 1)
    const save = vi.fn()
      .mockRejectedValueOnce(new Error('disk unavailable'))
      .mockResolvedValueOnce(snapshot(settings('dark'), 2))
    const controller = new SettingsEditorController(port(initial, save))
    controller.start()
    controller.edit((draft) => ({ ...draft, theme: 'dark' }))
    const closed = vi.fn()

    await controller.requestClose(closed)
    expect(closed).not.toHaveBeenCalled()
    expect(controller.snapshot.settings?.theme).toBe('dark')
    expect(controller.snapshot.saveError).toContain('disk unavailable')
    await controller.requestClose(closed)
    expect(closed).toHaveBeenCalledOnce()
    controller.dispose()
  })

  it('coalesces repeated close requests and does not close a new view twice', async () => {
    const pending = deferred<SettingsSnapshot>()
    const controller = new SettingsEditorController(port(snapshot(settings(), 1), () => pending.promise))
    controller.start()
    controller.edit((draft) => ({ ...draft, theme: 'dark' }))
    const closed = vi.fn()
    const first = controller.requestClose(closed)
    const second = controller.requestClose(closed)
    pending.resolve(snapshot(settings('dark'), 2))
    await Promise.all([first, second])
    expect(closed).toHaveBeenCalledOnce()
    controller.dispose()
  })

  it('navigation close still leaves after a previous keep-alive close issued', async () => {
    const controller = new SettingsEditorController(port(snapshot(settings(), 1), async () => snapshot(settings(), 2)))
    controller.start()
    const closed = vi.fn()
    await controller.requestClose(closed, { waitForSave: false })
    expect(closed).toHaveBeenCalledOnce()
    await controller.requestClose(closed, { waitForSave: false })
    expect(closed).toHaveBeenCalledTimes(2)
    controller.dispose()
  })

  it('does not close a reopened view when an older flush finishes late', async () => {
    const pending = deferred<SettingsSnapshot>()
    const controller = new SettingsEditorController(port(snapshot(settings(), 1), () => pending.promise))
    controller.start()
    controller.edit((draft) => ({ ...draft, theme: 'dark' }))
    const closed = vi.fn()
    const oldClose = controller.requestClose(closed)
    controller.start()
    pending.resolve(snapshot(settings('dark'), 2))
    await oldClose
    expect(closed).not.toHaveBeenCalled()
    controller.dispose()
  })

  it('does not import over an unsaved draft when the prerequisite flush fails', async () => {
    const initial = snapshot(settings(), 1)
    const importSettings = vi.fn(async () => snapshot(settings('system'), 2))
    const controller = new SettingsEditorController({
      ...port(initial, async () => { throw new Error('disk unavailable') }),
      import: importSettings,
    })
    controller.start()
    controller.edit((draft) => ({ ...draft, theme: 'dark' }))

    await expect(controller.import('backup.json')).rejects.toThrow('Save failed')
    expect(importSettings).not.toHaveBeenCalled()
    expect(controller.snapshot.settings?.theme).toBe('dark')
    expect(controller.snapshot.hasUnsavedChanges).toBe(true)
    controller.dispose()
  })

  it('retries a version conflict against the fresh snapshot without losing unrelated remote edits', async () => {
    const initial = snapshot(settings(), 1)
    const remote = snapshot({ ...settings(), favoriteModels: ['one'] }, 2)
    const saved = snapshot({ ...settings('dark'), favoriteModels: ['one'] }, 3)
    const save = vi.fn()
      .mockRejectedValueOnce({ code: 'versionConflict', message: 'stale', expectedVersion: initial.version, actualVersion: remote.version })
      .mockResolvedValueOnce(saved)
    const controller = new SettingsEditorController({
      ...port(initial, save),
      refresh: async () => remote,
    })
    controller.start()
    controller.edit((draft) => ({ ...draft, theme: 'dark' }))

    expect(await controller.flush()).toBe(true)
    expect(controller.snapshot.settings?.theme).toBe('dark')
    expect(controller.snapshot.settings?.favoriteModels).toEqual(['one'])
    expect(save).toHaveBeenCalledTimes(2)
    expect(save.mock.calls[1][1]).toEqual(remote.version)
    controller.dispose()
  })

  it('does not treat its own save notification as a remote conflict', async () => {
    const initial = snapshot(settings(), 1)
    let receive!: (value: SettingsSnapshot) => void
    const saved = snapshot(settings('dark'), 2)
    const controller = new SettingsEditorController({
      ...port(initial, async () => {
        receive(saved)
        return saved
      }),
      subscribe: (listener) => { receive = listener; return () => {} },
    })
    controller.start()
    controller.edit((draft) => ({ ...draft, theme: 'dark' }))

    expect(await controller.flush()).toBe(true)
    expect(controller.snapshot.conflicts).toEqual([])
    expect(controller.snapshot.hasUnsavedChanges).toBe(false)
    controller.dispose()
  })
})
