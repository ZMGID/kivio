import { act, renderHook } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { useLensSessionCoordinator } from './useLensSessionCoordinator'

describe('useLensSessionCoordinator', () => {
  it('rejects an in-flight capture when history replaces the current content', async () => {
    const { result } = renderHook(() => useLensSessionCoordinator())
    act(() => result.current.beginOpening())
    const initialization = result.current.beginInitialization()
    act(() => result.current.markCaptureReady(initialization))
    const capture = result.current.beginCapture()!
    let visibleImage = 'old capture'

    await act(async () => result.current.restoreSession(() => { visibleImage = 'history image' }))
    if (result.current.isTokenCurrent(capture)) visibleImage = 'late capture'

    expect(visibleImage).toBe('history image')
    expect(result.current.finishCapture(capture)).toBe(false)
  })

  it('rejects late initialization, selection and capture completions after reset', () => {
    const { result } = renderHook(() => useLensSessionCoordinator())
    const initialization = result.current.beginInitialization()
    result.current.beginOpening()
    const selection = result.current.beginSelectionRead()
    let markedReady = false
    act(() => { markedReady = result.current.markCaptureReady(initialization) })
    expect(markedReady).toBe(true)
    expect(result.current.captureReady).toBe(true)
    const capture = result.current.beginCapture()
    expect(capture).not.toBeNull()

    act(() => result.current.resetForHide())

    expect(result.current.isInitializationCurrent(initialization)).toBe(false)
    expect(result.current.isSelectionCurrent(selection)).toBe(false)
    expect(result.current.finishCapture(capture!)).toBe(false)
    expect(result.current.canCapture()).toBe(false)
    expect(result.current.captureReady).toBe(false)
  })

  it('owns freeze-frame identity and consumes only the crop handle', () => {
    const { result } = renderHook(() => useLensSessionCoordinator())
    act(() => result.current.replaceFreezeFrame('frame-1'))
    expect(result.current.freezeFrameImageId).toBe('frame-1')
    expect(result.current.freezeFramePreviewId).toBe('frame-1')

    act(() => result.current.consumeFreezeFrame())
    expect(result.current.freezeFrameImageId).toBe('')
    expect(result.current.freezeFramePreviewId).toBe('frame-1')

    act(() => result.current.resetForHide())
    expect(result.current.freezeFramePreviewId).toBe('')
  })

  it('rejects a late request completion after cancellation and accepts the next request', async () => {
    const cancelRequest = vi.fn().mockResolvedValue(undefined)
    const { result } = renderHook(() => useLensSessionCoordinator({ cancelRequest }))
    act(() => result.current.beginOpening())
    const stale = result.current.beginRequest('chat', 'image-1')

    await act(() => result.current.cancelActiveRequest())

    expect(result.current.isRequestCurrent(stale)).toBe(false)
    expect(result.current.acceptsRequestEvent('chat', 'image-1')).toBe(false)
    const current = result.current.beginRequest('chat', 'image-1')
    expect(result.current.isRequestCurrent(current)).toBe(true)
    expect(result.current.acceptsRequestEvent('chat', 'image-1')).toBe(true)
    expect(cancelRequest).toHaveBeenCalledOnce()
  })

  it('reopening invalidates and releases the previous request before accepting new work', () => {
    const cancelRequest = vi.fn().mockResolvedValue(undefined)
    const { result } = renderHook(() => useLensSessionCoordinator({ cancelRequest }))
    act(() => result.current.beginOpening())
    const stale = result.current.beginRequest('translate', 'image-1')

    act(() => result.current.beginOpening())

    expect(result.current.isRequestCurrent(stale)).toBe(false)
    expect(result.current.acceptsRequestEvent('translate', 'image-1')).toBe(false)
    expect(cancelRequest).toHaveBeenCalledOnce()
  })

  it('keeps an event-finished request latest until it is superseded', () => {
    const { result } = renderHook(() => useLensSessionCoordinator())
    act(() => result.current.beginOpening())
    const finished = result.current.beginRequest('chat', 'image-1')

    expect(result.current.finishRequestEvent('chat', 'image-1')).toBe(true)
    expect(result.current.isRequestCurrent(finished)).toBe(false)
    expect(result.current.isRequestLatest(finished)).toBe(true)

    result.current.beginRequest('chat', 'image-1')
    expect(result.current.isRequestLatest(finished)).toBe(false)
  })

  it('invalidates an event-finished result when the caller changes context', async () => {
    const cancelRequest = vi.fn().mockResolvedValue(undefined)
    const { result } = renderHook(() => useLensSessionCoordinator({ cancelRequest }))
    act(() => result.current.beginOpening())
    const finished = result.current.beginRequest('chat', 'image-1')
    expect(result.current.finishRequestEvent('chat', 'image-1')).toBe(true)

    await act(() => result.current.cancelActiveRequest())

    expect(result.current.isRequestLatest(finished)).toBe(false)
    expect(cancelRequest).not.toHaveBeenCalled()
  })

  it('cancels an active request and invalidates all tokens on unmount', () => {
    const cancelRequest = vi.fn().mockResolvedValue(undefined)
    const { result, unmount } = renderHook(() => useLensSessionCoordinator({ cancelRequest }))
    act(() => result.current.beginOpening())
    const request = result.current.beginRequest('replace', 'image-1')

    unmount()

    expect(result.current.isRequestCurrent(request)).toBe(false)
    expect(cancelRequest).toHaveBeenCalledOnce()
  })

  it('waits for a hidden surface before closing, but never hides a reopened session', async () => {
    let releasePaint!: () => void
    const paint = new Promise<void>((resolve) => { releasePaint = resolve })
    const cancelRequest = vi.fn().mockResolvedValue(undefined)
    const prepareHiddenSurface = vi.fn()
    const hide = vi.fn().mockResolvedValue(undefined)
    const { result } = renderHook(() => useLensSessionCoordinator({ cancelRequest }))
    act(() => result.current.beginOpening())
    const old = result.current.beginRequest('chat', 'old')

    const closing = result.current.closeOpening({ prepareHiddenSurface, waitForPaint: () => paint, hide })
    await act(async () => { await Promise.resolve() })
    expect(cancelRequest).toHaveBeenCalledOnce()
    expect(prepareHiddenSurface).toHaveBeenCalledOnce()
    expect(hide).not.toHaveBeenCalled()

    act(() => result.current.beginOpening())
    expect(result.current.isRequestLatest(old)).toBe(false)
    releasePaint()
    await act(async () => { expect(await closing).toBe(false) })
    expect(hide).not.toHaveBeenCalled()
  })

  it('invalidates capture and selection immediately while close awaits backend cancellation', async () => {
    let releaseCancel!: () => void
    const cancelRequest = vi.fn(() => new Promise<void>(resolve => { releaseCancel = resolve }))
    const { result } = renderHook(() => useLensSessionCoordinator({ cancelRequest }))
    act(() => result.current.beginOpening())
    const initialization = result.current.beginInitialization()
    act(() => result.current.markCaptureReady(initialization))
    const capture = result.current.beginCapture()!
    const selection = result.current.beginSelectionRead()
    result.current.beginRequest('chat', 'old')
    const hide = vi.fn().mockResolvedValue(undefined)

    let closing!: Promise<boolean>
    act(() => { closing = result.current.closeOpening({
      prepareHiddenSurface: () => result.current.resetForHide(),
      waitForPaint: async () => undefined,
      hide,
    }) })
    expect(result.current.isTokenCurrent(capture)).toBe(false)
    expect(result.current.isSelectionCurrent(selection)).toBe(false)
    expect(hide).not.toHaveBeenCalled()
    releaseCancel()
    await act(async () => { expect(await closing).toBe(true) })
    expect(cancelRequest).toHaveBeenCalledOnce()
    expect(hide).toHaveBeenCalledOnce()
  })

  it('closes once after a terminal event without re-cancelling a completed request', async () => {
    const cancelRequest = vi.fn().mockResolvedValue(undefined)
    const steps: string[] = []
    const { result } = renderHook(() => useLensSessionCoordinator({ cancelRequest }))
    act(() => result.current.beginOpening())
    const completed = result.current.beginRequest('chat', 'image-1')
    expect(result.current.finishRequestEvent('chat', 'image-1')).toBe(true)
    expect(result.current.isRequestLatest(completed)).toBe(true)

    const closed = await act(async () => result.current.closeOpening({
      prepareHiddenSurface: () => { steps.push('reset'); result.current.resetForHide() },
      waitForPaint: async () => { steps.push('paint') },
      hide: async () => { steps.push('hide') },
    }))
    expect(closed).toBe(true)
    expect(steps).toEqual(['reset', 'paint', 'hide'])
    expect(cancelRequest).not.toHaveBeenCalled()
    expect(result.current.isRequestLatest(completed)).toBe(false)
  })

  it('coalesces repeated close intents for the same opening', async () => {
    const hide = vi.fn().mockResolvedValue(undefined)
    const prepareHiddenSurface = vi.fn()
    const { result } = renderHook(() => useLensSessionCoordinator())
    act(() => result.current.beginOpening())
    const operations = { prepareHiddenSurface, waitForPaint: async () => undefined, hide }

    const first = result.current.closeOpening(operations)
    const second = result.current.closeOpening(operations)
    expect(second).toBe(first)
    expect(await first).toBe(true)
    expect(await result.current.closeOpening(operations)).toBe(false)
    expect(prepareHiddenSurface).toHaveBeenCalledOnce()
    expect(hide).toHaveBeenCalledOnce()
  })

  it('restores history after invalidating a done event and a pending selection read', async () => {
    const cancelRequest = vi.fn().mockResolvedValue(undefined)
    const apply = vi.fn()
    const { result } = renderHook(() => useLensSessionCoordinator({ cancelRequest }))
    act(() => result.current.beginOpening())
    const selection = result.current.beginSelectionRead()
    const request = result.current.beginRequest('chat', 'same-image')
    expect(result.current.finishRequestEvent('chat', 'same-image')).toBe(true)

    await act(async () => result.current.restoreSession(apply))

    expect(apply).toHaveBeenCalledOnce()
    expect(result.current.isSelectionCurrent(selection)).toBe(false)
    expect(result.current.isRequestLatest(request)).toBe(false)
    expect(result.current.acceptsRequestEvent('chat', 'same-image')).toBe(false)
    expect(cancelRequest).not.toHaveBeenCalled()
  })
})
