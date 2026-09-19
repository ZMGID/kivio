import { useCallback, useEffect, useReducer, useRef, useState } from 'react'
import {
  api,
  type RapidOcrStatus,
  type RapidOcrTier,
  type ReplaceTranslationPackStatus,
} from '../api/tauri'
import { initialReplacePackProgressState, reduceReplacePackProgress } from './replacePackProgress'

/** Owns OCR package status, download events, and install progress independently of draft saves. */
export type SettingsOcrPort = Pick<typeof api,
  'rapidOcrStatus' | 'rapidOcrInstall' | 'replaceTranslationPackStatus'
  | 'replaceTranslationPackInstall' | 'onReplaceTranslationPackProgress'>

export function useSettingsOcrDownloads(enabled: boolean, tier: RapidOcrTier, port: SettingsOcrPort = api) {
  const [rapidStatus, setRapidStatus] = useState<RapidOcrStatus | null>(null)
  const [rapidDownloadState, setRapidDownloadState] = useState<'idle' | 'downloading' | 'failed'>('idle')
  const [rapidDownloadError, setRapidDownloadError] = useState('')
  const [replaceStatus, setReplaceStatus] = useState<ReplaceTranslationPackStatus | null>(null)
  const [replaceDownload, dispatchReplaceDownload] = useReducer(
    reduceReplacePackProgress,
    initialReplacePackProgressState,
  )
  const live = useRef(true)
  const rapidRefreshSequence = useRef(0)
  const replaceRefreshSequence = useRef(0)
  const rapidDownloading = useRef(false)
  const replaceDownloading = useRef(false)
  const tierRef = useRef(tier)
  tierRef.current = tier

  useEffect(() => {
    live.current = true
    return () => {
      live.current = false
      rapidRefreshSequence.current += 1
      replaceRefreshSequence.current += 1
    }
  }, [])

  const refreshRapid = useCallback(async () => {
    if (!enabled) return
    const sequence = ++rapidRefreshSequence.current
    try {
      const status = await port.rapidOcrStatus()
      if (live.current && sequence === rapidRefreshSequence.current) setRapidStatus(status)
    } catch (error) {
      if (live.current && sequence === rapidRefreshSequence.current) console.error('rapidOcrStatus failed:', error)
    }
  }, [enabled, port])

  const downloadRapid = useCallback(async (selectedTier: RapidOcrTier) => {
    if (rapidDownloading.current) return
    rapidDownloading.current = true
    setRapidDownloadState('downloading')
    setRapidDownloadError('')
    try {
      const result = await port.rapidOcrInstall(selectedTier)
      if (result.success) {
        if (live.current) setRapidDownloadState('idle')
        await refreshRapid()
      } else {
        if (live.current) {
          setRapidDownloadError(result.message)
          setRapidDownloadState('failed')
        }
      }
    } catch (error) {
      if (live.current) {
        setRapidDownloadError(error instanceof Error ? error.message : String(error))
        setRapidDownloadState('failed')
      }
    } finally {
      rapidDownloading.current = false
    }
  }, [port, refreshRapid])

  const refreshReplace = useCallback(async (selectedTier: RapidOcrTier) => {
    if (!enabled || selectedTier !== tierRef.current) return
    const sequence = ++replaceRefreshSequence.current
    try {
      const status = await port.replaceTranslationPackStatus(selectedTier)
      if (live.current && selectedTier === tierRef.current && sequence === replaceRefreshSequence.current) {
        setReplaceStatus(status)
      }
    } catch (error) {
      if (live.current && sequence === replaceRefreshSequence.current) console.error('replaceTranslationPackStatus failed:', error)
    }
  }, [enabled, port])

  const downloadReplace = useCallback(async (selectedTier: RapidOcrTier) => {
    if (replaceDownloading.current) return
    replaceDownloading.current = true
    dispatchReplaceDownload({ type: 'start' })
    try {
      const result = await port.replaceTranslationPackInstall(selectedTier)
      if (result.success) {
        if (live.current) dispatchReplaceDownload({ type: 'success' })
        await Promise.all([refreshReplace(selectedTier), refreshRapid()])
      } else {
        if (live.current) dispatchReplaceDownload({ type: 'failure', error: result.message })
      }
    } catch (error) {
      if (live.current) dispatchReplaceDownload({ type: 'failure', error: error instanceof Error ? error.message : String(error) })
    } finally {
      replaceDownloading.current = false
    }
  }, [port, refreshRapid, refreshReplace])

  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined
    port.onReplaceTranslationPackProgress((progress) => {
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
  }, [port, refreshReplace, tier])

  useEffect(() => { void refreshRapid() }, [refreshRapid])
  useEffect(() => { void refreshReplace(tier) }, [refreshReplace, tier])

  return {
    rapidStatus, rapidDownloadState, rapidDownloadError,
    replaceStatus, replaceDownload,
    refreshRapid, downloadRapid, refreshReplace, downloadReplace,
  }
}
