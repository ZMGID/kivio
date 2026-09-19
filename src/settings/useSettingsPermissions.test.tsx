import { act, renderHook } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { api, type PermissionStatus } from '../api/tauri'
import { useSettingsPermissions } from './useSettingsPermissions'

vi.mock('../api/tauri', () => ({
  api: { getPermissionStatus: vi.fn(), openPermissionSettings: vi.fn() },
}))

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((yes) => { resolve = yes })
  return { promise, resolve }
}

describe('settings permissions lifecycle', () => {
  beforeEach(() => vi.mocked(api.getPermissionStatus).mockReset())

  it('ignores an older refresh when a newer permission result has arrived', async () => {
    const older = deferred<PermissionStatus>()
    const newer = deferred<PermissionStatus>()
    vi.mocked(api.getPermissionStatus)
      .mockReturnValueOnce(older.promise)
      .mockReturnValueOnce(newer.promise)
    const { result } = renderHook(() => useSettingsPermissions())
    let refresh!: Promise<void>
    act(() => { refresh = result.current.refresh() })
    const current = { accessibility: true } as PermissionStatus
    const stale = { accessibility: false } as PermissionStatus

    await act(async () => { newer.resolve(current); await refresh })
    expect(result.current.status).toBe(current)
    expect(result.current.loading).toBe(false)
    await act(async () => { older.resolve(stale); await older.promise })
    expect(result.current.status).toBe(current)
  })
})
