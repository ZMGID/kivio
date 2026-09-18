import { act, cleanup, renderHook, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { api } from '../api/tauri'
import { useFreezeFramePreview } from './useFreezeFramePreview'

vi.mock('../api/tauri', () => ({ api: { lensReadFreezeFrame: vi.fn() } }))

describe('freeze frame binary preview lifecycle', () => {
  const create = vi.fn<(blob: Blob) => string>(() => 'blob:freeze-frame')
  const revoke = vi.fn()

  beforeEach(() => {
    vi.clearAllMocks()
    vi.stubGlobal('URL', { createObjectURL: create, revokeObjectURL: revoke })
  })
  afterEach(() => { cleanup(); vi.unstubAllGlobals() })

  it.each([new Uint8Array([137, 80, 78, 71]).buffer, [137, 80, 78, 71]])('releases the binary preview on close', async bytes => {
    vi.mocked(api.lensReadFreezeFrame).mockResolvedValue(bytes)
    const { result, rerender } = renderHook(({ id }) => useFreezeFramePreview(id), { initialProps: { id: 'frame-a' } })
    await waitFor(() => expect(result.current).toBe('blob:freeze-frame'))
    const blob = create.mock.calls[0][0]
    expect(blob.type).toBe('image/png')
    expect(blob.size).toBe(4)
    rerender({ id: '' })
    expect(result.current).toBe('')
    expect(revoke).toHaveBeenCalledWith('blob:freeze-frame')
  })

  it('ignores a previous session read that completes after reopening', async () => {
    let finishOld!: (bytes: ArrayBuffer) => void
    vi.mocked(api.lensReadFreezeFrame)
      .mockReturnValueOnce(new Promise(resolve => { finishOld = resolve }))
      .mockResolvedValueOnce(new ArrayBuffer(8))
    const { result, rerender, unmount } = renderHook(({ id }) => useFreezeFramePreview(id), { initialProps: { id: 'old' } })
    rerender({ id: 'new' })
    await waitFor(() => expect(result.current).toBe('blob:freeze-frame'))
    await act(async () => { finishOld(new ArrayBuffer(4)) })
    expect(create).toHaveBeenCalledTimes(1)
    unmount()
    expect(revoke).toHaveBeenCalledTimes(1)
  })

  it('does not allocate a URL after unmount', async () => {
    let finish!: (bytes: ArrayBuffer) => void
    vi.mocked(api.lensReadFreezeFrame).mockReturnValueOnce(new Promise(resolve => { finish = resolve }))
    const { unmount } = renderHook(() => useFreezeFramePreview('closed'))
    unmount()
    await act(async () => { finish(new ArrayBuffer(4)) })
    expect(create).not.toHaveBeenCalled()
  })
})
