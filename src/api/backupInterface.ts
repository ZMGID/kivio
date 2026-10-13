import { api } from './tauri'

const RESTORED_ID = 'kivio-backup-restored-id'
const portable = (key: string) => (key.startsWith('kivio-') || key.startsWith('kivio.') || key.startsWith('kivio:')) && key !== RESTORED_ID
  && !key.includes('window-geometry') && !key.includes('window-size')

export function captureBackupInterface(): Record<string, string> {
  const values: Record<string, string> = {}
  for (let i = 0; i < window.localStorage.length; i++) {
    const key = window.localStorage.key(i)
    if (key && portable(key)) values[key] = window.localStorage.getItem(key) ?? ''
  }
  return values
}

/** Restore once per WebView storage origin, before any component reads preferences. */
export async function restoreBackupInterface(): Promise<void> {
  const restored = await api.backupInterface()
  if (!restored || window.localStorage.getItem(RESTORED_ID) === restored.id) return
  for (const key of Object.keys(captureBackupInterface())) window.localStorage.removeItem(key)
  for (const [key, value] of Object.entries(restored.values)) {
    if (portable(key) && typeof value === 'string') window.localStorage.setItem(key, value)
  }
  window.localStorage.setItem(RESTORED_ID, restored.id)
}
