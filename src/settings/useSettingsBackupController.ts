import { useCallback, useEffect, useRef, useState } from 'react'
import type { WebDavConfig, WebDavConfigInput, WebDavBackup, WebDavDownload } from '../generated/webdav'
import type { Lang } from '../components/i18n'

export type BackupKind = 'configuration' | 'full'
export interface WebDavBackupPort {
  load(): Promise<WebDavConfig>
  save(input: WebDavConfigInput): Promise<WebDavConfig>
  test(input: WebDavConfigInput): Promise<void>
  list(): Promise<WebDavBackup[]>
  upload(kind: BackupKind): Promise<string>
  download(name: string): Promise<WebDavDownload>
  release(id: string): Promise<void>
  delete(name: string): Promise<void>
  confirmRestore(kind: BackupKind): Promise<boolean>
  confirmDelete(name: string): Promise<boolean>
}
type WebDavAction = { type: 'load' | 'list' } | { type: 'save' | 'test'; input: WebDavConfigInput }
  | { type: 'upload'; kind: BackupKind } | { type: 'restore'; backup: WebDavBackup; trusted: boolean }
  | { type: 'delete'; name: string }
export interface SettingsBackupPort {
  webdav?: WebDavBackupPort
  pickImport(): Promise<string | null>
  pickExport(kind: BackupKind): Promise<string | null>
  import(path: string, trusted?: boolean): Promise<unknown>
  export(path: string, kind: BackupKind): Promise<unknown>
  importFull(path: string, trusted: boolean): Promise<void>
  confirmFullRestore(): Promise<boolean>
  restart(): Promise<void>
  pendingRestore(): Promise<boolean>
}
type Status = { kind: 'ok' | 'err'; msg: string } | null

/** Owns one file-selection/backup operation at a time, including restore confirmation. */
export function useSettingsBackupController(port: SettingsBackupPort, lang: Lang) {
  const portRef = useRef(port)
  portRef.current = port
  const langRef = useRef(lang)
  langRef.current = lang
  const inFlight = useRef(false)
  const live = useRef(true)
  const [busy, setBusy] = useState(false)
  const [status, setStatus] = useState<Status>(null)
  const [restartRequired, setRestartRequired] = useState(false)
  const [webdavConfig, setWebdavConfig] = useState<WebDavConfig | null>(null)
  const [webdavFiles, setWebdavFiles] = useState<WebDavBackup[] | null>(null)
  const [webdavPhase, setWebdavPhase] = useState('')
  const [statusSource, setStatusSource] = useState<'local' | 'webdav'>('local')

  // Shares the local backup gate: leaving the settings page does not cancel a committed restore.
  const runWebDav = useCallback(async (action: WebDavAction): Promise<boolean> => {
    const remote = portRef.current.webdav
    if (!remote || inFlight.current) return false
    inFlight.current = true
    setBusy(true)
    setStatusSource('webdav')
    setStatus(null)
    const zh = langRef.current === 'zh'
    let cleanupWarning = ''
    const phase = (text: string) => { if (live.current) setWebdavPhase(text) }
    try {
      if (await portRef.current.pendingRestore()) {
        if (live.current) {
          setRestartRequired(true)
          setStatus({ kind: 'ok', msg: zh ? '已有待恢复备份，请重启完成恢复。' : 'Restart to finish the pending restore.' })
        }
        return false
      }
      if (action.type === 'load') {
        const config = await remote.load()
        if (live.current) setWebdavConfig(config)
        return false
      }
      if (action.type === 'save') {
        const config = await remote.save(action.input)
        if (live.current) { setWebdavConfig(config); setWebdavFiles(null) }
      } else if (action.type === 'test') {
        phase(zh ? '正在测试读取、写入和删除权限…' : 'Testing read, write and delete access…')
        await remote.test(action.input)
      } else if (action.type === 'restore') {
        if (!await remote.confirmRestore(action.backup.kind)) return false
        phase(zh ? '正在下载备份…' : 'Downloading backup…')
        const download = await remote.download(action.backup.name)
        try {
          phase(zh ? '正在校验并恢复备份…' : 'Validating and restoring backup…')
          if (action.backup.kind === 'full') {
            await portRef.current.importFull(download.path, action.trusted)
            if (live.current) setRestartRequired(true)
          } else await portRef.current.import(download.path, action.trusted)
        } finally {
          try { await remote.release(download.id) }
          catch (error) {
            cleanupWarning = ` ${zh ? '临时备份文件清理失败：' : 'Temporary backup cleanup failed: '}${error instanceof Error ? error.message : String(error)}`
          }
        }
      } else {
        if (action.type === 'upload') {
          phase(zh ? '正在生成并上传备份，请稍候…' : 'Creating and uploading backup…')
          await remote.upload(action.kind)
        } else if (action.type === 'delete') {
          if (!await remote.confirmDelete(action.name)) return false
          await remote.delete(action.name)
        }
        phase(zh ? '正在读取远程备份列表…' : 'Loading remote backups…')
        try {
          const files = await remote.list()
          if (live.current) setWebdavFiles(files)
        } catch (error) {
          if (live.current) setWebdavFiles(null)
          if (action.type === 'upload' || action.type === 'delete') {
            throw new Error(`${zh ? '操作已完成，但列表刷新失败，请重新刷新：' : 'Operation completed, but the list could not refresh. Refresh again: '}${String(error)}`)
          }
          throw error
        }
      }
      if (live.current) setStatus({ kind: 'ok', msg: (action.type === 'restore'
        ? action.backup.kind === 'full'
          ? (zh ? '备份已校验，重启后完成恢复。当前数据会保留副本。' : 'Backup validated. Restart to restore; previous data will be retained.')
          : (zh ? '配置已恢复。' : 'Configuration restored.')
        : action.type === 'test' ? (zh ? '连接测试成功。' : 'Connection test succeeded.')
        : action.type === 'save' ? (zh ? 'WebDAV 配置已保存。' : 'WebDAV configuration saved.')
        : action.type === 'upload' ? (zh ? '备份已上传。' : 'Backup uploaded.')
        : action.type === 'delete' ? (zh ? '远程备份已删除。' : 'Remote backup deleted.')
        : (zh ? '备份列表已更新。' : 'Backup list updated.')) + cleanupWarning })
      return true
    } catch (error) {
      if (live.current) setStatus({ kind: 'err', msg: `${zh ? '操作失败：' : 'Operation failed: '}${error instanceof Error ? error.message : String(error)}${cleanupWarning}` })
      return false
    } finally {
      inFlight.current = false
      if (live.current) { setBusy(false); setWebdavPhase('') }
    }
  }, [])

  const run = useCallback(async (action: 'import' | 'restore' | BackupKind | 'restart'): Promise<void> => {
    if (inFlight.current) return
    inFlight.current = true
    setBusy(true)
    setStatusSource('local')
    setStatus(null)
    const zh = langRef.current === 'zh'
    try {
      if (action === 'restart') { await portRef.current.restart(); return }
      if (await portRef.current.pendingRestore()) {
        if (live.current) {
          setRestartRequired(true)
          setStatus({ kind: 'ok', msg: zh ? '已有待恢复备份，请重启完成恢复。' : 'A backup is ready to restore. Restart to finish.' })
        }
        return
      }
      const importing = action === 'import' || action === 'restore'
      const path = importing ? await portRef.current.pickImport() : await portRef.current.pickExport(action)
      if (!path) return
      let fullRestore = false
      if (importing) {
        fullRestore = /\.(kivio|zip)$/i.test(path)
        if (fullRestore) {
          if (!await portRef.current.confirmFullRestore()) return
          await portRef.current.importFull(path, action === 'restore')
        } else if (action === 'restore') await portRef.current.import(path, true)
        else await portRef.current.import(path)
      } else await portRef.current.export(path, action)
      if (!live.current) return
      if (fullRestore) setRestartRequired(true)
      setStatus({ kind: 'ok', msg: fullRestore
        ? (zh ? '备份已校验。重启后完成恢复；恢复前的数据会保留一份副本。' : 'Backup validated. Restart to restore. A copy of your previous data will be kept.')
        : importing
          ? action === 'restore'
            ? (zh ? '配置已恢复，包括原有执行权限。' : 'Configuration restored, including execution permissions.')
            : (zh ? '配置已导入。MCP 和 hooks 已禁用，外部 CLI 保留本机配置。' : 'Configuration imported. MCP and hooks are disabled; local CLI configuration is retained.')
          : action === 'full'
            ? (zh ? '完全备份已导出。' : 'Full backup exported.')
            : (zh ? '配置已导出。' : 'Configuration exported.'),
      })
    } catch (error) {
      if (!live.current) return
      setStatus({ kind: 'err', msg: `${zh ? '操作失败：' : 'Operation failed: '}${error instanceof Error ? error.message : String(error)}` })
    } finally {
      inFlight.current = false
      if (live.current) setBusy(false)
    }
  }, [])
  useEffect(() => {
    live.current = true
    let current = true
    void portRef.current.pendingRestore().then((pending) => {
      if (current && pending) setRestartRequired(true)
    }).catch((error) => {
      if (current) setStatus({ kind: 'err', msg: String(error) })
    })
    return () => { current = false; live.current = false }
  }, [])
  const importBackup = useCallback(() => run('import'), [run])
  const restoreBackup = useCallback(() => run('restore'), [run])
  const exportBackup = useCallback(() => run('configuration'), [run])
  const exportFullBackup = useCallback(() => run('full'), [run])
  const restart = useCallback(() => run('restart'), [run])
  return { busy, status, statusSource, restartRequired, webdavConfig, webdavFiles, webdavPhase, runWebDav, importBackup, restoreBackup, exportBackup, exportFullBackup, restart }
}
