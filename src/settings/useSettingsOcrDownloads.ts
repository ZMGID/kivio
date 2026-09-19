import { useCallback, useEffect, useReducer, useState } from 'react'
import {
  api,
  type RapidOcrStatus,
  type RapidOcrTier,
  type ReplaceTranslationPackStatus,
} from '../api/tauri'
import { initialReplacePackProgressState, reduceReplacePackProgress } from './replacePackProgress'

/** Owns OCR package status, download events, and install progress independently of draft saves. */
export function useSettingsOcrDownloads(enabled: boolean, tier: RapidOcrTier) {
  const [rapidStatus, setRapidStatus] = useState<RapidOcrStatus | null>(null)
  const [rapidDownloadState, setRapidDownloadState] = useState<'idle' | 'downloading' | 'failed'>('idle')
  const [rapidDownloadError, setRapidDownloadError] = useState('')
  const [replaceStatus, setReplaceStatus] = useState<ReplaceTranslationPackStatus | null>(null)
  const [replaceDownload, dispatchReplaceDownload] = useReducer(
    reduceReplacePackProgress,
    initialReplacePackProgressState,
  )

  const refreshRapid = useCallback(async () => {
    if (!enabled) return
    try { setRapidStatus(await api.rapidOcrStatus()) }
    catch (error) { console.error('rapidOcrStatus failed:', error) }
  }, [enabled])

  const downloadRapid = useCallback(async (selectedTier: RapidOcrTier) => {
    setRapidDownloadState('downloading')
    setRapidDownloadError('')
    try {
      const result = await api.rapidOcrInstall(selectedTier)
      if (result.success) {
        setRapidDownloadState('idle')
        await refreshRapid()
      } else {
        setRapidDownloadError(result.message)
        setRapidDownloadState('failed')
      }
    } catch (error) {
      setRapidDownloadError(error instanceof Error ? error.message : String(error))
      setRapidDownloadState('failed')
    }
  }, [refreshRapid])

  const refreshReplace = useCallback(async (selectedTier: RapidOcrTier) => {
    if (!enabled) return
    try { setReplaceStatus(await api.replaceTranslationPackStatus(selectedTier)) }
    catch (error) { console.error('replaceTranslationPackStatus failed:', error) }
  }, [enabled])

  const downloadReplace = useCallback(async (selectedTier: RapidOcrTier) => {
    dispatchReplaceDownload({ type: 'start' })
    try {
      const result = await api.replaceTranslationPackInstall(selectedTier)
      if (result.success) {
        dispatchReplaceDownload({ type: 'success' })
        await Promise.all([refreshReplace(selectedTier), refreshRapid()])
      } else {
        dispatchReplaceDownload({ type: 'failure', error: result.message })
      }
    } catch (error) {
      dispatchReplaceDownload({ type: 'failure', error: error instanceof Error ? error.message : String(error) })
    }
  }, [refreshRapid, refreshReplace])

  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined
    api.onReplaceTranslationPackProgress((progress) => {
      if (cancelled || progress.pack !== 'replace_translation') return
      dispatchReplaceDownload({ type: 'progress', progress })
      if (progress.state === 'completed' && progress.overallDownloadedBytes >= progress.overallTotalBytes) {
        void refreshReplace(tier)
      }
    }).then((dispose) => {
      if (cancelled) dispose()
      else unlisten = dispose
    }).catch((error) => console.error('replace translation pack progress listener failed:', error))
    return () => { cancelled = true; unlisten?.() }
  }, [refreshReplace, tier])

  useEffect(() => { void refreshRapid() }, [refreshRapid])
  useEffect(() => { void refreshReplace(tier) }, [refreshReplace, tier])

  return {
    rapidStatus, rapidDownloadState, rapidDownloadError,
    replaceStatus, replaceDownload,
    refreshRapid, downloadRapid, refreshReplace, downloadReplace,
  }
}
