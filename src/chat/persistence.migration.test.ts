/** @vitest-environment jsdom */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke }))

const legacyKey = 'kivio-chat-last-route'
function deferred() {
  let resolve!: () => void
  let reject!: (reason: Error) => void
  const promise = new Promise<void>((res, rej) => { resolve = res; reject = rej })
  return { promise, resolve, reject }
}

beforeEach(() => {
  vi.resetModules()
  invoke.mockReset()
  vi.spyOn(console, 'warn').mockImplementation(() => {})
  window.localStorage.clear()
})
afterEach(() => vi.restoreAllMocks())

describe('legacy route migration at the IPC boundary', () => {
  it('does not delete a legacy value replaced while migration is pending', async () => {
    const migration = deferred()
    invoke.mockReturnValue(migration.promise)
    const persistence = await import('./persistence')
    window.localStorage.setItem(legacyKey, '#chat/legacy')
    persistence.getRememberedChatRoute()
    window.localStorage.setItem(legacyKey, '#chat/replaced')
    migration.resolve()
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(window.localStorage.getItem(legacyKey)).toBe('#chat/replaced')
  })

  it('retains legacy storage on a failed clear without resurrecting it, and allows retry', async () => {
    const first = deferred()
    invoke.mockReturnValueOnce(first.promise).mockResolvedValue(undefined)
    const persistence = await import('./persistence')
    window.localStorage.setItem(legacyKey, '#chat/legacy')
    const clear = persistence.forgetRememberedChatRoute()
    const failure = new Error('disk full')
    first.reject(failure)
    await expect(clear).rejects.toBe(failure)
    expect(window.localStorage.getItem(legacyKey)).toBe('#chat/legacy')
    expect(persistence.getRememberedChatRoute()).toBeNull()
    expect(console.warn).toHaveBeenCalledWith('[persistence] Failed to persist chat route:', failure)
    await persistence.forgetRememberedChatRoute()
    expect(window.localStorage.getItem(legacyKey)).toBeNull()
  })

  it.each(['remember', 'clear'] as const)('a late migration cannot overwrite a newer %s', async (action) => {
    const migration = deferred()
    let stored: string | null = null
    invoke.mockImplementation((_command: string, { route }: { route: string | null }) => {
      const completion = route === '#chat/legacy' ? migration.promise : Promise.resolve()
      return completion.then(() => { stored = route })
    })
    const persistence = await import('./persistence')
    window.localStorage.setItem(legacyKey, '#chat/legacy')
    persistence.getRememberedChatRoute()
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledTimes(1))

    window.location.hash = '#chat/newer'
    if (action === 'remember') persistence.rememberCurrentChatRoute()
    else persistence.forgetRememberedChatRoute()
    const expected = action === 'remember' ? '#chat/newer' : null
    expect(persistence.getRememberedChatRoute()).toBe(expected)
    expect(window.localStorage.getItem(legacyKey)).toBe('#chat/legacy')
    await new Promise((resolve) => setTimeout(resolve, 0))
    migration.resolve()
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(stored).toBe(expected)
    expect(persistence.getRememberedChatRoute()).toBe(expected)
    expect(window.localStorage.getItem(legacyKey)).toBeNull()
  })

  it.each(['remember', 'clear'] as const)('a failed migration cannot erase the newer %s intent', async (action) => {
    const migration = deferred()
    invoke.mockReturnValueOnce(migration.promise).mockResolvedValue(undefined)
    const persistence = await import('./persistence')
    window.localStorage.setItem(legacyKey, '#chat/legacy')
    persistence.getRememberedChatRoute()
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledTimes(1))
    window.location.hash = '#chat/newer'
    if (action === 'remember') persistence.rememberCurrentChatRoute()
    else persistence.forgetRememberedChatRoute()
    migration.reject(new Error('disk full'))
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(persistence.getRememberedChatRoute()).toBe(action === 'remember' ? '#chat/newer' : null)
  })

  it('retains a failed migration and retries it on the next read', async () => {
    const first = deferred()
    const retry = deferred()
    invoke.mockReturnValueOnce(first.promise).mockReturnValueOnce(retry.promise)
    const persistence = await import('./persistence')
    window.localStorage.setItem(legacyKey, '#chat/legacy')
    persistence.getRememberedChatRoute()
    first.reject(new Error('disk full'))
    await new Promise((resolve) => setTimeout(resolve, 0))

    expect(window.localStorage.getItem(legacyKey)).toBe('#chat/legacy')
    expect(persistence.getRememberedChatRoute()).toBe('#chat/legacy')
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledTimes(2))
    retry.resolve()
    await vi.waitFor(() => expect(window.localStorage.getItem(legacyKey)).toBeNull())
  })

  it('propagates the persistence rejection through the transport adapter', async () => {
    const write = deferred()
    invoke.mockReturnValue(write.promise)
    const { api } = await import('../api/tauri')
    const result = api.rememberChatLastRoute('#chat/legacy')
    const failure = new Error('disk full')
    write.reject(failure)
    await expect(result).rejects.toBe(failure)
  })

  it('keeps the recoverable legacy route until the new store confirms success', async () => {
    const write = deferred()
    invoke.mockReturnValue(write.promise)
    const persistence = await import('./persistence')
    window.localStorage.setItem(legacyKey, '#chat/legacy')

    expect(persistence.getRememberedChatRoute()).toBe('#chat/legacy')
    expect(window.localStorage.getItem(legacyKey)).toBe('#chat/legacy')
    write.resolve()
    await vi.waitFor(() => expect(window.localStorage.getItem(legacyKey)).toBeNull())
    expect(persistence.getRememberedChatRoute()).toBe('#chat/legacy')
  })
})
