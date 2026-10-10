import { act, renderHook } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { useSettingsBackupController, type SettingsBackupPort } from './useSettingsBackupController'

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}

describe('useSettingsBackupController', () => {
  it('coalesces repeated imports and never reports success before the import commits', async () => {
    const pending = deferred<void>()
    const imported = vi.fn(() => pending.promise)
    const port: SettingsBackupPort = {
      pickImport: async () => 'backup.json', pickExport: async () => null,
      import: imported, export: async () => {},
    }
    const { result } = renderHook(() => useSettingsBackupController(port, 'en'))
    let first!: Promise<void>
    act(() => {
      first = result.current.importBackup()
      void result.current.importBackup()
    })
    expect(result.current.busy).toBe(true)
    expect(result.current.status).toBeNull()
    await act(async () => { await Promise.resolve() })
    expect(imported).toHaveBeenCalledOnce()
    pending.resolve()
    await act(async () => { await first })
    expect(result.current.status).toEqual({ kind: 'ok', msg: 'Imported. MCP and hooks are disabled, local CLI configuration is retained, and tool execution requires approval. Review settings before enabling them.' })
  })

  it('keeps import failure visible and allows a later retry', async () => {
    const imported = vi.fn()
      .mockRejectedValueOnce(new Error('version conflict'))
      .mockResolvedValueOnce(undefined)
    const port: SettingsBackupPort = {
      pickImport: async () => 'backup.json', pickExport: async () => null,
      import: imported, export: async () => {},
    }
    const { result } = renderHook(() => useSettingsBackupController(port, 'en'))
    await act(async () => { await result.current.importBackup() })
    expect(result.current.status).toEqual({ kind: 'err', msg: 'Import failed: version conflict' })
    await act(async () => { await result.current.importBackup() })
    expect(result.current.status).toEqual({ kind: 'ok', msg: 'Imported. MCP and hooks are disabled, local CLI configuration is retained, and tool execution requires approval. Review settings before enabling them.' })
  })
  it('requires an explicit trusted restore action to preserve executable configuration', async () => {
    const imported = vi.fn(async () => {})
    const exported = vi.fn(async () => {})
    const port: SettingsBackupPort = {
      pickImport: async () => 'backup.json', pickExport: async () => 'share.json',
      import: imported, export: exported,
    }
    const { result } = renderHook(() => useSettingsBackupController(port, 'en'))
    await act(async () => { await result.current.importBackup() })
    expect(imported).toHaveBeenLastCalledWith('backup.json')
    await act(async () => { await result.current.restoreBackup() })
    expect(imported).toHaveBeenLastCalledWith('backup.json', true)
    expect(result.current.status?.msg).toContain('execution permissions')
    await act(async () => { await result.current.shareBackup() })
    expect(exported).toHaveBeenLastCalledWith('share.json', true)
    expect(result.current.status?.msg).toContain('Review sets')
  })

})
