import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Settings, SettingsSnapshot, SettingsVersion } from './tauri'

const getSettingsMock = vi.fn()
const saveSettingsMock = vi.fn()
const importSettingsMock = vi.fn()
const setFavoriteModelsMock = vi.fn()
const setTranslateCardSizeMock = vi.fn()
const onKivioSettingsChangedMock = vi.fn()

vi.mock('./tauri', () => ({
  api: {
    getSettings: (...args: unknown[]) => getSettingsMock(...args),
    saveSettings: (...args: unknown[]) => saveSettingsMock(...args),
    importSettings: (...args: unknown[]) => importSettingsMock(...args),
    setFavoriteModels: (...args: unknown[]) => setFavoriteModelsMock(...args),
    setTranslateCardSize: (...args: unknown[]) => setTranslateCardSizeMock(...args),
    onKivioSettingsChanged: (...args: unknown[]) => onKivioSettingsChangedMock(...args),
  },
  isSettingsVersionConflict: (error: unknown) => (
    typeof error === 'object'
    && error !== null
    && (error as { code?: unknown }).code === 'versionConflict'
  ),
}))

import {
  __resetSettingsCacheForTest,
  getSettingsCached,
  getSettingsSnapshotCached,
  importSettingsCached,
  peekSettings,
  peekSettingsSnapshot,
  refreshSettings,
  refreshSettingsSnapshot,
  saveSettingsCached,
  setFavoriteModelsCached,
  setTranslateCardSizeCached,
  startBackendSettingsSync,
  subscribeSettings,
  subscribeSettingsSnapshot,
  updateSettingsCached,
} from './settingsCache'

const settingsA = { theme: 'dark', providers: [], favoriteModels: [] } as unknown as Settings
const settingsB = { theme: 'light', providers: [], favoriteModels: [] } as unknown as Settings
const version = (revision: number, epoch = 'epoch-a'): SettingsVersion => ({ epoch, revision })
const snapshot = (settings: Settings, revision: number, epoch = 'epoch-a'): SettingsSnapshot => ({
  settings,
  version: version(revision, epoch),
})

beforeEach(() => {
  __resetSettingsCacheForTest()
  getSettingsMock.mockReset()
  saveSettingsMock.mockReset()
  importSettingsMock.mockReset()
  setFavoriteModelsMock.mockReset()
  setTranslateCardSizeMock.mockReset()
  onKivioSettingsChangedMock.mockReset()
})

describe('settingsCache versioned snapshots', () => {
  it('deduplicates the initial read and exposes Settings-only compatibility views', async () => {
    getSettingsMock.mockResolvedValue(snapshot(settingsA, 1))
    const [first, second] = await Promise.all([getSettingsCached(), getSettingsSnapshotCached()])
    expect(first).toBe(settingsA)
    expect(second).toEqual(snapshot(settingsA, 1))
    expect(peekSettings()).toBe(settingsA)
    expect(peekSettingsSnapshot()).toEqual(snapshot(settingsA, 1))
    expect(getSettingsMock).toHaveBeenCalledTimes(1)
  })

  it('does not let a late older read roll the cache back within one epoch', async () => {
    let finishOld: ((value: SettingsSnapshot) => void) | undefined
    getSettingsMock
      .mockImplementationOnce(() => new Promise<SettingsSnapshot>((resolve) => { finishOld = resolve }))
      .mockResolvedValueOnce(snapshot(settingsB, 2))
    const oldRead = getSettingsSnapshotCached()
    await expect(refreshSettingsSnapshot()).resolves.toEqual(snapshot(settingsB, 2))
    finishOld?.(snapshot(settingsA, 1))
    await expect(oldRead).resolves.toEqual(snapshot(settingsB, 2))
    expect(peekSettingsSnapshot()).toEqual(snapshot(settingsB, 2))
  })

  it('does not let a late save response roll back a newer canonical snapshot', async () => {
    getSettingsMock
      .mockResolvedValueOnce(snapshot(settingsA, 1))
      .mockResolvedValueOnce(snapshot(settingsB, 3))
    await getSettingsSnapshotCached()
    let finishSave: ((value: SettingsSnapshot) => void) | undefined
    saveSettingsMock.mockImplementationOnce(() => new Promise<SettingsSnapshot>((resolve) => { finishSave = resolve }))

    const saving = saveSettingsCached(settingsB, version(1))
    await refreshSettingsSnapshot()
    finishSave?.(snapshot(settingsA, 2))

    await expect(saving).resolves.toBe(settingsB)
    expect(peekSettingsSnapshot()).toEqual(snapshot(settingsB, 3))
  })

  it('uses request order to reject a late response from an earlier backend epoch', async () => {
    let finishOldEpoch: ((value: SettingsSnapshot) => void) | undefined
    getSettingsMock
      .mockImplementationOnce(() => new Promise<SettingsSnapshot>((resolve) => { finishOldEpoch = resolve }))
      .mockResolvedValueOnce(snapshot(settingsB, 1, 'epoch-b'))
    const oldRead = getSettingsSnapshotCached()
    await refreshSettingsSnapshot()
    finishOldEpoch?.(snapshot(settingsA, 99, 'epoch-a'))
    await expect(oldRead).resolves.toEqual(snapshot(settingsB, 1, 'epoch-b'))
    expect(peekSettingsSnapshot()?.version.epoch).toBe('epoch-b')
  })

  it('notifies both snapshot subscribers and Settings-only compatibility subscribers', async () => {
    getSettingsMock.mockResolvedValue(snapshot(settingsB, 2))
    const settingsListener = vi.fn()
    const snapshotListener = vi.fn()
    subscribeSettings(settingsListener)
    subscribeSettingsSnapshot(snapshotListener)
    await refreshSettingsSnapshot()
    expect(settingsListener).toHaveBeenCalledWith(settingsB)
    expect(snapshotListener).toHaveBeenCalledWith(snapshot(settingsB, 2))
  })

  it('passes the caller-provided version to full save and import instead of borrowing cache state', async () => {
    getSettingsMock.mockResolvedValue(snapshot(settingsA, 3))
    await getSettingsSnapshotCached()
    saveSettingsMock.mockResolvedValue(snapshot(settingsB, 4))
    importSettingsMock.mockResolvedValue(snapshot(settingsA, 5))
    const editedFrom = version(2)
    await expect(saveSettingsCached(settingsB, editedFrom)).resolves.toBe(settingsB)
    expect(saveSettingsMock).toHaveBeenCalledWith(settingsB, editedFrom)
    await expect(importSettingsCached('/tmp/x.json', version(4))).resolves.toBe(settingsA)
    expect(importSettingsMock).toHaveBeenCalledWith('/tmp/x.json', version(4))
  })

  it('does not mutate or notify the cache when a full save fails', async () => {
    getSettingsMock.mockResolvedValue(snapshot(settingsA, 1))
    await getSettingsSnapshotCached()
    const snapshotListener = vi.fn()
    subscribeSettingsSnapshot(snapshotListener)
    saveSettingsMock.mockRejectedValue(new Error('disk full'))
    await expect(saveSettingsCached(settingsB, version(1))).rejects.toThrow('disk full')
    expect(peekSettings()).toBe(settingsA)
    expect(snapshotListener).not.toHaveBeenCalled()
  })

  it('retries a pure mutation once from a fresh snapshot after a version conflict', async () => {
    const conflict = {
      code: 'versionConflict', message: 'stale', expectedVersion: version(1), actualVersion: version(2),
    }
    getSettingsMock
      .mockResolvedValueOnce(snapshot(settingsA, 1))
      .mockResolvedValueOnce(snapshot(settingsB, 2))
    saveSettingsMock
      .mockRejectedValueOnce(conflict)
      .mockResolvedValueOnce(snapshot({ ...settingsB, theme: 'dark' } as Settings, 3))
    const mutate = vi.fn((current: Settings) => ({ ...current, theme: 'dark' }) as Settings)
    const saved = await updateSettingsCached(mutate)
    expect(saved.theme).toBe('dark')
    expect(mutate).toHaveBeenCalledTimes(2)
    expect(saveSettingsMock).toHaveBeenNthCalledWith(1, { ...settingsA, theme: 'dark' }, version(1))
    expect(saveSettingsMock).toHaveBeenNthCalledWith(2, { ...settingsB, theme: 'dark' }, version(2))
  })

  it('bounds conflict retries and preserves the latest authoritative cache', async () => {
    const conflict = { code: 'versionConflict', message: 'stale' }
    getSettingsMock
      .mockResolvedValueOnce(snapshot(settingsA, 1))
      .mockResolvedValueOnce(snapshot(settingsB, 2))
    saveSettingsMock.mockRejectedValue(conflict)
    await expect(updateSettingsCached((current) => ({ ...current, theme: 'dark' }), { maxConflictRetries: 1 }))
      .rejects.toBe(conflict)
    expect(saveSettingsMock).toHaveBeenCalledTimes(2)
    expect(peekSettings()).toBe(settingsB)
  })

  it('accepts canonical snapshots returned by lightweight writes', async () => {
    getSettingsMock.mockResolvedValue(snapshot(settingsA, 1))
    await getSettingsSnapshotCached()
    const favoriteCanonical = { ...settingsB, favoriteModels: ['p:m', 'p:n'] } as Settings
    setFavoriteModelsMock.mockResolvedValue(snapshot(favoriteCanonical, 2))
    setTranslateCardSizeMock.mockResolvedValue(snapshot(settingsB, 3))
    await setFavoriteModelsCached([' p:m ', '', 'p:m', 'p:n'])
    expect(setFavoriteModelsMock).toHaveBeenCalledWith([' p:m ', '', 'p:m', 'p:n'])
    expect(peekSettings()).toBe(favoriteCanonical)
    await setTranslateCardSizeCached(999)
    expect(peekSettings()).toBe(settingsB)
    expect(peekSettingsSnapshot()?.version).toEqual(version(3))
  })

  it('leaves the cache unchanged when a lightweight write fails', async () => {
    getSettingsMock.mockResolvedValue(snapshot(settingsA, 1))
    await getSettingsSnapshotCached()
    setFavoriteModelsMock.mockRejectedValue(new Error('disk full'))
    await expect(setFavoriteModelsCached(['p:m'])).rejects.toThrow('disk full')
    expect(peekSettingsSnapshot()).toEqual(snapshot(settingsA, 1))
  })

  it('refreshes only for an ahead settings event and ignores an older event', async () => {
    let emit: ((event: { version: SettingsVersion }) => void) | undefined
    const unlisten = vi.fn()
    onKivioSettingsChangedMock.mockImplementation(async (listener) => { emit = listener; return unlisten })
    getSettingsMock.mockResolvedValueOnce(snapshot(settingsA, 2))
    await getSettingsSnapshotCached()
    const stop = await startBackendSettingsSync()
    emit?.({ version: version(1) })
    await Promise.resolve()
    expect(getSettingsMock).toHaveBeenCalledTimes(1)
    getSettingsMock.mockResolvedValueOnce(snapshot(settingsB, 3))
    emit?.({ version: version(3) })
    await vi.waitFor(() => expect(peekSettings()).toBe(settingsB))
    stop()
    expect(unlisten).toHaveBeenCalledOnce()
  })

  it('keeps the previous snapshot when a refresh fails and retries on the next call', async () => {
    getSettingsMock.mockResolvedValueOnce(snapshot(settingsA, 1))
    await getSettingsSnapshotCached()
    getSettingsMock.mockRejectedValueOnce(new Error('ipc down'))
    await expect(refreshSettings()).rejects.toThrow('ipc down')
    expect(peekSettings()).toBe(settingsA)
    getSettingsMock.mockResolvedValueOnce(snapshot(settingsB, 2))
    await expect(refreshSettings()).resolves.toBe(settingsB)
  })

  it('clears a failed cold-read flight so the next cached read can retry', async () => {
    getSettingsMock.mockRejectedValueOnce(new Error('ipc down'))
    await expect(getSettingsSnapshotCached()).rejects.toThrow('ipc down')
    expect(peekSettingsSnapshot()).toBeNull()
    getSettingsMock.mockResolvedValueOnce(snapshot(settingsA, 1))
    await expect(getSettingsCached()).resolves.toBe(settingsA)
    expect(getSettingsMock).toHaveBeenCalledTimes(2)
  })
})
