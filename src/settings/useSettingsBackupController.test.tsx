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

describe('manual WebDAV backup lifecycle', () => {
  function remotePort(): SettingsBackupPort {
    return { ...fullPort, pickImport: async () => null, pickExport: async () => null,
      import: vi.fn(async () => {}), export: vi.fn(async () => {}),
      webdav: {
        load: async () => ({ url: 'https://dav.test', username: 'user', directory: 'Kivio', hasPassword: true }),
        save: async () => ({ url: 'https://dav.test', username: 'user', directory: 'Kivio', hasPassword: true }),
        test: async () => {}, list: async () => [], upload: vi.fn(async () => [] as string[]),
        download: vi.fn(async () => ({ id: 'download', path: '/private/backup.kivio' })),
        release: vi.fn(async () => {}), delete: vi.fn(async () => {}),
        confirmRestore: async () => true, confirmDelete: async () => true,
      },
    }
  }
  const backup = { name: 'kivio-full-test.kivio', kind: 'full' as const, size: 10, modified: '' }

  it('requires confirmation before downloading and cleans temporary data after rejected archive validation', async () => {
    const port = remotePort()
    port.webdav!.confirmRestore = async () => false
    const { result } = renderHook(() => useSettingsBackupController(port, 'en'))
    await act(async () => { await result.current.runWebDav({ type: 'restore', backup, trusted: false }) })
    expect(port.webdav!.download).not.toHaveBeenCalled()
    port.webdav!.confirmRestore = async () => true
    port.importFull = async () => { throw new Error('Invalid checksum') }
    await act(async () => { await result.current.runWebDav({ type: 'restore', backup, trusted: false }) })
    expect(port.webdav!.release).toHaveBeenCalledWith('download')
    expect(result.current.restartRequired).toBe(false)
    expect(result.current.status).toEqual({ kind: 'err', msg: 'Operation failed: Invalid checksum' })
    port.importFull = async () => {}
    await act(async () => { await result.current.runWebDav({ type: 'restore', backup, trusted: false }) })
    expect(result.current.restartRequired).toBe(true)
  })

  it.each(['configuration', 'full', 'invalid'] as const)('preserves the %s restore outcome when temporary cleanup fails', async (outcome) => {
    const port = remotePort()
    port.webdav!.release = vi.fn(async () => { throw new Error('Cleanup unavailable') })
    if (outcome === 'invalid') port.importFull = async () => { throw new Error('Invalid checksum') }
    const selected = { ...backup, kind: outcome === 'configuration' ? 'configuration' as const : 'full' as const }
    const { result } = renderHook(() => useSettingsBackupController(port, 'en'))
    let restored = false
    await act(async () => { restored = await result.current.runWebDav({ type: 'restore', backup: selected, trusted: false }) })
    expect(restored).toBe(outcome !== 'invalid')
    expect(result.current.restartRequired).toBe(outcome === 'full')
    expect(result.current.status?.kind).toBe(outcome === 'invalid' ? 'err' : 'ok')
    expect(result.current.status?.msg).toContain(outcome === 'invalid' ? 'Invalid checksum' : outcome === 'full' ? 'Restart to restore' : 'Configuration restored')
    expect(result.current.status?.msg).toContain('Temporary backup cleanup failed')
  })

  it.each(['local', 'remote', 'refresh-failure'] as const)('shows skipped paths after %s backup completion', async (mode) => {
    const paths = ['E:\\Work\\.claude\\skills\\find-skills', '/workspace/broken']
    const port = remotePort()
    port.pickExport = async () => 'backup.kivio'
    port.export = async () => paths
    port.webdav!.upload = async () => paths
    if (mode === 'refresh-failure') port.webdav!.list = async () => { throw new Error('Offline') }
    const { result } = renderHook(() => useSettingsBackupController(port, 'zh'))
    await act(async () => {
      if (mode === 'local') await result.current.exportFullBackup()
      else await result.current.runWebDav({ type: 'upload', kind: 'full' })
    })
    expect(result.current.status?.msg).toContain('已跳过 2 个问题链接')
    for (const path of paths) expect(result.current.status?.msg).toContain(path)
    expect(result.current.status?.kind).toBe(mode === 'refresh-failure' ? 'err' : 'ok')
  })

  it('blocks overlapping local export and duplicate upload until remote upload completes', async () => {
    const pending = deferred<string[]>()
    const port = remotePort()
    port.webdav!.upload = vi.fn(() => pending.promise)
    const { result } = renderHook(() => useSettingsBackupController(port, 'en'))
    let upload!: Promise<boolean>
    act(() => { upload = result.current.runWebDav({ type: 'upload', kind: 'full' }) })
    await act(async () => {
      await result.current.exportBackup()
      await result.current.runWebDav({ type: 'upload', kind: 'full' })
    })
    expect(port.export).not.toHaveBeenCalled()
    expect(port.webdav!.upload).toHaveBeenCalledOnce()
    expect(result.current.busy).toBe(true)
    pending.resolve([])
    await act(async () => { await upload })
    expect(result.current.status).toEqual({ kind: 'ok', msg: 'Backup uploaded.' })
    expect(result.current.busy).toBe(false)
  })

  it('keeps a successful upload distinct from a failed list refresh', async () => {
    const port = remotePort()
    port.webdav!.list = async () => { throw new Error('Offline') }
    const { result } = renderHook(() => useSettingsBackupController(port, 'en'))
    await act(async () => { await result.current.runWebDav({ type: 'upload', kind: 'configuration' }) })
    expect(result.current.status?.msg).toContain('Operation completed, but the list could not refresh')
    expect(result.current.webdavFiles).toBeNull()
    expect(port.webdav!.upload).toHaveBeenCalledOnce()
  })
})
