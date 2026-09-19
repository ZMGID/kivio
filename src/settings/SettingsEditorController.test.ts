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
  it('persists both windows\' new providers after a remote edit', async () => {
    const provider = { id: 'one', name: 'One', apiKeys: ['key'] } as Settings['providers'][number]
    const initial = snapshot({ ...settings(), providers: [provider] }, 1)
    const localNew = { ...provider, id: 'local-new' }
    const remoteNew = { ...provider, id: 'remote-new' }
    const remoteEdit = { ...provider, name: 'Remote' }
    let receive!: (value: SettingsSnapshot) => void
    const save = vi.fn(async (draft: Settings) => snapshot(draft, 3))
    const controller = new SettingsEditorController({
      ...port(initial, save),
      subscribe: (listener) => { receive = listener; return () => {} },
    })
    try {
      controller.start()
      await Promise.resolve()
      controller.edit((draft) => ({ ...draft, providers: [...draft.providers, localNew] }))
      receive(snapshot({ ...settings(), providers: [remoteEdit, remoteNew] }, 2))

      expect(await controller.flush()).toBe(true)
      expect(save).toHaveBeenCalledOnce()
      expect(save.mock.calls[0][0].providers).toEqual([remoteEdit, remoteNew, localNew])
      expect(controller.snapshot.hasUnsavedChanges).toBe(false)
    } finally {
      controller.dispose()
    }
  })

  it('does not autosave an unresolved conflict after unrelated notifications', async () => {
    vi.useFakeTimers()
    const initial = snapshot(settings(), 1)
    let receive!: (value: SettingsSnapshot) => void
    const save = vi.fn(async (draft: Settings) => snapshot(draft, 5))
    const controller = new SettingsEditorController({
      ...port(initial, save),
      subscribe: (listener) => { receive = listener; return () => {} },
    })
    try {
      controller.start()
      await Promise.resolve()
      controller.edit((draft) => ({ ...draft, theme: 'dark' }))
      receive(snapshot(settings('system'), 2))
      receive(snapshot({ ...settings('system'), favoriteModels: ['one'] }, 3))
      receive(snapshot({ ...settings('system'), favoriteModels: ['one', 'two'] }, 4))
      await vi.advanceTimersByTimeAsync(800)

      expect(save).not.toHaveBeenCalled()
      expect(await controller.flush()).toBe(false)
      expect(controller.snapshot.conflicts.map((conflict) => conflict.path)).toEqual(['theme'])
      expect(controller.snapshot.settings?.favoriteModels).toEqual(['one', 'two'])
    } finally {
      controller.dispose()
      vi.useRealTimers()
    }
  })

  it.each([true, false])('ordinary close works after a keep-alive close (waitForSave=%s)', async (waitForSave) => {
    const controller = new SettingsEditorController(port(snapshot(settings(), 1), async () => snapshot(settings(), 2)))
    controller.start()
    const closed = vi.fn()
    try {
      await controller.requestClose(closed, { waitForSave })
      await controller.requestClose(closed)
      expect(closed).toHaveBeenCalledTimes(2)
    } finally {
      controller.dispose()
    }
  })

  it('navigation supersedes an ordinary close still waiting for a save', async () => {
    const pending = deferred<SettingsSnapshot>()
    const controller = new SettingsEditorController(port(snapshot(settings(), 1), () => pending.promise))
    controller.start()
    controller.edit((draft) => ({ ...draft, theme: 'dark' }))
    const ordinaryClose = vi.fn()
    const navigationClose = vi.fn()
    const ordinary = controller.requestClose(ordinaryClose)
    const navigation = controller.requestClose(navigationClose, { waitForSave: false })
    expect(navigationClose).toHaveBeenCalledOnce()
    pending.resolve(snapshot(settings('dark'), 2))
    await Promise.all([ordinary, navigation])
    expect(ordinaryClose).not.toHaveBeenCalled()
    controller.dispose()
  })

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

describe('T5 two windows share one CAS store', () => {
  // The product only has one Settings page, but each webview owns its own
  // settings cache. T5 is two of those clients submitting through CAS.
  function createCasStore(initial: SettingsSnapshot) {
    let current = structuredClone(initial)
    const listeners = new Set<(value: SettingsSnapshot) => void>()
    const attempts: Array<{ draft: Settings; version: { epoch: string; revision: number } }> = []

    const persist = (): SettingsSnapshot => structuredClone(current)

    const commit = (draft: Settings, expectedVersion: SettingsSnapshot['version']): SettingsSnapshot => {
      attempts.push({ draft: structuredClone(draft), version: { ...expectedVersion } })
      if (
        expectedVersion.epoch !== current.version.epoch
        || expectedVersion.revision !== current.version.revision
      ) {
        throw {
          code: 'versionConflict',
          message: 'stale settings',
          expectedVersion: { ...expectedVersion },
          actualVersion: { ...current.version },
        }
      }
      current = {
        settings: structuredClone(draft),
        version: { epoch: current.version.epoch, revision: current.version.revision + 1 },
      }
      const next = persist()
      for (const listener of listeners) listener(persist())
      return next
    }

    const connect = (gate?: { wait: () => Promise<void> }): SettingsEditorPort => {
      let cached = persist()
      return {
        peek: () => structuredClone(cached),
        load: async () => {
          cached = persist()
          return structuredClone(cached)
        },
        refresh: async () => {
          cached = persist()
          return structuredClone(cached)
        },
        save: async (draft, expectedVersion) => {
          if (gate) await gate.wait()
          const saved = commit(draft, expectedVersion)
          cached = saved
          return structuredClone(saved)
        },
        subscribe: (listener) => {
          const wrapped = (value: SettingsSnapshot) => {
            if (
              value.version.epoch !== cached.version.epoch
              || value.version.revision > cached.version.revision
            ) {
              cached = structuredClone(value)
            }
            listener(structuredClone(value))
          }
          listeners.add(wrapped)
          return () => { listeners.delete(wrapped) }
        },
      }
    }

    return { persist, connect, attempts }
  }

  it('lets B rebase a non-conflicting edit after A commits first', async () => {
    const store = createCasStore(snapshot(settings(), 1))
    const bEntered = deferred<void>()
    const bMayEnter = deferred<void>()
    const windowA = new SettingsEditorController(store.connect())
    const windowB = new SettingsEditorController(store.connect({
      wait: async () => {
        bEntered.resolve()
        await bMayEnter.promise
      },
    }))
    windowA.start()
    windowB.start()
    windowA.edit((draft) => ({ ...draft, theme: 'dark' }))
    windowB.edit((draft) => ({ ...draft, favoriteModels: ['one'] }))

    const bFlush = windowB.flush()
    await bEntered.promise
    expect(await windowA.flush()).toBe(true)
    expect(store.persist().settings.theme).toBe('dark')
    expect(store.persist().version.revision).toBe(2)

    bMayEnter.resolve()
    expect(await bFlush).toBe(true)

    expect(store.attempts[0]?.version).toEqual({ epoch: 'boot', revision: 1 })
    expect(store.attempts[1]?.version).toEqual({ epoch: 'boot', revision: 1 })
    expect(store.persist().settings.theme).toBe('dark')
    expect(store.persist().settings.favoriteModels).toEqual(['one'])
    expect(store.persist().version.revision).toBe(3)
    expect(windowA.snapshot.settings?.theme).toBe('dark')
    expect(windowA.snapshot.settings?.favoriteModels).toEqual(['one'])
    expect(windowB.snapshot.settings?.theme).toBe('dark')
    expect(windowB.snapshot.settings?.favoriteModels).toEqual(['one'])
    expect(windowA.snapshot.hasUnsavedChanges).toBe(false)
    expect(windowB.snapshot.hasUnsavedChanges).toBe(false)
    expect(windowB.snapshot.conflicts).toEqual([])
    windowA.dispose()
    windowB.dispose()
  })

  it('keeps B\'s same-field draft and does not let the stale submit overwrite A', async () => {
    const store = createCasStore(snapshot(settings(), 1))
    const bEntered = deferred<void>()
    const bMayEnter = deferred<void>()
    const windowA = new SettingsEditorController(store.connect())
    const windowB = new SettingsEditorController(store.connect({
      wait: async () => {
        bEntered.resolve()
        await bMayEnter.promise
      },
    }))
    windowA.start()
    windowB.start()
    windowA.edit((draft) => ({ ...draft, theme: 'dark' }))
    windowB.edit((draft) => ({ ...draft, theme: 'system' }))

    const bFlush = windowB.flush()
    await bEntered.promise
    expect(await windowA.flush()).toBe(true)
    expect(store.persist().settings.theme).toBe('dark')

    bMayEnter.resolve()
    expect(await bFlush).toBe(false)

    expect(store.attempts.map((attempt) => attempt.version.revision)).toEqual([1, 1])
    expect(store.persist().settings.theme).toBe('dark')
    expect(store.persist().version.revision).toBe(2)
    expect(windowA.snapshot.settings?.theme).toBe('dark')
    expect(windowB.snapshot.settings?.theme).toBe('system')
    expect(windowB.snapshot.conflicts.map((conflict) => conflict.path)).toEqual(['theme'])
    expect(windowB.snapshot.hasUnsavedChanges).toBe(true)
    expect(windowB.snapshot.saveError).toContain('theme')
    windowA.dispose()
    windowB.dispose()
  })
})
