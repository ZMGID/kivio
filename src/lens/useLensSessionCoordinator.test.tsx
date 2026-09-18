import { act, renderHook } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { useLensSessionCoordinator } from './useLensSessionCoordinator'

describe('useLensSessionCoordinator', () => {
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
})
