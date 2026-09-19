import { useCallback, useEffect, useRef, useState } from 'react'
import { api, type UpdateInfo } from '../api/tauri'

/** Update discovery and installer lifetime are independent of the settings editor. */
export type SettingsUpdatePort = Pick<typeof api,
  'checkUpdate' | 'onUpdateAvailable' | 'onUpdateDownloadProgress'
  | 'downloadUpdate' | 'installUpdate' | 'openExternal'>

export function useSettingsUpdateController(autoCheckUpdate: boolean | null | undefined, port: SettingsUpdatePort = api) {
  const [status, setStatus] = useState<'idle' | 'checking' | 'up-to-date' | 'available' | 'check-failed'>('idle')
  const [info, setInfo] = useState<UpdateInfo | null>(null)
  const [downloadState, setDownloadState] = useState<'idle' | 'downloading' | 'downloaded' | 'failed'>('idle')
  const [downloadPercent, setDownloadPercent] = useState(0)
  const [downloadedPath, setDownloadedPath] = useState('')
  const [downloadError, setDownloadError] = useState('')
  const live = useRef(true)
  const checkSequence = useRef(0)
  const downloadFlight = useRef(false)
  const downloadSequence = useRef(0)
  const upToDateTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(() => {
    live.current = true
    return () => {
      live.current = false
      checkSequence.current += 1
      downloadSequence.current += 1
      if (upToDateTimer.current) clearTimeout(upToDateTimer.current)
    }
  }, [])

  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined
    port.onUpdateAvailable((available) => {
      if (cancelled) return
      checkSequence.current += 1
      setInfo(available)
      setStatus('available')
    }).then((dispose) => {
      if (cancelled) dispose()
      else unlisten = dispose
    })
    return () => { cancelled = true; unlisten?.() }
  }, [port])

  useEffect(() => {
    if (autoCheckUpdate === null || autoCheckUpdate === false) return
    if (status === 'available' || status === 'checking') return
    let cancelled = false
    const sequence = ++checkSequence.current
    port.checkUpdate().then((available) => {
      if (cancelled || !live.current || sequence !== checkSequence.current || !available.available) return
      setInfo(available)
      setStatus('available')
    }).catch(() => {})
    return () => { cancelled = true }
    // Auto-check only when settings first load or the preference changes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [autoCheckUpdate, port])

  const check = useCallback(async () => {
    const sequence = ++checkSequence.current
    setStatus('checking')
    try {
      const available = await port.checkUpdate()
      if (!live.current || sequence !== checkSequence.current) return
      if (available.checkFailed) setStatus('check-failed')
      else if (available.available) {
        setInfo(available)
        setStatus('available')
      } else {
        setStatus('up-to-date')
        if (upToDateTimer.current) clearTimeout(upToDateTimer.current)
        upToDateTimer.current = setTimeout(() => {
          if (live.current && sequence === checkSequence.current) {
            setStatus((current) => current === 'up-to-date' ? 'idle' : current)
          }
          upToDateTimer.current = null
        }, 5000)
      }
    } catch (error) {
      if (!live.current || sequence !== checkSequence.current) return
      console.error('Check update failed:', error)
      setStatus('check-failed')
    }
  }, [port])

  const openGithubReleases = useCallback(async () => {
    try { await port.openExternal('https://github.com/ZMGID/kivio/releases') }
    catch (error) { console.error('Open GitHub releases failed:', error) }
  }, [port])

  const openReleasePage = useCallback(async () => {
    if (!info?.htmlUrl) return
    try { await port.openExternal(info.htmlUrl) }
    catch (error) { console.error('Open release page failed:', error) }
  }, [info, port])

  const downloadAndInstall = useCallback(async () => {
    if (!info?.version || downloadFlight.current) return
    downloadFlight.current = true
    const sequence = ++downloadSequence.current
    setDownloadState('downloading')
    setDownloadPercent(0)
    setDownloadError('')
    let unlisten: (() => void) | undefined
    try {
      unlisten = await port.onUpdateDownloadProgress((progress) => {
        if (live.current && sequence === downloadSequence.current) {
          setDownloadPercent(Math.max(0, Math.min(100, Math.round(progress.percent))))
        }
      })
      const path = await port.downloadUpdate(info.version)
      if (live.current && sequence === downloadSequence.current) {
        setDownloadedPath(path)
        setDownloadState('downloaded')
      }
      await port.installUpdate(path)
    } catch (error) {
      console.error('Download or install update failed:', error)
      if (live.current && sequence === downloadSequence.current) {
        setDownloadError(error instanceof Error ? error.message : String(error))
        setDownloadState('failed')
      }
    } finally {
      unlisten?.()
      downloadFlight.current = false
    }
  }, [info, port])

  const install = useCallback(async () => {
    if (!downloadedPath) return
    try { await port.installUpdate(downloadedPath) }
    catch (error) {
      console.error('Install update failed:', error)
      setDownloadError(error instanceof Error ? error.message : String(error))
      setDownloadState('failed')
    }
  }, [downloadedPath, port])

  const dismiss = useCallback(() => {
    checkSequence.current += 1
    downloadSequence.current += 1
    if (upToDateTimer.current) clearTimeout(upToDateTimer.current)
    upToDateTimer.current = null
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
