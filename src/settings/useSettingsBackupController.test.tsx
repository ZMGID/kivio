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
  it('confirms full chat replacement and permits retry after a failed restore', async () => {
    let accepted = false
    let attempts = 0
    const port: SettingsBackupPort = {
      pickImport: async () => null, pickExport: async () => null,
      import: async () => {}, export: async () => {},
      chatBackup: {
        pickImport: async () => 'chat.zip', pickExport: async () => 'chat.zip',
        confirmRestore: async () => accepted,
        export: async () => {},
        restore: async () => { if (++attempts === 1) throw new Error('invalid chat backup') },
      },
    }
    const { result } = renderHook(() => useSettingsBackupController(port, 'en'))
    await act(async () => { await result.current.restoreChatBackup() })
    expect(attempts).toBe(0)
    accepted = true
    await act(async () => { await result.current.restoreChatBackup() })
    expect(result.current.status).toEqual({ kind: 'err', msg: 'Operation failed: invalid chat backup' })
    expect(result.current.busy).toBe(false)
    await act(async () => { await result.current.restoreChatBackup() })
    expect(result.current.status?.msg).toContain('Restarting')
    await act(async () => { await result.current.exportChatBackup() })
    expect(result.current.status?.msg).toBe('Full chat backup exported.')
  })
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
    expect(result.current.status?.kind).toBe('ok')
  })

  it('shows structured settings errors as text and allows a later retry', async () => {
    const imported = vi.fn()
      .mockRejectedValueOnce({ code: 'operationFailed', message: '不支持的配置备份格式或版本' })
      .mockResolvedValueOnce(undefined)
    const port: SettingsBackupPort = {
      pickImport: async () => 'backup.json', pickExport: async () => null,
      import: imported, export: async () => {},
    }
    const { result } = renderHook(() => useSettingsBackupController(port, 'en'))
    await act(async () => { await result.current.importBackup() })
    expect(result.current.status).toMatchObject({ kind: 'err', msg: expect.stringContaining('不支持的配置备份格式或版本') })
    expect(result.current.status?.msg).not.toContain('object')
    await act(async () => { await result.current.importBackup() })
    expect(result.current.status?.kind).toBe('ok')
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
    await act(async () => { await result.current.exportBackup() })
    expect(exported).toHaveBeenLastCalledWith('share.json')
    expect(result.current.status?.kind).toBe('ok')
  })
})

describe('manual WebDAV backup lifecycle', () => {
  function remotePort(): SettingsBackupPort {
    return { pickImport: async () => null, pickExport: async () => null,
      import: vi.fn(async () => {}), export: vi.fn(async () => {}),
      webdav: {
        load: async () => ({ url: 'https://dav.test', username: 'user', directory: 'Kivio', hasPassword: true }),
        save: async () => ({ url: 'https://dav.test', username: 'user', directory: 'Kivio', hasPassword: true }),
        test: async () => {}, list: async () => [], upload: vi.fn(async () => {}),
        download: vi.fn(async () => ({ id: 'download', path: '/private/backup.json' })),
        release: vi.fn(async () => {}), delete: vi.fn(async () => {}),
        confirmRestore: async () => true, confirmDelete: async () => true,
      },
    }
  }
  const backup = { kind: 'configuration' as const, name: 'kivio-configuration-test.json', size: 10, modified: '' }

  it('routes cloud chats through chat confirmation and restore, with cleanup on failure', async () => {
    const port = remotePort()
    const confirmRestore = vi.fn(async () => false)
    const restore = vi.fn(async () => { throw new Error('Invalid chat archive') })
    port.chatBackup = { pickImport: async () => null, pickExport: async () => null, export: async () => {}, confirmRestore, restore }
    const { result } = renderHook(() => useSettingsBackupController(port, 'en'))
    const action = { type: 'restore' as const, backup: { ...backup, kind: 'chat' as const, name: 'kivio-chat-test.zip' }, trusted: true }
    await act(async () => { await result.current.runWebDav(action) })
    expect(port.webdav!.download).not.toHaveBeenCalled()
    confirmRestore.mockResolvedValue(true)
    await act(async () => { await result.current.runWebDav(action) })
    expect(restore).toHaveBeenCalledWith('/private/backup.json', 'download')
    expect(port.import).not.toHaveBeenCalled()
    expect(port.webdav!.release).toHaveBeenCalledWith('download')
    expect(result.current.status?.msg).toContain('Invalid chat archive')
    port.chatBackup.restore = vi.fn(async () => {})
    await act(async () => { await result.current.runWebDav(action) })
    expect(result.current.status?.msg).toContain('Restarting')
  })

  it('requires confirmation before downloading and cleans temporary data after rejected configuration validation', async () => {
    const port = remotePort()
    port.webdav!.confirmRestore = async () => false
    const { result } = renderHook(() => useSettingsBackupController(port, 'en'))
    await act(async () => { await result.current.runWebDav({ type: 'restore', backup, trusted: false }) })
    expect(port.webdav!.download).not.toHaveBeenCalled()
    port.webdav!.confirmRestore = async () => true
    port.import = async () => { throw new Error('Unsupported configuration format') }
    await act(async () => { await result.current.runWebDav({ type: 'restore', backup, trusted: false }) })
    expect(port.webdav!.release).toHaveBeenCalledWith('download')
    expect(result.current.status).toMatchObject({ kind: 'err', msg: expect.stringContaining('Unsupported configuration format') })
    port.import = async () => {}
    await act(async () => { await result.current.runWebDav({ type: 'restore', backup, trusted: false }) })
    expect(result.current.status?.msg).toContain('Configuration restored')
  })

  it.each(['configuration', 'invalid'] as const)('preserves the %s restore outcome when temporary cleanup fails', async (outcome) => {
    const port = remotePort()
    port.webdav!.release = vi.fn(async () => { throw new Error('Cleanup unavailable') })
    if (outcome === 'invalid') port.import = async () => { throw new Error('Unsupported configuration format') }
    const { result } = renderHook(() => useSettingsBackupController(port, 'en'))
    let restored = false
    await act(async () => { restored = await result.current.runWebDav({ type: 'restore', backup, trusted: false }) })
    expect(restored).toBe(outcome !== 'invalid')
    expect(result.current.status?.kind).toBe(outcome === 'invalid' ? 'err' : 'ok')
    expect(result.current.status?.msg).toContain(outcome === 'invalid' ? 'Unsupported configuration format' : 'Configuration restored')
    expect(result.current.status?.msg).toContain('Temporary backup cleanup failed')
  })

  it('blocks overlapping local export and duplicate upload until remote upload completes', async () => {
    const pending = deferred<void>()
    const port = remotePort()
    port.webdav!.upload = vi.fn(() => pending.promise)
    const { result } = renderHook(() => useSettingsBackupController(port, 'en'))
    let upload!: Promise<boolean>
    act(() => { upload = result.current.runWebDav({ type: 'upload', kind: 'configuration' }) })
    await act(async () => {
      await result.current.exportBackup()
      await result.current.runWebDav({ type: 'upload', kind: 'configuration' })
    })
    expect(port.export).not.toHaveBeenCalled()
    expect(port.webdav!.upload).toHaveBeenCalledOnce()
    expect(result.current.busy).toBe(true)
    pending.resolve()
    await act(async () => { await upload })
    expect(result.current.status?.kind).toBe('ok')
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
