import { act, renderHook } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import type { UpdateInfo } from '../api/tauri'
import { useSettingsUpdateController, type SettingsUpdatePort } from './useSettingsUpdateController'

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail })
  return { promise, resolve, reject }
}

function port(overrides: Partial<SettingsUpdatePort> = {}): SettingsUpdatePort {
  return {
    checkUpdate: vi.fn(async () => ({ available: false })),
    onUpdateAvailable: vi.fn(async () => () => {}),
    onUpdateDownloadProgress: vi.fn(async () => () => {}),
    downloadUpdate: vi.fn(async () => '/tmp/update'),
    installUpdate: vi.fn(async () => {}),
    openExternal: vi.fn(async () => {}),
    ...overrides,
  }
}

describe('useSettingsUpdateController', () => {
  it('ignores a stale check after the newer result becomes available', async () => {
    const old = deferred<UpdateInfo>()
    const newer = deferred<UpdateInfo>()
    const updatePort = port({ checkUpdate: vi.fn().mockReturnValueOnce(old.promise).mockReturnValueOnce(newer.promise) })
    const { result } = renderHook(() => useSettingsUpdateController(false, updatePort))
    let first!: Promise<void>
    let second!: Promise<void>
    act(() => { first = result.current.check(); second = result.current.check() })
    await act(async () => { newer.resolve({ available: true, version: '2.0' }); await second })
    await act(async () => { old.resolve({ available: false }); await first })
    expect(result.current.status).toBe('available')
    expect(result.current.info?.version).toBe('2.0')
  })

  it('runs one download/installation and keeps failure visible', async () => {
    const pending = deferred<string>()
    const updatePort = port({ downloadUpdate: vi.fn(() => pending.promise) })
    const { result } = renderHook(() => useSettingsUpdateController(false, updatePort))
    await act(async () => { await result.current.check() })
    // A check with an available result supplies the selected version.
    vi.mocked(updatePort.checkUpdate).mockResolvedValueOnce({ available: true, version: '2.0' })
    await act(async () => { await result.current.check() })
    let first!: Promise<void>
    act(() => { first = result.current.downloadAndInstall(); void result.current.downloadAndInstall() })
    await act(async () => { await Promise.resolve(); await Promise.resolve() })
    expect(updatePort.downloadUpdate).toHaveBeenCalledOnce()
    await act(async () => { pending.reject(new Error('download failed')); await first })
    expect(updatePort.installUpdate).not.toHaveBeenCalled()
    expect(result.current.downloadState).toBe('failed')
    expect(result.current.downloadError).toContain('download failed')
  })

  it('does not let a dismissed in-flight check restore an old result', async () => {
    const pending = deferred<UpdateInfo>()
    const updatePort = port({ checkUpdate: vi.fn(() => pending.promise) })
    const { result } = renderHook(() => useSettingsUpdateController(false, updatePort))
    let flight!: Promise<void>
    act(() => { flight = result.current.check(); result.current.dismiss() })
    await act(async () => { pending.resolve({ available: true, version: 'old' }); await flight })
    expect(result.current.status).toBe('idle')
  })

  it('releases an update listener even if subscription finishes after unmount', async () => {
    const subscription = deferred<() => void>()
    const unlisten = vi.fn()
    const updatePort = port({ onUpdateAvailable: vi.fn(() => subscription.promise) })
    const { unmount } = renderHook(() => useSettingsUpdateController(false, updatePort))
    unmount()
    await act(async () => { subscription.resolve(unlisten); await subscription.promise })
    expect(unlisten).toHaveBeenCalledOnce()
  })
})
