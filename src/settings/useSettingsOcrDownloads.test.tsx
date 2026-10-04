import { act, renderHook } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { OfflineModelProgress, RapidOcrStatus, ReplaceTranslationPackStatus } from '../api/tauri'
import { resetSettingsOcrDownloadsForTests, useSettingsOcrDownloads, type SettingsOcrPort } from './useSettingsOcrDownloads'

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
  beforeEach(() => {
    resetSettingsOcrDownloadsForTests()
  })

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
    await act(async () => {
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

  it('does not let a retired subscription fail a new download or open a duplicate listener', async () => {
    const oldSubscription = deferred<() => void>()
    const newSubscription = deferred<() => void>()
    const install = deferred<{ success: boolean; message: string }>()
    const listeners: Array<(progress: OfflineModelProgress) => void> = []
    const oldDispose = vi.fn()
    const newDispose = vi.fn()
    const ocrPort = port({
      replaceTranslationPackInstall: vi.fn(() => install.promise),
      onReplaceTranslationPackProgress: vi.fn((listener) => {
        listeners.push(listener)
        return listeners.length === 1 ? oldSubscription.promise : newSubscription.promise
      }),
    })
    const previous = renderHook(() => useSettingsOcrDownloads(true, 'high', ocrPort))
    previous.unmount()
    const returned = renderHook(() => useSettingsOcrDownloads(true, 'high', ocrPort))
    await act(async () => { oldSubscription.resolve(oldDispose); await oldSubscription.promise })
    let flight!: Promise<void>
    await act(async () => { flight = returned.result.current.downloadReplace('high') })
    act(() => {
      listeners[0]({
        pack: 'replace_translation', componentId: 'rapidocr-high', fileName: 'high/det.onnx',
        downloadedBytes: 25, fileTotalBytes: 100, overallDownloadedBytes: 25,
        overallTotalBytes: 200, attempt: 1, state: 'failed', error: 'retired listener',
      })
    })
    expect(returned.result.current.replaceDownload.downloadState).toBe('downloading')
    expect(returned.result.current.replaceDownload.error).toBe('')
    expect(listeners).toHaveLength(2)
    await act(async () => {
      install.resolve({ success: false, message: 'current failure' })
      await flight
    })
    expect(returned.result.current.replaceDownload.error).toBe('current failure')
    returned.unmount()
    await act(async () => { newSubscription.resolve(newDispose); await newSubscription.promise })
    expect(oldDispose).toHaveBeenCalledOnce()
    expect(newDispose).toHaveBeenCalledOnce()
  })

  it('shares one rapid install across settings and knowledge subscribers, including after both leave', async () => {
    const pending = deferred<{ success: boolean; message: string }>()
    const ocrPort = port({
      rapidOcrInstall: vi.fn(() => pending.promise),
      rapidOcrStatus: vi.fn(async () => ({ standardAvailable: false, highAvailable: false })),
    })
    const settings = renderHook(() => useSettingsOcrDownloads(true, 'standard', ocrPort))
    const knowledge = renderHook(() => useSettingsOcrDownloads(true, 'high', ocrPort, { observeReplacePack: false }))
    let flight!: Promise<void>
    await act(async () => {
      flight = knowledge.result.current.downloadRapid('high')
      void settings.result.current.downloadRapid('standard')
    })
    expect(ocrPort.rapidOcrInstall).toHaveBeenCalledOnce()
    expect(ocrPort.rapidOcrInstall).toHaveBeenCalledWith('high')
    expect(settings.result.current.rapidDownloadState).toBe('downloading')
    knowledge.unmount()
    settings.unmount()

    const returned = renderHook(() => useSettingsOcrDownloads(true, 'high', ocrPort, { observeReplacePack: false }))
    expect(returned.result.current.rapidDownloadState).toBe('downloading')
    await act(async () => {
      pending.reject(new Error('offline'))
      await flight
    })
    expect(returned.result.current.rapidDownloadState).toBe('failed')
    expect(returned.result.current.rapidDownloadError).toContain('offline')

    vi.mocked(ocrPort.rapidOcrStatus).mockResolvedValue({ standardAvailable: true, highAvailable: true })
    vi.mocked(ocrPort.rapidOcrInstall).mockResolvedValue({ success: true, message: 'ok' })
    await act(async () => { await returned.result.current.downloadRapid('high') })
    expect(ocrPort.rapidOcrInstall).toHaveBeenCalledTimes(2)
    expect(returned.result.current.rapidDownloadState).toBe('idle')
    expect(returned.result.current.rapidStatus?.highAvailable).toBe(true)
  })

  it('holds the replace progress subscription across unmount until the install reaches a terminal state', async () => {
    const subscription = deferred<() => void>()
    const unlisten = vi.fn()
    const install = deferred<{ success: boolean; message: string }>()
    let emit: (progress: OfflineModelProgress) => void = () => {}
    const ocrPort = port({
      replaceTranslationPackInstall: vi.fn(() => install.promise),
      onReplaceTranslationPackProgress: vi.fn((listener) => {
        emit = listener
        return subscription.promise
      }),
    })
    const progress = {
      pack: 'replace_translation' as const,
      componentId: 'rapidocr-high',
      fileName: 'high/det.onnx',
      downloadedBytes: 25,
      fileTotalBytes: 100,
      overallDownloadedBytes: 25,
      overallTotalBytes: 200,
      attempt: 1,
      state: 'downloading' as const,
    }
    const seen = renderHook(() => useSettingsOcrDownloads(true, 'high', ocrPort))
    let flight!: Promise<void>
    await act(async () => { flight = seen.result.current.downloadReplace('high') })
    expect(seen.result.current.replaceDownload.downloadState).toBe('downloading')
    seen.unmount()

    act(() => {
      emit({ ...progress, pack: 'replace_translation', downloadedBytes: 25 })
      emit({ ...progress, pack: 'rapidocr', downloadedBytes: 99, overallDownloadedBytes: 99 })
    })
    const during = renderHook(() => useSettingsOcrDownloads(true, 'high', ocrPort))
    expect(during.result.current.replaceDownload.downloadState).toBe('downloading')
    expect(during.result.current.replaceDownload.progress?.downloadedBytes).toBe(25)
    during.unmount()

    await act(async () => { subscription.resolve(unlisten); await subscription.promise })
    expect(unlisten).not.toHaveBeenCalled()
    await act(async () => {
      install.resolve({ success: false, message: 'disk full' })
      await flight
    })
    expect(unlisten).toHaveBeenCalledOnce()

    const after = renderHook(() => useSettingsOcrDownloads(true, 'high', ocrPort))
    expect(after.result.current.replaceDownload.downloadState).toBe('failed')
    expect(after.result.current.replaceDownload.error).toContain('disk full')
  })
})
