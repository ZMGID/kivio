import { useCallback, useEffect, useState } from 'react'
import { api, type UpdateInfo } from '../api/tauri'

/** Update discovery and installer lifetime are independent of the settings editor. */
export function useSettingsUpdateController(autoCheckUpdate: boolean | null | undefined) {
  const [status, setStatus] = useState<'idle' | 'checking' | 'up-to-date' | 'available' | 'check-failed'>('idle')
  const [info, setInfo] = useState<UpdateInfo | null>(null)
  const [downloadState, setDownloadState] = useState<'idle' | 'downloading' | 'downloaded' | 'failed'>('idle')
  const [downloadPercent, setDownloadPercent] = useState(0)
  const [downloadedPath, setDownloadedPath] = useState('')
  const [downloadError, setDownloadError] = useState('')

  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined
    api.onUpdateAvailable((available) => {
      if (cancelled) return
      setInfo(available)
      setStatus('available')
    }).then((dispose) => {
      if (cancelled) dispose()
      else unlisten = dispose
    })
    return () => { cancelled = true; unlisten?.() }
  }, [])

  useEffect(() => {
    if (autoCheckUpdate === null || autoCheckUpdate === false) return
    if (status === 'available' || status === 'checking') return
    let cancelled = false
    api.checkUpdate().then((available) => {
      if (cancelled || !available.available) return
      setInfo(available)
      setStatus('available')
    }).catch(() => {})
    return () => { cancelled = true }
    // Auto-check only when settings first load or the preference changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [autoCheckUpdate])

  const check = useCallback(async () => {
    setStatus('checking')
    try {
      const available = await api.checkUpdate()
      if (available.checkFailed) setStatus('check-failed')
      else if (available.available) {
        setInfo(available)
        setStatus('available')
      } else {
        setStatus('up-to-date')
        setTimeout(() => setStatus((current) => current === 'up-to-date' ? 'idle' : current), 5000)
      }
    } catch (error) {
      console.error('Check update failed:', error)
      setStatus('check-failed')
    }
  }, [])

  const openGithubReleases = useCallback(async () => {
    try { await api.openExternal('https://github.com/ZMGID/kivio/releases') }
    catch (error) { console.error('Open GitHub releases failed:', error) }
  }, [])

  const openReleasePage = useCallback(async () => {
    if (!info?.htmlUrl) return
    try { await api.openExternal(info.htmlUrl) }
    catch (error) { console.error('Open release page failed:', error) }
  }, [info])

  const downloadAndInstall = useCallback(async () => {
    if (!info?.version) return
    setDownloadState('downloading')
    setDownloadPercent(0)
    setDownloadError('')
    let unlisten: (() => void) | undefined
    try {
      unlisten = await api.onUpdateDownloadProgress((progress) => {
        setDownloadPercent(Math.max(0, Math.min(100, Math.round(progress.percent))))
      })
      const path = await api.downloadUpdate(info.version)
      setDownloadedPath(path)
      setDownloadState('downloaded')
      await api.installUpdate(path)
    } catch (error) {
      console.error('Download or install update failed:', error)
      setDownloadError(error instanceof Error ? error.message : String(error))
      setDownloadState('failed')
    } finally {
      unlisten?.()
    }
  }, [info])

  const install = useCallback(async () => {
    if (!downloadedPath) return
    try { await api.installUpdate(downloadedPath) }
    catch (error) {
      console.error('Install update failed:', error)
      setDownloadError(error instanceof Error ? error.message : String(error))
      setDownloadState('failed')
    }
  }, [downloadedPath])

  const dismiss = useCallback(() => {
    setStatus('idle')
    setDownloadState('idle')
    setDownloadPercent(0)
    setDownloadError('')
  }, [])

  return {
    status, info, downloadState, downloadPercent, downloadError,
    check, downloadAndInstall, install, openReleasePage, openGithubReleases, dismiss,
  }
}
