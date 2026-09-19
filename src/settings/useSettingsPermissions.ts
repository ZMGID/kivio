import { useCallback, useEffect, useRef, useState } from 'react'
import { api, type PermissionStatus } from '../api/tauri'

/** System permission status has its own refresh lifetime, separate from draft editing. */
export function useSettingsPermissions() {
  const [status, setStatus] = useState<PermissionStatus | null>(null)
  const [loading, setLoading] = useState(false)
  const refreshSequence = useRef(0)
  const refresh = useCallback(async () => {
    const sequence = ++refreshSequence.current
    setLoading(true)
    try {
      const fresh = await api.getPermissionStatus()
      if (sequence === refreshSequence.current) setStatus(fresh)
    } catch (error) {
      if (sequence === refreshSequence.current) console.error('Failed to get permission status:', error)
    } finally {
      if (sequence === refreshSequence.current) setLoading(false)
    }
  }, [])
  const open = useCallback(async (kind: 'accessibility' | 'screen-recording') => {
    try { await api.openPermissionSettings(kind) }
    catch (error) { console.error('Failed to open permission settings:', error) }
  }, [])
  useEffect(() => {
    void refresh()
    return () => { refreshSequence.current += 1 }
  }, [refresh])
  return { status, loading, refresh, open }
}
