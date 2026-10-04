import { act, render, renderHook, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { api, type DocumentProcessingConfig } from '../api/tauri'
import { DocumentProcessingPanel } from './DocumentProcessingPanel'
import { resetSettingsOcrDownloadsForTests, useSettingsOcrDownloads } from './useSettingsOcrDownloads'

vi.mock('../api/tauri', () => ({
  api: {
    rapidOcrStatus: vi.fn(),
    rapidOcrInstall: vi.fn(),
    replaceTranslationPackStatus: vi.fn(),
    replaceTranslationPackInstall: vi.fn(),
    onReplaceTranslationPackProgress: vi.fn(async () => () => {}),
  },
}))

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail })
  return { promise, resolve, reject }
}

function config(ocrEngine: DocumentProcessingConfig['ocrEngine']): DocumentProcessingConfig {
  return {
    ocrEngine,
    rapidOcrTier: 'high',
    pdfStrategy: 'text',
    activeProcessor: '',
    fallbackToThirdParty: false,
    providers: [],
  }
}

describe('DocumentProcessingPanel RapidOCR flight', () => {
  let highAvailable = false

  beforeEach(() => {
    highAvailable = false
    resetSettingsOcrDownloadsForTests()
    vi.mocked(api.rapidOcrStatus).mockReset()
    vi.mocked(api.rapidOcrInstall).mockReset()
    vi.mocked(api.replaceTranslationPackStatus).mockReset()
    vi.mocked(api.rapidOcrStatus).mockImplementation(async () => ({
      standardAvailable: false,
      highAvailable,
      modelDir: highAvailable ? '/models/rapid' : null,
    }))
    vi.mocked(api.replaceTranslationPackStatus).mockImplementation(async (tier) => ({
      tier,
      ready: false,
      totalBytes: 1,
      readyBytes: 0,
      missingBytes: 1,
      files: [],
    }))
  })

  it('keeps the settings OCR flight when the knowledge row unmounts, then retries to ready', async () => {
    const install = deferred<{ success: boolean; message: string }>()
    vi.mocked(api.rapidOcrInstall).mockImplementation(() => install.promise)
    const onChange = vi.fn()
    const panel = render(
      <DocumentProcessingPanel lang="zh" config={config('rapid_ocr')} onChange={onChange} />,
    )
    const button = await screen.findByRole('button', { name: '下载离线模型' })
    const settings = renderHook(() => useSettingsOcrDownloads(true, 'standard'))

    await act(async () => {
      button.click()
      button.click()
    })
    expect(api.rapidOcrInstall).toHaveBeenCalledTimes(1)
    expect(api.rapidOcrInstall).toHaveBeenCalledWith('high')
    expect(settings.result.current.rapidDownloadState).toBe('downloading')
    expect(screen.getByText('正在下载…')).toBeInTheDocument()

    panel.rerender(<DocumentProcessingPanel lang="zh" config={config('off')} onChange={onChange} />)
    expect(screen.queryByText('正在下载…')).not.toBeInTheDocument()
    expect(settings.result.current.rapidDownloadState).toBe('downloading')

    panel.rerender(<DocumentProcessingPanel lang="zh" config={config('rapid_ocr')} onChange={onChange} />)
    expect(screen.getByText('正在下载…')).toBeInTheDocument()

    panel.unmount()
    settings.unmount()
    const returned = render(
      <DocumentProcessingPanel lang="zh" config={config('rapid_ocr')} onChange={onChange} />,
    )
    expect(screen.getByText('正在下载…')).toBeInTheDocument()

    await act(async () => { install.reject(new Error('model host unreachable')) })
    expect(await screen.findByText(/model host unreachable/)).toBeInTheDocument()
    returned.rerender(<DocumentProcessingPanel lang="zh" config={config('system')} onChange={onChange} />)
    returned.rerender(<DocumentProcessingPanel lang="zh" config={config('rapid_ocr')} onChange={onChange} />)
    expect(screen.getByText(/model host unreachable/)).toBeInTheDocument()

    const retry = deferred<{ success: boolean; message: string }>()
    vi.mocked(api.rapidOcrInstall).mockImplementation(() => retry.promise)
    await act(async () => { screen.getByRole('button', { name: '下载离线模型' }).click() })
    expect(api.rapidOcrInstall).toHaveBeenCalledTimes(2)
    expect(screen.getByText('正在下载…')).toBeInTheDocument()
    highAvailable = true
    await act(async () => { retry.resolve({ success: true, message: 'ok' }) })
    expect(await screen.findByText('RapidOCR 已就绪')).toBeInTheDocument()
    expect(screen.getByText('/models/rapid')).toBeInTheDocument()
  })
})
