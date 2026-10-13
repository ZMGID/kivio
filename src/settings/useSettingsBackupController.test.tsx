import { act, renderHook } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { useSettingsBackupController, type SettingsBackupPort } from './useSettingsBackupController'

const fullPort = {
  pendingRestore: async () => false, importFull: async () => {}, confirmFullRestore: async () => true, restart: async () => {},
}

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
    const port: SettingsBackupPort = { ...fullPort,
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
    expect(result.current.status).toEqual({ kind: 'ok', msg: 'Configuration imported. MCP and hooks are disabled; local CLI configuration is retained.' })
  })

  it('keeps import failure visible and allows a later retry', async () => {
    const imported = vi.fn()
      .mockRejectedValueOnce(new Error('version conflict'))
      .mockResolvedValueOnce(undefined)
    const port: SettingsBackupPort = { ...fullPort,
      pickImport: async () => 'backup.json', pickExport: async () => null,
      import: imported, export: async () => {},
    }
    const { result } = renderHook(() => useSettingsBackupController(port, 'en'))
    await act(async () => { await result.current.importBackup() })
    expect(result.current.status).toEqual({ kind: 'err', msg: 'Operation failed: version conflict' })
    await act(async () => { await result.current.importBackup() })
    expect(result.current.status).toEqual({ kind: 'ok', msg: 'Configuration imported. MCP and hooks are disabled; local CLI configuration is retained.' })
  })
  it('requires an explicit trusted restore action to preserve executable configuration', async () => {
    const imported = vi.fn(async () => {})
    const exported = vi.fn(async () => {})
    const port: SettingsBackupPort = { ...fullPort,
      pickImport: async () => 'backup.json', pickExport: async () => 'share.json',
      import: imported, export: exported,
    }
    const { result } = renderHook(() => useSettingsBackupController(port, 'en'))
    await act(async () => { await result.current.importBackup() })
    expect(imported).toHaveBeenLastCalledWith('backup.json')
    await act(async () => { await result.current.restoreBackup() })
    expect(imported).toHaveBeenLastCalledWith('backup.json', true)
    expect(result.current.status?.msg).toContain('execution permissions')
    await act(async () => { await result.current.exportBackup() })
    expect(exported).toHaveBeenLastCalledWith('share.json', 'configuration')
    expect(result.current.status?.msg).toContain('Configuration exported')
  })

  it('prepares full restore only after confirmation and offers restart only after validation', async () => {
    const pending = deferred<void>()
    const imported = vi.fn(() => pending.promise)
    const confirm = vi.fn().mockResolvedValueOnce(false).mockResolvedValueOnce(true)
    const port: SettingsBackupPort = { ...fullPort, pickImport: async () => 'full.kivio', pickExport: async () => null,
      export: async () => {}, import: vi.fn(), importFull: imported, confirmFullRestore: confirm }
    const { result } = renderHook(() => useSettingsBackupController(port, 'en'))
    await act(async () => { await result.current.importBackup() })
    expect(imported).not.toHaveBeenCalled()
    let operation!: Promise<void>
    await act(async () => { operation = result.current.importBackup(); await Promise.resolve() })
    expect(result.current.restartRequired).toBe(false)
    expect(result.current.busy).toBe(true)
    pending.resolve()
    await act(async () => { await operation })
    expect(result.current.restartRequired).toBe(true)
    expect(port.import).not.toHaveBeenCalled()
    expect(imported).toHaveBeenCalledWith('full.kivio', false)
  })

  it('recovers a pending restore after leaving and reopening settings', async () => {
    const port: SettingsBackupPort = { ...fullPort, pendingRestore: async () => true,
      pickImport: async () => null, pickExport: async () => null, import: async () => {}, export: async () => {} }
    const { result } = renderHook(() => useSettingsBackupController(port, 'zh'))
    await act(async () => { await Promise.resolve() })
    expect(result.current.restartRequired).toBe(true)
  })

})
