import { useCallback, useEffect, useRef, useState } from 'react'
import type { Lang } from '../components/i18n'

export type BackupKind = 'configuration' | 'full'
export interface SettingsBackupPort {
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

  const run = useCallback(async (action: 'import' | 'restore' | BackupKind | 'restart'): Promise<void> => {
    if (inFlight.current) return
    inFlight.current = true
    setBusy(true)
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
  return { busy, status, restartRequired, importBackup, restoreBackup, exportBackup, exportFullBackup, restart }
}
