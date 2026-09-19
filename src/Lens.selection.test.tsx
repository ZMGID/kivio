import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import Lens from './Lens'
import { StrictMode } from 'react'

const mocks = vi.hoisted(() => ({
  takeReset: vi.fn(),
  readFrame: vi.fn(),
  readImage: vi.fn(),
  readBase64: vi.fn(),
  capture: vi.fn(),
  compose: vi.fn(),
  copyImage: vi.fn(),
  close: vi.fn(),
}))

vi.mock('./lens/annotation', async importOriginal => ({
  ...await importOriginal<typeof import('./lens/annotation')>(),
  composeAnnotatedImage: mocks.compose,
}))
vi.mock('./chat/ChatMarkdown', () => ({ ChatMarkdown: () => null }))
vi.mock('./api/settingsCache', () => ({
  getSettingsCached: async () => ({ settingsLanguage: 'zh', lens: {}, screenshotTranslation: {} }),
  setTranslateCardSizeCached: vi.fn(),
}))
vi.mock('./api/tauri', () => ({
  api: new Proxy({}, {
    get: (_, key) => {
      if (key === 'lensTakeResetPayload') return mocks.takeReset
      if (key === 'lensReadFreezeFrame') return mocks.readFrame
      if (key === 'lensReadImage') return mocks.readImage
      if (key === 'explainReadImage') return mocks.readBase64
      if (key === 'lensCaptureRegion') return mocks.capture
      if (key === 'lensCopyImageToClipboard') return mocks.copyImage
      if (key === 'lensClose') return mocks.close
      if (key === 'lensListWindows') return async () => []
      if (key === 'takeLensSelection') return async () => ''
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

describe('Lens selection during cold initialization', () => {
  beforeEach(() => {
    window.location.hash = '#lens?mode=screenshot'
    mocks.takeReset.mockReset()
    mocks.readFrame.mockReset()
    mocks.readImage.mockReset().mockReturnValue(new Promise(() => {}))
    mocks.readBase64.mockReset()
    mocks.capture.mockReset()
    mocks.compose.mockReset().mockResolvedValue('png')
    mocks.copyImage.mockReset().mockResolvedValue({ success: true })
    mocks.close.mockReset().mockResolvedValue(undefined)
  })
  afterEach(() => { cleanup(); vi.useRealTimers(); vi.unstubAllGlobals(); vi.restoreAllMocks() })

  it.each(['feedback-reopen', 'copy-reopen', 'same-opening'])('keeps screenshot copy completion owned by its opening (%s)', async scenario => {
    vi.stubGlobal('URL', { createObjectURL: () => 'blob:cropped', revokeObjectURL: vi.fn() })
    const payload = JSON.stringify({ frame: { x: 0, y: 0, width: 1280, height: 800 } })
    mocks.takeReset.mockResolvedValue(payload)
    mocks.capture.mockResolvedValue({ success: true, imageId: 'cropped' })
    mocks.readImage.mockResolvedValue(new ArrayBuffer(4))
    let releaseCopy!: (value: { success: boolean }) => void
    if (scenario === 'copy-reopen') {
      mocks.copyImage.mockReturnValue(new Promise(resolve => { releaseCopy = resolve }))
    }
    const { container } = render(<Lens />)
    await act(async () => {})
    const root = container.firstElementChild!
    fireEvent.mouseDown(root, { clientX: 100, clientY: 100 })
    fireEvent.mouseMove(root, { clientX: 200, clientY: 180 })
    await act(async () => { fireEvent.mouseUp(root, { clientX: 200, clientY: 180 }) })
    vi.useFakeTimers()
    vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => window.setTimeout(() => callback(performance.now()), 16))
    await act(async () => { fireEvent.click(container.querySelector('button[title="复制"]')!) })
    expect(mocks.copyImage).toHaveBeenCalledOnce()
    if (scenario !== 'same-opening') {
      await act(async () => { window.dispatchEvent(new CustomEvent('lens:reset')) })
    }
    if (scenario === 'copy-reopen') await act(async () => { releaseCopy({ success: true }) })
    await act(async () => { await vi.advanceTimersByTimeAsync(449) })
    expect(mocks.close).not.toHaveBeenCalled()
    await act(async () => { await vi.advanceTimersByTimeAsync(1000) })
    if (scenario === 'same-opening') {
      expect(mocks.close).toHaveBeenCalledOnce()
      expect(container.firstElementChild?.getAttribute('aria-hidden')).toBe('true')
    } else {
      expect(mocks.close).not.toHaveBeenCalled()
      expect(container.firstElementChild?.getAttribute('aria-hidden')).not.toBe('true')
    }
  })

  const displayCases = [
    [1366, 768], [1920, 1080], [2560, 1440], [2560, 1600],
    [3840, 2160], [5120, 2880], [3440, 1440], [1080, 1920], [1537, 901],
  ].flatMap(([width, height]) => [1, 1.1, 1.25, 1.5, 1.75, 2, 2.25, 2.5, 3].map(scale => ({ width, height, scale })))

  it.each(displayCases)('displays $width x $height frozen pixels 1:1 at $scale device scale', async ({ width, height, scale }) => {
    vi.stubGlobal('devicePixelRatio', scale)
    vi.stubGlobal('URL', { createObjectURL: () => 'blob:freeze', revokeObjectURL: vi.fn() })
    vi.stubGlobal('Image', class {
      naturalWidth = width
      naturalHeight = height
      onload: (() => void) | null = null
      set src(_value: string) { queueMicrotask(() => this.onload?.()) }
    })
    const drawImage = vi.fn()
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({ clearRect: vi.fn(), drawImage } as unknown as CanvasRenderingContext2D)
    mocks.readFrame.mockResolvedValue(new ArrayBuffer(4))
    mocks.takeReset.mockResolvedValueOnce(JSON.stringify({ frame: { x: 0, y: 0, width: width / scale, height: height / scale }, freezeFrameImageId: 'freeze' }))
    const { container } = render(<Lens />)
    await waitFor(() => expect(drawImage).toHaveBeenCalled())
    const canvas = container.querySelector('canvas')!
    expect(canvas.width).toBe(width)
    expect(canvas.height).toBe(height)
    expect(parseFloat(canvas.style.width) * scale).toBeCloseTo(width)
    expect(parseFloat(canvas.style.height) * scale).toBeCloseTo(height)
  })

  it('updates frozen canvas scale when DPI changes without changing CSS viewport size', async () => {
    vi.stubGlobal('devicePixelRatio', 1.5)
    vi.stubGlobal('URL', { createObjectURL: () => 'blob:freeze', revokeObjectURL: vi.fn() })
    vi.stubGlobal('Image', class {
      naturalWidth = 2560
      naturalHeight = 1600
      onload: (() => void) | null = null
      set src(_value: string) { queueMicrotask(() => this.onload?.()) }
    })
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({ clearRect: vi.fn(), drawImage: vi.fn() } as unknown as CanvasRenderingContext2D)
    mocks.readFrame.mockResolvedValue(new ArrayBuffer(4))
    mocks.takeReset.mockResolvedValueOnce(JSON.stringify({ frame: { x: 0, y: 0, width: window.innerWidth, height: window.innerHeight }, freezeFrameImageId: 'freeze' }))
    const { container } = render(<Lens />)
    await waitFor(() => expect(parseFloat(container.querySelector('canvas')!.style.width)).toBeCloseTo(2560 / 1.5))
    vi.stubGlobal('devicePixelRatio', 2)
    await act(async () => { window.dispatchEvent(new Event('resize')) })
    await waitFor(() => expect(parseFloat(container.querySelector('canvas')!.style.width)).toBe(1280))
    expect(mocks.readFrame).toHaveBeenCalledTimes(1)
  })

  it('replays a take-once initialization in StrictMode', async () => {
    mocks.takeReset.mockResolvedValueOnce(JSON.stringify({ frame: { x: -1280, y: 0, width: 1280, height: 800 }, freezeFrameImageId: 'strict-frame' }))
      .mockResolvedValue(null)
    mocks.readFrame.mockReturnValue(new Promise(() => {}))
    mocks.capture.mockReturnValue(new Promise(() => {}))
    const { container } = render(<StrictMode><Lens /></StrictMode>)
    await waitFor(() => expect(mocks.readFrame).toHaveBeenCalledWith('strict-frame'))
    const root = container.firstElementChild!
    fireEvent.mouseDown(root, { clientX: 100, clientY: 100 })
    fireEvent.mouseMove(root, { clientX: 200, clientY: 180 })
    await act(async () => { fireEvent.mouseUp(root, { clientX: 200, clientY: 180 }) })
    expect(mocks.capture).toHaveBeenCalledWith(expect.objectContaining({ absoluteX: -1180, freezeFrameImageId: 'strict-frame' }))
  })

  it('releases the cropped Blob preview when closing', async () => {
    const revoke = vi.fn()
    vi.stubGlobal('URL', { createObjectURL: () => 'blob:cropped', revokeObjectURL: revoke })
    mocks.takeReset.mockResolvedValueOnce(JSON.stringify({ frame: { x: 0, y: 0, width: 1280, height: 800 } }))
    mocks.capture.mockResolvedValue({ success: true, imageId: 'cropped' })
    mocks.readImage.mockResolvedValue(new ArrayBuffer(4))
    const { container } = render(<Lens />)
    await act(async () => {})
    const root = container.firstElementChild!
    fireEvent.mouseDown(root, { clientX: 100, clientY: 100 })
    fireEvent.mouseMove(root, { clientX: 200, clientY: 180 })
    await act(async () => { fireEvent.mouseUp(root, { clientX: 200, clientY: 180 }) })
    expect(revoke).not.toHaveBeenCalled()
    await act(async () => { fireEvent.keyDown(window, { key: 'Escape' }) })
    await waitFor(() => expect(revoke).toHaveBeenCalledWith('blob:cropped'))
  })

  it('finishes a queued screenshot after initialization instead of discarding it', async () => {
    let deliver!: (value: string) => void
    mocks.takeReset.mockReturnValueOnce(new Promise<string>(resolve => { deliver = resolve }))
    mocks.readFrame.mockReturnValue(new Promise(() => {}))
    mocks.capture.mockResolvedValue({ success: true, imageId: 'cropped' })
    const { container } = render(<Lens />)
    const root = container.firstElementChild!
    fireEvent.mouseDown(root, { clientX: 100, clientY: 100 })
    fireEvent.mouseMove(root, { clientX: 200, clientY: 180 })
    await act(async () => { fireEvent.mouseUp(root, { clientX: 200, clientY: 180 }) })
    expect(container.querySelector<HTMLElement>('.border-\\[2px\\].rounded-sm')?.style.width).toBe('100px')
    await act(async () => {
      deliver(JSON.stringify({ frame: { x: 0, y: 0, width: 1280, height: 800 }, freezeFrameImageId: 'freeze' }))
    })
    await waitFor(() => expect(container.querySelector('button[title="复制"]')).not.toBeNull())
    expect(mocks.capture).toHaveBeenCalledTimes(1)
  })

  it('cancels an early selection on Escape even if initialization arrives later', async () => {
    let deliver!: (value: string) => void
    mocks.takeReset.mockReturnValueOnce(new Promise<string>(resolve => { deliver = resolve }))
    const { container } = render(<Lens />)
    const root = container.firstElementChild!
    fireEvent.mouseDown(root, { clientX: 100, clientY: 100 })
    fireEvent.mouseMove(root, { clientX: 200, clientY: 180 })
    await act(async () => { fireEvent.mouseUp(root, { clientX: 200, clientY: 180 }) })
    await act(async () => { fireEvent.keyDown(window, { key: 'Escape' }) })
    await act(async () => {
      deliver(JSON.stringify({ frame: { x: 0, y: 0, width: 1280, height: 800 }, freezeFrameImageId: 'freeze' }))
    })
    expect(mocks.capture).not.toHaveBeenCalled()
    expect(mocks.readFrame).not.toHaveBeenCalled()
    expect(container.querySelector('button[title="复制"]')).toBeNull()
  })

  it('ignores capture completion after Escape', async () => {
    let finish!: (value: { success: boolean; imageId: string }) => void
    mocks.takeReset.mockResolvedValueOnce(JSON.stringify({ frame: { x: 0, y: 0, width: 1280, height: 800 } }))
    mocks.capture.mockReturnValueOnce(new Promise(resolve => { finish = resolve }))
    const { container } = render(<Lens />)
    await act(async () => {})
    const root = container.firstElementChild!
    fireEvent.mouseDown(root, { clientX: 100, clientY: 100 })
    fireEvent.mouseMove(root, { clientX: 200, clientY: 180 })
    await act(async () => { fireEvent.mouseUp(root, { clientX: 200, clientY: 180 }) })
    expect(mocks.capture).toHaveBeenCalledTimes(1)
    // Another gesture during capture must not submit a second crop.
    fireEvent.mouseDown(root, { clientX: 300, clientY: 300 })
    fireEvent.mouseMove(root, { clientX: 400, clientY: 380 })
    await act(async () => { fireEvent.mouseUp(root, { clientX: 400, clientY: 380 }) })
    expect(mocks.capture).toHaveBeenCalledTimes(1)
    await act(async () => { fireEvent.keyDown(window, { key: 'Escape' }) })
    await act(async () => { finish({ success: true, imageId: 'stale' }) })
    expect(mocks.readImage).not.toHaveBeenCalled()
  })

  it.each(['screenshot', 'chat', 'translate', 'replace'])('queues a fast %s release until the frame and monitor origin arrive', async mode => {
    window.location.hash = `#lens?mode=${mode}`
    let deliver!: (value: string) => void
    mocks.takeReset.mockReturnValueOnce(new Promise<string>(resolve => { deliver = resolve }))
    mocks.readFrame.mockReturnValue(new Promise(() => {}))
    mocks.capture.mockReturnValue(new Promise(() => {}))
    const { container } = render(<Lens />)
    const root = container.firstElementChild!
    fireEvent.mouseDown(root, { clientX: 100, clientY: 100 })
    fireEvent.mouseMove(root, { clientX: 200, clientY: 180 })
    await act(async () => { fireEvent.mouseUp(root, { clientX: 200, clientY: 180 }) })
    expect(mocks.capture).not.toHaveBeenCalled()
    await act(async () => {
      deliver(JSON.stringify({ frame: { x: -1280, y: 50, width: 1280, height: 800 }, freezeFrameImageId: 'freeze' }))
    })
    await waitFor(() => expect(mocks.capture).toHaveBeenCalledTimes(1))
    expect(mocks.capture).toHaveBeenCalledWith(expect.objectContaining({
      x: 100, y: 100, width: 100, height: 80,
      absoluteX: -1180, absoluteY: 150, freezeFrameImageId: 'freeze',
    }))
  })

  it('keeps the frozen background after cropping consumes the backend frame', async () => {
    const revoke = vi.fn()
    vi.stubGlobal('URL', { createObjectURL: () => 'blob:freeze', revokeObjectURL: revoke })
    mocks.readFrame.mockResolvedValue(new ArrayBuffer(4))
    mocks.capture.mockResolvedValue({ success: true, imageId: 'cropped' })
    mocks.takeReset.mockResolvedValueOnce(JSON.stringify({ frame: { x: 0, y: 0, width: 1280, height: 800 }, freezeFrameImageId: 'freeze' }))
    const { container } = render(<Lens />)
    await waitFor(() => expect(container.querySelector('canvas')).not.toBeNull())
    const root = container.firstElementChild!
    fireEvent.mouseDown(root, { clientX: 100, clientY: 100 })
    fireEvent.mouseMove(root, { clientX: 200, clientY: 180 })
    await act(async () => { fireEvent.mouseUp(root, { clientX: 200, clientY: 180 }) })
    expect(mocks.capture).toHaveBeenCalledWith(expect.objectContaining({ freezeFrameImageId: 'freeze' }))
    expect(container.querySelector('button[title="复制"]')).not.toBeNull()
    expect(mocks.readImage).toHaveBeenCalledWith('cropped')
    expect(mocks.readBase64).not.toHaveBeenCalled()
    expect(container.querySelector('canvas')).not.toBeNull()
    expect(revoke).not.toHaveBeenCalled()
  })

  it('recovers the initial held-button drag if the new WebView missed mouse down', async () => {
    mocks.takeReset.mockReturnValueOnce(new Promise(() => {}))
    const { container } = render(<Lens />)
    const root = container.firstElementChild!
    fireEvent.mouseMove(root, { clientX: 100, clientY: 100, buttons: 0 })
    fireEvent.mouseMove(root, { clientX: 180, clientY: 160, buttons: 1 })
    expect(container.querySelector<HTMLElement>('.border-\\[2px\\].rounded-sm')?.style.width).toBe('80px')
  })

  it('does not turn a drag from the input bar into a screenshot selection', async () => {
    window.location.hash = '#lens?mode=chat'
    mocks.takeReset.mockReturnValueOnce(new Promise(() => {}))
    const { container } = render(<Lens />)
    const input = container.querySelector('input')!
    expect(input).not.toBeNull()
    fireEvent.mouseDown(input, { clientX: 100, clientY: 100 })
    fireEvent.mouseMove(container.firstElementChild!, { clientX: 180, clientY: 160, buttons: 1 })
    expect(container.querySelector('.border-\\[2px\\].rounded-sm')).toBeNull()
  })

  it.each(['screenshot', 'chat', 'translate', 'replace'])('preserves an early %s drag when the initial reset payload arrives', async mode => {
    window.location.hash = `#lens?mode=${mode}`
    let deliver!: (value: string) => void
    mocks.takeReset.mockReturnValueOnce(new Promise<string>(resolve => { deliver = resolve }))
    const { container } = render(<Lens />)
    const root = container.firstElementChild!
    const selection = () => container.querySelector<HTMLElement>('.border-\\[2px\\].rounded-sm')

    fireEvent.mouseDown(root, { clientX: 100, clientY: 100 })
    fireEvent.mouseMove(root, { clientX: 200, clientY: 180 })
    expect(selection()?.style.width).toBe('100px')

    await act(async () => {
      deliver(JSON.stringify({ frame: { x: 0, y: 0, width: 1280, height: 800 } }))
    })
    fireEvent.mouseMove(root, { clientX: 260, clientY: 220 })
    expect(selection()?.style.width).toBe('160px')
    expect(selection()?.style.left).toBe('100px')

    // A later session reset must still clear the previous selection.
    mocks.takeReset.mockResolvedValueOnce(JSON.stringify({ frame: { x: 0, y: 0, width: 1280, height: 800 } }))
    await act(async () => { window.dispatchEvent(new CustomEvent('lens:reset')) })
    expect(selection()).toBeNull()
  })
})
