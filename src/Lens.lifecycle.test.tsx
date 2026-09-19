import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import Lens from './Lens'
import type { LensStreamPayload } from './api/tauri'

const mocks = vi.hoisted(() => ({
  capture: vi.fn(), readImage: vi.fn(), close: vi.fn(), takeSelection: vi.fn(),
  dialog: vi.fn(), saveImage: vi.fn(), ask: vi.fn(), commitImage: vi.fn(),
  streamListener: vi.fn(),
}))

vi.mock('./chat/ChatMarkdown', () => ({ ChatMarkdown: ({ content }: { content: string }) => <div>{content}</div> }))
vi.mock('./lens/annotation', async importOriginal => ({
  ...await importOriginal<typeof import('./lens/annotation')>(),
  composeAnnotatedImage: async () => 'png',
}))
vi.mock('./lens/history', async importOriginal => ({
  ...await importOriginal<typeof import('./lens/history')>(),
  makeThumbnail: async () => 'thumbnail',
}))
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: mocks.dialog }))
vi.mock('./api/settingsCache', () => ({
  getSettingsCached: async () => ({ settingsLanguage: 'zh', lens: { sendToChat: false }, screenshotTranslation: {} }),
  setTranslateCardSizeCached: vi.fn(),
}))
vi.mock('./api/tauri', () => ({
  api: new Proxy({}, {
    get: (_, key) => {
      if (key === 'lensTakeResetPayload') return async () => JSON.stringify({ frame: { x: 0, y: 0, width: 1280, height: 800 } })
      if (key === 'lensListWindows') return async () => []
      if (key === 'lensCaptureRegion') return mocks.capture
      if (key === 'lensReadImage') return mocks.readImage
      if (key === 'lensClose') return mocks.close
      if (key === 'takeLensSelection') return mocks.takeSelection
      if (key === 'lensSaveAnnotatedPng') return mocks.saveImage
      if (key === 'lensAsk') return mocks.ask
      if (key === 'lensCommitImageToHistory') return mocks.commitImage
      if (key === 'onLensStream') return mocks.streamListener
      return async () => () => {}
    },
  }),
}))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    onFocusChanged: async () => () => {},
    outerPosition: async () => ({ x: 0, y: 0 }),
    scaleFactor: async () => 1,
  }),
}))

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: Error) => void
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej })
  return { promise, resolve, reject }
}

async function capture(container: HTMLElement) {
  const root = container.firstElementChild!
  fireEvent.mouseDown(root, { clientX: 100, clientY: 100 })
  fireEvent.mouseMove(root, { clientX: 200, clientY: 180 })
  await act(async () => { fireEvent.mouseUp(root, { clientX: 200, clientY: 180 }) })
}

describe('Lens content lifecycle', () => {
  beforeEach(() => {
    window.location.hash = '#lens?mode=screenshot'
    localStorage.clear()
    vi.stubGlobal('URL', { createObjectURL: () => 'blob:cropped', revokeObjectURL: vi.fn() })
    mocks.capture.mockReset().mockResolvedValue({ success: true, imageId: 'cropped' })
    mocks.readImage.mockReset().mockResolvedValue(new ArrayBuffer(4))
    mocks.close.mockReset().mockResolvedValue(undefined)
    mocks.takeSelection.mockReset().mockResolvedValue('')
    mocks.dialog.mockReset().mockResolvedValue('screenshot.png')
    mocks.saveImage.mockReset().mockResolvedValue({ success: true })
    mocks.ask.mockReset()
    mocks.commitImage.mockReset().mockResolvedValue(undefined)
    mocks.streamListener.mockReset().mockResolvedValue(() => {})
  })
  afterEach(() => { cleanup(); vi.unstubAllGlobals(); vi.restoreAllMocks() })

  it.each(['dialog', 'write'])('does not let an old annotation save %s affect a reopened Lens', async pendingStage => {
    const dialog = deferred<string>()
    const write = deferred<{ success: boolean }>()
    if (pendingStage === 'dialog') mocks.dialog.mockReturnValue(dialog.promise)
    else mocks.saveImage.mockReturnValue(write.promise)
    const { container } = render(<Lens />)
    await act(async () => {})
    await capture(container)
    await act(async () => { fireEvent.click(container.querySelector('button[title="保存"]')!) })
    await waitFor(() => expect(mocks.dialog).toHaveBeenCalledOnce())
    if (pendingStage === 'write') await waitFor(() => expect(mocks.saveImage).toHaveBeenCalledOnce())

    await act(async () => { window.dispatchEvent(new CustomEvent('lens:reset')) })
    await act(async () => {
      if (pendingStage === 'dialog') dialog.resolve('screenshot.png')
      else write.resolve({ success: true })
    })

    if (pendingStage === 'dialog') expect(mocks.saveImage).not.toHaveBeenCalled()
    expect(container.firstElementChild?.getAttribute('aria-hidden')).not.toBe('true')
    expect(mocks.close).not.toHaveBeenCalled()
  })

  it('saves the current annotation successfully before closing', async () => {
    const { container } = render(<Lens />)
    await act(async () => {})
    await capture(container)
    await act(async () => { fireEvent.click(container.querySelector('button[title="保存"]')!) })
    await waitFor(() => expect(mocks.saveImage).toHaveBeenCalledWith('png', 'screenshot.png'))
    await waitFor(() => expect(mocks.close).toHaveBeenCalledOnce())
    expect(container.firstElementChild?.getAttribute('aria-hidden')).toBe('true')
  })

  it.each(['old-reject', 'old-empty', 'current-reject'])('keeps text-selection completion scoped to its opening (%s)', async scenario => {
    const selection = deferred<string>()
    mocks.takeSelection.mockReturnValue(selection.promise)
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    window.location.hash = '#lens?mode=translateText'
    const { container } = render(<Lens />)
    await waitFor(() => expect(mocks.takeSelection).toHaveBeenCalledOnce())

    if (scenario !== 'current-reject') {
      window.location.hash = '#lens?mode=screenshot'
      await act(async () => { window.dispatchEvent(new CustomEvent('lens:reset')) })
    }
    await act(async () => {
      if (scenario === 'old-empty') selection.resolve('')
      else selection.reject(new Error('selection failed'))
    })

    if (scenario === 'current-reject') expect(mocks.close).toHaveBeenCalledOnce()
    else {
      expect(mocks.close).not.toHaveBeenCalled()
      expect(container.firstElementChild?.getAttribute('aria-hidden')).not.toBe('true')
    }
  })

  it.each(['success', 'error', 'reopen'])('keeps completed history when %s arrives during image persistence', async scenario => {
    window.location.hash = '#lens?mode=chat'
    const answer = deferred<{ success: boolean; response?: string; error?: string }>()
    const imageCommit = deferred<void>()
    mocks.ask.mockReturnValue(answer.promise)
    mocks.commitImage.mockReturnValue(imageCommit.promise)
    let stream!: (payload: LensStreamPayload) => void
    mocks.streamListener.mockImplementation(async listener => { stream = listener; return () => {} })
    const { container } = render(<Lens />)
    await act(async () => {})
    await capture(container)
    fireEvent.change(container.querySelector('input')!, { target: { value: 'question' } })
    await act(async () => { fireEvent.keyDown(container.querySelector('input')!, { key: 'Enter' }) })
    expect(mocks.ask).toHaveBeenCalledOnce()
    await act(async () => { stream({ imageId: 'cropped', kind: 'answer', delta: 'streamed answer' }) })
    await act(async () => { stream({ imageId: 'cropped', kind: 'answer', delta: '', done: true }) })
    expect(mocks.commitImage).toHaveBeenCalledWith('cropped')
    if (scenario === 'reopen') await act(async () => { window.dispatchEvent(new CustomEvent('lens:reset')) })
    await act(async () => {
      answer.resolve(scenario === 'error'
        ? { success: false, error: 'request failed' }
        : { success: true, response: 'streamed answer' })
    })
    await act(async () => { imageCommit.resolve() })

    const history = JSON.parse(localStorage.getItem('kivio:lens-history:v1') || '[]')
    expect(history).toHaveLength(1)
    expect(history[0].messages).toEqual([
      { role: 'user', content: 'question' },
      { role: 'assistant', content: scenario === 'error' ? '出错了: request failed' : 'streamed answer' },
    ])
  })
})
