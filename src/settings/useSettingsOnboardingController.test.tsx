import { act, renderHook } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { useSettingsOnboardingController, type SettingsOnboardingPort } from './useSettingsOnboardingController'

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => { resolve = done })
  return { promise, resolve }
}

describe('useSettingsOnboardingController', () => {
  it('keeps the page and draft intact when the initial flush rejects or reports unsaved changes', async () => {
    const writePending = vi.fn()
    const committed = vi.fn()
    const navigate = vi.fn()
    const port: SettingsOnboardingPort = {
      flush: vi.fn().mockResolvedValue(false),
      writePending, committed, navigate,
    }
    const { result } = renderHook(() => useSettingsOnboardingController(port, 'en'))
    await act(async () => { await result.current.restart() })
    expect(writePending).not.toHaveBeenCalled()
    expect(committed).not.toHaveBeenCalled()
    expect(navigate).not.toHaveBeenCalled()
    expect(result.current.error).toMatch(/unsaved/i)

    port.flush = vi.fn().mockRejectedValue(new Error('disk full'))
    await act(async () => { await result.current.restart() })
    expect(writePending).not.toHaveBeenCalled()
    expect(navigate).not.toHaveBeenCalled()
    expect(result.current.error).toContain('disk full')
  })

  it('coalesces restarts, waits for narrow write and a second flush, then navigates once', async () => {
    const pending = deferred<void>()
    const finalFlush = deferred<boolean>()
    const flush = vi.fn().mockResolvedValueOnce(true).mockImplementationOnce(() => finalFlush.promise)
    const committed = vi.fn()
    const navigate = vi.fn()
    const port: SettingsOnboardingPort = {
      flush,
      writePending: vi.fn(() => pending.promise),
      committed, navigate,
    }
    const { result } = renderHook(() => useSettingsOnboardingController(port, 'en'))
    let first!: Promise<void>
    act(() => { first = result.current.restart(); void result.current.restart() })
    expect(result.current.busy).toBe(true)
    await act(async () => { await Promise.resolve() })
    expect(port.writePending).toHaveBeenCalledOnce()
    pending.resolve()
    await act(async () => { await Promise.resolve(); await Promise.resolve() })
    expect(flush).toHaveBeenCalledTimes(2)
    expect(navigate).not.toHaveBeenCalled()
    finalFlush.resolve(true)
    await act(async () => { await first })
    expect(committed).toHaveBeenCalledOnce()
    expect(navigate).toHaveBeenCalledOnce()
    expect(result.current.error).toBe('')
  })

  it('does not navigate when the final flush fails after the narrow write', async () => {
    const navigate = vi.fn()
    const committed = vi.fn()
    const port: SettingsOnboardingPort = {
      flush: vi.fn().mockResolvedValueOnce(true).mockResolvedValueOnce(false),
      writePending: vi.fn().mockResolvedValue(undefined), committed, navigate,
    }
    const { result } = renderHook(() => useSettingsOnboardingController(port, 'en'))
    await act(async () => { await result.current.restart() })
    expect(committed).toHaveBeenCalledOnce()
    expect(navigate).not.toHaveBeenCalled()
    expect(result.current.error).toMatch(/unsaved/i)
  })
})
