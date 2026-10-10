import { useCallback, useEffect, useRef, useState } from 'react'
import type { Lang } from '../components/i18n'

export interface SettingsBackupPort {
  pickImport(): Promise<string | null>
  pickExport(share?: boolean): Promise<string | null>
  import(path: string, trusted?: boolean): Promise<unknown>
  export(path: string, share?: boolean): Promise<unknown>
}

type Status = { kind: 'ok' | 'err'; msg: string } | null

/** Serializes backup file selection and commit, with status tied to the settled operation. */
export function useSettingsBackupController(port: SettingsBackupPort, lang: Lang) {
  const portRef = useRef(port)
  portRef.current = port
  const langRef = useRef(lang)
  langRef.current = lang
  const inFlight = useRef(false)
  const live = useRef(true)
  const [busy, setBusy] = useState(false)
  const [status, setStatus] = useState<Status>(null)

  const run = useCallback(async (kind: 'import' | 'restore' | 'export' | 'share'): Promise<void> => {
    if (inFlight.current) return
    inFlight.current = true
    setBusy(true)
    setStatus(null)
    try {
      const path = (kind === 'import' || kind === 'restore')
        ? await portRef.current.pickImport()
        : kind === 'share' ? await portRef.current.pickExport(true) : await portRef.current.pickExport()
      if (!path) return
      if (kind === 'import') await portRef.current.import(path)
      else if (kind === 'restore') await portRef.current.import(path, true)
      else if (kind === 'share') await portRef.current.export(path, true)
      else await portRef.current.export(path)
      if (!live.current) return
      setStatus({
        kind: 'ok',
        msg: kind === 'import'
          ? (langRef.current === 'zh' ? '已导入。MCP 和 hooks 已禁用，外部 CLI 保留本机配置，工具执行需要确认；检查配置后可重新启用。' : 'Imported. MCP and hooks are disabled, local CLI configuration is retained, and tool execution requires approval. Review settings before enabling them.')
          : kind === 'restore'
            ? (langRef.current === 'zh' ? '已恢复可信备份，包括原有执行权限。' : 'Trusted backup restored, including execution permissions.')
            : kind === 'share'
              ? (langRef.current === 'zh' ? '已导出分享配置。请检查集和自定义内容后再分享。' : 'Share configuration exported. Review sets and custom content before sharing.')
          : (langRef.current === 'zh' ? '备份已导出。' : 'Backup exported.'),
      })
    } catch (error) {
      if (!live.current) return
      const prefix = (kind === 'import' || kind === 'restore')
        ? (langRef.current === 'zh' ? '导入失败：' : 'Import failed: ')
        : (langRef.current === 'zh' ? '导出失败：' : 'Export failed: ')
      setStatus({ kind: 'err', msg: `${prefix}${error instanceof Error ? error.message : String(error)}` })
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
  const exportBackup = useCallback(() => run('export'), [run])
  const restoreBackup = useCallback(() => run('restore'), [run])
  const shareBackup = useCallback(() => run('share'), [run])
  return { busy, status, importBackup, exportBackup, restoreBackup, shareBackup }
}
