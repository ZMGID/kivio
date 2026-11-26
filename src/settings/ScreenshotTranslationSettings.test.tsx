import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import type { ComponentProps } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { ScreenshotTranslationSettings } from './ScreenshotTranslationSettings'
import { makeProvider, makeSettings } from './tabs/testFixtures'
import type { ReplaceTranslationPackStatus } from '../api/tauri'
import { i18n } from '../components/i18n'

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => {}) }))
vi.mock('../api/tauri', async (importOriginal) => ({
  ...await importOriginal<typeof import('../api/tauri')>(),
  api: { thinkingCapabilitiesForModel: vi.fn().mockResolvedValue({ levels: ['low', 'high'], offMode: 'supported' }) },
}))

describe('ScreenshotTranslationSettings', () => {
  it('hides the previous download error after the offline pack is verified', () => {
    const error = 'HTTP onnxruntime.dll: 404 Not Found'
    const missing: ReplaceTranslationPackStatus = {
      tier: 'standard',
      ready: false,
      readyBytes: 0,
      totalBytes: 100,
      missingBytes: 100,
      files: [],
    }
    const props = {
      settings: makeSettings({ providers: [makeProvider()] }),
      isMac: false,
      hasSystemOcr: true,
      defaultPrompts: null,
      rapidOcrStatus: null,
      rapidOcrDownloadState: 'idle',
      rapidOcrDownloadError: '',
      replacePackDownloadState: 'failed',
      replacePackDownloadError: error,
      replacePackProgress: null,
      t: i18n.zh,
      onUpdate: vi.fn(),
      onRefreshRapidOcrStatus: vi.fn(),
      onDownloadRapidOcr: vi.fn(),
      onRefreshReplacePack: vi.fn(),
      onDownloadReplacePack: vi.fn(),
    } satisfies Omit<ComponentProps<typeof ScreenshotTranslationSettings>, 'replacePackStatus'>
    const { rerender } = render(<ScreenshotTranslationSettings {...props} replacePackStatus={missing} />)
    expect(screen.getByText(error, { exact: false })).toBeTruthy()

    rerender(<ScreenshotTranslationSettings {...props} replacePackStatus={{
      ...missing,
      ready: true,
      readyBytes: 100,
      missingBytes: 0,
    }} />)
    expect(screen.getByText(i18n.zh.replaceTranslatePackReady)).toBeTruthy()
    expect(screen.queryByText(error, { exact: false })).toBeNull()
  })

  it('saves effort beside the selected model and clears it when changing models', async () => {
    const onUpdate = vi.fn()
    const settings = makeSettings({ providers: [makeProvider()] })
    render(<ScreenshotTranslationSettings
      settings={settings}
      isMac={false}
      hasSystemOcr={false}
      defaultPrompts={null}
      rapidOcrStatus={null}
      rapidOcrDownloadState="idle"
      rapidOcrDownloadError=""
      replacePackStatus={null}
      replacePackDownloadState="idle"
      replacePackDownloadError=""
      replacePackProgress={null}
      t={i18n.zh}
      onUpdate={onUpdate}
      onRefreshRapidOcrStatus={vi.fn()}
      onDownloadRapidOcr={vi.fn()}
      onRefreshReplacePack={vi.fn()}
      onDownloadReplacePack={vi.fn()}
    />)
    expect(screen.queryByText(i18n.zh.screenshotTranslationThinking)).toBeNull()
    expect(screen.queryByText(i18n.zh.screenshotTranslationStream)).toBeNull()
    await userEvent.click(await screen.findByRole('button', { name: i18n.zh.reasoningEffort }))
    await userEvent.click(screen.getByRole('option', { name: 'high' }))
    expect(onUpdate).toHaveBeenCalledWith({ thinkingLevel: 'high' })
    await userEvent.click(screen.getByRole('button', { name: /gpt-4o/ }))
    await userEvent.click(screen.getByRole('option', { name: /gpt-4o/ }))
    expect(onUpdate).toHaveBeenLastCalledWith({ providerId: 'p1', model: 'gpt-4o', thinkingLevel: '' })
  })
})
