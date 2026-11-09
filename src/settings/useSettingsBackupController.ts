import { useCallback, useEffect, useRef, useState } from 'react'
import type { WebDavConfig, WebDavConfigInput, WebDavBackup, WebDavDownload } from '../generated/webdav'
import type { Lang } from '../components/i18n'

export interface WebDavBackupPort {
  load(): Promise<WebDavConfig>
  save(input: WebDavConfigInput): Promise<WebDavConfig>
  test(input: WebDavConfigInput): Promise<void>
  list(): Promise<WebDavBackup[]>
  upload(): Promise<void>
  download(name: string): Promise<WebDavDownload>
  release(id: string): Promise<void>
  delete(name: string): Promise<void>
  confirmRestore(): Promise<boolean>
  confirmDelete(name: string): Promise<boolean>
}
type WebDavAction = { type: 'load' | 'list' } | { type: 'save' | 'test'; input: WebDavConfigInput }
  | { type: 'upload' } | { type: 'restore'; backup: WebDavBackup; trusted: boolean }
  | { type: 'delete'; name: string }
export interface SettingsBackupPort {
  webdav?: WebDavBackupPort
  pickImport(): Promise<string | null>
  pickExport(): Promise<string | null>
  import(path: string, trusted?: boolean): Promise<unknown>
  export(path: string): Promise<void>
}
/** Tauri rejects with strings, Errors or structured `{ code, message }` payloads (settings commands). */
function errorText(error: unknown): string {
  if (error instanceof Error) return error.message
  if (typeof error === 'object' && error !== null && 'message' in error && typeof error.message === 'string') return error.message
  return typeof error === 'string' ? error : JSON.stringify(error) ?? String(error)
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
  const [webdavConfig, setWebdavConfig] = useState<WebDavConfig | null>(null)
  const [webdavFiles, setWebdavFiles] = useState<WebDavBackup[] | null>(null)
  const [webdavPhase, setWebdavPhase] = useState('')
  const [statusSource, setStatusSource] = useState<'local' | 'webdav'>('local')

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
        if (!await remote.confirmRestore()) return false
        phase(zh ? '正在下载备份…' : 'Downloading backup…')
        const download = await remote.download(action.backup.name)
        try {
          phase(zh ? '正在校验并恢复备份…' : 'Validating and restoring backup…')
          await portRef.current.import(download.path, action.trusted)
        } finally {
          try { await remote.release(download.id) }
          catch (error) {
            cleanupWarning = ` ${zh ? '临时备份文件清理失败：' : 'Temporary backup cleanup failed: '}${errorText(error)}`
          }
        }
      } else {
        if (action.type === 'upload') {
          phase(zh ? '正在生成并上传备份，请稍候…' : 'Creating and uploading backup…')
          await remote.upload()
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
            throw new Error(`${zh ? '操作已完成，但列表刷新失败，请重新刷新：' : 'Operation completed, but the list could not refresh. Refresh again: '}${errorText(error)}`)
          }
          throw error
        }
      }
      if (live.current) setStatus({ kind: 'ok', msg: (action.type === 'restore'
        ? (zh ? '配置已恢复。' : 'Configuration restored.')
        : action.type === 'test' ? (zh ? '连接测试成功。' : 'Connection test succeeded.')
        : action.type === 'save' ? (zh ? 'WebDAV 配置已保存。' : 'WebDAV configuration saved.')
        : action.type === 'upload' ? (zh ? '备份已上传。' : 'Backup uploaded.')
        : action.type === 'delete' ? (zh ? '远程备份已删除。' : 'Remote backup deleted.')
        : (zh ? '备份列表已更新。' : 'Backup list updated.')) + cleanupWarning })
      return true
    } catch (error) {
      if (live.current) setStatus({ kind: 'err', msg: `${zh ? '操作失败：' : 'Operation failed: '}${errorText(error)}${cleanupWarning}` })
      return false
    } finally {
      inFlight.current = false
      if (live.current) { setBusy(false); setWebdavPhase('') }
    }
  }, [])

  const run = useCallback(async (action: 'import' | 'restore' | 'export'): Promise<void> => {
    if (inFlight.current) return
    inFlight.current = true
    setBusy(true)
    setStatusSource('local')
    setStatus(null)
    const zh = langRef.current === 'zh'
    try {
      const importing = action === 'import' || action === 'restore'
      const path = importing ? await portRef.current.pickImport() : await portRef.current.pickExport()
      if (!path) return
      if (importing) {
        if (action === 'restore') await portRef.current.import(path, true)
        else await portRef.current.import(path)
      } else await portRef.current.export(path)
      if (!live.current) return
      setStatus({ kind: 'ok', msg: importing
        ? action === 'restore'
          ? (zh ? '配置已恢复，包括原有执行权限。' : 'Configuration restored, including execution permissions.')
          : (zh ? '配置已导入。MCP 和 hooks 已禁用，外部 CLI 保留本机配置。' : 'Configuration imported. MCP and hooks are disabled; local CLI configuration is retained.')
        : (zh ? '配置已导出。' : 'Configuration exported.'),
      })
    } catch (error) {
      if (!live.current) return
      setStatus({ kind: 'err', msg: `${zh ? '操作失败：' : 'Operation failed: '}${errorText(error)}` })
    } finally {
      inFlight.current = false
      if (live.current) setBusy(false)
    }
  }, [])
  useEffect(() => {
    live.current = true
    return () => { live.current = false }
  }, [])
  const importBackup = useCallback(() => run('import'), [run])
  const restoreBackup = useCallback(() => run('restore'), [run])
  const exportBackup = useCallback(() => run('export'), [run])
  return { busy, status, statusSource, webdavConfig, webdavFiles, webdavPhase, runWebDav, importBackup, restoreBackup, exportBackup }
}
