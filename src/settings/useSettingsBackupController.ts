import { useCallback, useEffect, useRef, useState } from 'react'
import type { Lang } from '../components/i18n'

export interface SettingsBackupPort {
  pickImport(): Promise<string | null>
  pickExport(): Promise<string | null>
  import(path: string): Promise<unknown>
  export(path: string): Promise<unknown>
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

  const run = useCallback(async (kind: 'import' | 'export'): Promise<void> => {
    if (inFlight.current) return
    inFlight.current = true
    setBusy(true)
    setStatus(null)
    try {
      const path = kind === 'import'
        ? await portRef.current.pickImport()
        : await portRef.current.pickExport()
      if (!path) return
      if (kind === 'import') await portRef.current.import(path)
      else await portRef.current.export(path)
      if (!live.current) return
      setStatus({
        kind: 'ok',
        msg: kind === 'import'
          ? (langRef.current === 'zh' ? '设置已导入并生效。' : 'Settings imported and applied.')
          : (langRef.current === 'zh' ? '设置已导出。' : 'Settings exported.'),
      })
    } catch (error) {
      if (!live.current) return
      const prefix = kind === 'import'
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
  return { busy, status, importBackup, exportBackup }
}
