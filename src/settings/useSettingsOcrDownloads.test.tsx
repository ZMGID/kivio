import { act, renderHook } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import type { RapidOcrStatus, ReplaceTranslationPackStatus } from '../api/tauri'
import { useSettingsOcrDownloads, type SettingsOcrPort } from './useSettingsOcrDownloads'

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail })
  return { promise, resolve, reject }
}

const rapid = (ready: boolean): RapidOcrStatus => ({ standardAvailable: ready, highAvailable: false })
const replace = (tier: 'standard' | 'high', ready: boolean): ReplaceTranslationPackStatus => ({
  tier, ready, totalBytes: 1, readyBytes: ready ? 1 : 0, missingBytes: ready ? 0 : 1, files: [],
})

function port(overrides: Partial<SettingsOcrPort> = {}): SettingsOcrPort {
  return {
    rapidOcrStatus: vi.fn(async () => rapid(false)),
    rapidOcrInstall: vi.fn(async () => ({ success: true, message: 'ok' })),
    replaceTranslationPackStatus: vi.fn(async (tier) => replace(tier, false)),
    replaceTranslationPackInstall: vi.fn(async () => ({ success: true, message: 'ok' })),
    onReplaceTranslationPackProgress: vi.fn(async () => () => {}),
    ...overrides,
  }
}

describe('useSettingsOcrDownloads', () => {
  it('keeps the newest rapid status when an older refresh finishes later', async () => {
    const old = deferred<RapidOcrStatus>()
    const newer = deferred<RapidOcrStatus>()
    const ocrPort = port({ rapidOcrStatus: vi.fn().mockReturnValueOnce(old.promise).mockReturnValueOnce(newer.promise) })
    const { result } = renderHook(() => useSettingsOcrDownloads(true, 'standard', ocrPort))
    let refresh!: Promise<void>
    act(() => { refresh = result.current.refreshRapid() })
    await act(async () => { newer.resolve(rapid(true)); await refresh })
    await act(async () => { old.resolve(rapid(false)); await old.promise })
    expect(result.current.rapidStatus?.standardAvailable).toBe(true)
  })

  it('ignores a previous tier status after the selected tier changes', async () => {
    const old = deferred<ReplaceTranslationPackStatus>()
    const newer = deferred<ReplaceTranslationPackStatus>()
    const ocrPort = port({ replaceTranslationPackStatus: vi.fn().mockReturnValueOnce(old.promise).mockReturnValueOnce(newer.promise) })
    const { result, rerender } = renderHook(({ tier }) => useSettingsOcrDownloads(true, tier, ocrPort), {
      initialProps: { tier: 'standard' as 'standard' | 'high' },
    })
    rerender({ tier: 'high' })
    await act(async () => { newer.resolve(replace('high', true)); await newer.promise })
    await act(async () => { old.resolve(replace('standard', false)); await old.promise })
    expect(result.current.replaceStatus).toMatchObject({ tier: 'high', ready: true })
  })

  it('does not let an older-tier download refresh replace the current tier', async () => {
    const install = deferred<{ success: boolean; message: string }>()
    const ocrPort = port({ replaceTranslationPackInstall: vi.fn(() => install.promise) })
    const { result, rerender } = renderHook(({ tier }) => useSettingsOcrDownloads(true, tier, ocrPort), {
      initialProps: { tier: 'standard' as 'standard' | 'high' },
    })
    let flight!: Promise<void>
    act(() => { flight = result.current.downloadReplace('standard') })
    rerender({ tier: 'high' })
    await act(async () => { await Promise.resolve() })
    expect(result.current.replaceStatus?.tier).toBe('high')
    await act(async () => { install.resolve({ success: true, message: 'ok' }); await flight })
    expect(result.current.replaceStatus?.tier).toBe('high')
  })

  it('runs each download once while pending and exposes a failed install', async () => {
    const pendingRapid = deferred<{ success: boolean; message: string }>()
    const pendingReplace = deferred<{ success: boolean; message: string }>()
    const ocrPort = port({
      rapidOcrInstall: vi.fn(() => pendingRapid.promise),
      replaceTranslationPackInstall: vi.fn(() => pendingReplace.promise),
    })
    const { result } = renderHook(() => useSettingsOcrDownloads(true, 'standard', ocrPort))
    let rapidFlight!: Promise<void>
    let replaceFlight!: Promise<void>
    act(() => {
      rapidFlight = result.current.downloadRapid('standard')
      void result.current.downloadRapid('standard')
      replaceFlight = result.current.downloadReplace('standard')
      void result.current.downloadReplace('standard')
    })
    expect(ocrPort.rapidOcrInstall).toHaveBeenCalledOnce()
    expect(ocrPort.replaceTranslationPackInstall).toHaveBeenCalledOnce()
    await act(async () => {
      pendingRapid.reject(new Error('rapid network failure'))
      pendingReplace.resolve({ success: false, message: 'replace checksum failure' })
      await Promise.all([rapidFlight, replaceFlight])
    })
    expect(result.current.rapidDownloadState).toBe('failed')
    expect(result.current.rapidDownloadError).toContain('rapid network failure')
    expect(result.current.replaceDownload.downloadState).toBe('failed')
    expect(result.current.replaceDownload.error).toContain('replace checksum failure')
  })

  it('releases a progress listener even if subscription finishes after unmount', async () => {
    const subscription = deferred<() => void>()
    const unlisten = vi.fn()
    const ocrPort = port({ onReplaceTranslationPackProgress: vi.fn(() => subscription.promise) })
    const { unmount } = renderHook(() => useSettingsOcrDownloads(true, 'standard', ocrPort))
    unmount()
    await act(async () => { subscription.resolve(unlisten); await subscription.promise })
    expect(unlisten).toHaveBeenCalledOnce()
  })
})
