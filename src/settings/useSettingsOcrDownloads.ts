import { useCallback, useEffect } from 'react'
import {
  api,
  type OfflineModelProgress,
  type RapidOcrStatus,
  type RapidOcrTier,
  type ReplaceTranslationPackStatus,
} from '../api/tauri'
import { createWindowStore, useWindowStore } from '../utils/windowStore'
import { initialReplacePackProgressState, reduceReplacePackProgress, type ReplacePackProgressState } from './replacePackProgress'

/** Owns OCR package status, download events, and install progress independently of draft saves. */
export type SettingsOcrPort = Pick<typeof api,
  'rapidOcrStatus' | 'rapidOcrInstall' | 'replaceTranslationPackStatus'
  | 'replaceTranslationPackInstall' | 'onReplaceTranslationPackProgress'>

type OcrDownloadsState = {
  rapidStatus: RapidOcrStatus | null
  rapidDownloadState: 'idle' | 'downloading' | 'failed'
  rapidDownloadError: string
  replaceStatus: ReplaceTranslationPackStatus | null
  replaceDownload: ReplacePackProgressState
}

const initialOcrDownloadsState: OcrDownloadsState = {
  rapidStatus: null,
  rapidDownloadState: 'idle',
  rapidDownloadError: '',
  replaceStatus: null,
  replaceDownload: initialReplacePackProgressState,
}

/** Settings screenshot OCR and the knowledge-base RapidOCR widget share this flight. */
const ocrDownloadsStore = createWindowStore(initialOcrDownloadsState)

let epoch = 0
let rapidRefreshSequence = 0
let replaceRefreshSequence = 0
let observedReplaceTier: RapidOcrTier = 'standard'
let replaceRefreshEnabled = false
let progressConsumers = 0
let replaceFlightActive = false
let progressUnlisten: (() => void) | null = null
let progressSubscribing = false
let progressGeneration = 0
let progressPort: SettingsOcrPort | null = null

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

function wantsProgressSubscription() {
  return progressConsumers > 0 || replaceFlightActive
}

function dropProgressSubscription() {
  progressGeneration += 1
  progressSubscribing = false
  progressPort = null
  const dispose = progressUnlisten
  progressUnlisten = null
  dispose?.()
}

function handleReplaceProgress(port: SettingsOcrPort, progress: OfflineModelProgress) {
  if (progress.pack !== 'replace_translation') return
  ocrDownloadsStore.setState((state) => ({
    ...state,
    replaceDownload: reduceReplacePackProgress(state.replaceDownload, { type: 'progress', progress }),
  }))
  if (
    progress.state === 'completed'
    && progress.overallDownloadedBytes >= progress.overallTotalBytes
  ) {
    void pullReplaceStatus(port, replaceRefreshEnabled, observedReplaceTier)
  }
}

function beginProgressSubscription(port: SettingsOcrPort) {
  if (progressUnlisten || progressSubscribing) return
  progressSubscribing = true
  progressPort = port
  const generation = progressGeneration
  port.onReplaceTranslationPackProgress((progress) => {
    if (generation === progressGeneration) handleReplaceProgress(port, progress)
  }).then((dispose) => {
    if (generation !== progressGeneration) {
      dispose()
      return
    }
    progressSubscribing = false
    if (!wantsProgressSubscription()) {
      dispose()
      return
    }
    progressUnlisten = dispose
  }).catch((error) => {
    if (generation === progressGeneration) progressSubscribing = false
    console.error('replace translation pack progress listener failed:', error)
  })
}

function syncProgressSubscription(port: SettingsOcrPort) {
  if (!wantsProgressSubscription()) {
    dropProgressSubscription()
    return
  }
  if (progressUnlisten && progressPort === port) return
  if ((progressUnlisten || progressSubscribing) && progressPort !== port) dropProgressSubscription()
  beginProgressSubscription(port)
}

function retainProgressConsumer(port: SettingsOcrPort) {
  progressConsumers += 1
  syncProgressSubscription(port)
}

function releaseProgressConsumer() {
  progressConsumers = Math.max(0, progressConsumers - 1)
  if (!wantsProgressSubscription()) dropProgressSubscription()
}

async function pullRapidStatus(port: SettingsOcrPort, enabled: boolean) {
  if (!enabled) return
  const sequence = ++rapidRefreshSequence
  try {
    const status = await port.rapidOcrStatus()
    if (sequence === rapidRefreshSequence) {
      ocrDownloadsStore.setState((state) => ({ ...state, rapidStatus: status }))
    }
  } catch (error) {
    if (sequence === rapidRefreshSequence) console.error('rapidOcrStatus failed:', error)
  }
}

async function pullReplaceStatus(port: SettingsOcrPort, enabled: boolean, selectedTier: RapidOcrTier) {
  if (!enabled || selectedTier !== observedReplaceTier) return
  const sequence = ++replaceRefreshSequence
  try {
    const status = await port.replaceTranslationPackStatus(selectedTier)
    if (selectedTier === observedReplaceTier && sequence === replaceRefreshSequence) {
      ocrDownloadsStore.setState((state) => ({ ...state, replaceStatus: status }))
    }
  } catch (error) {
    if (sequence === replaceRefreshSequence) console.error('replaceTranslationPackStatus failed:', error)
  }
}

function downloadRapidPack(port: SettingsOcrPort, enabled: boolean, selectedTier: RapidOcrTier) {
  const ticket = epoch
  return ocrDownloadsStore.run(`rapid-install:${ticket}`, async () => {
    if (ticket !== epoch) return
    ocrDownloadsStore.setState((state) => ({
      ...state,
      rapidDownloadState: 'downloading',
      rapidDownloadError: '',
    }))
    try {
      const result = await port.rapidOcrInstall(selectedTier)
      if (ticket !== epoch) return
      if (result.success) {
        ocrDownloadsStore.setState((state) => ({
          ...state,
          rapidDownloadState: 'idle',
          rapidDownloadError: '',
        }))
        await pullRapidStatus(port, enabled)
      } else {
        ocrDownloadsStore.setState((state) => ({
          ...state,
          rapidDownloadState: 'failed',
          rapidDownloadError: result.message,
        }))
      }
    } catch (error) {
      if (ticket !== epoch) return
      ocrDownloadsStore.setState((state) => ({
        ...state,
        rapidDownloadState: 'failed',
        rapidDownloadError: errorText(error),
      }))
    }
  })
}

function downloadReplacePack(port: SettingsOcrPort, enabled: boolean, selectedTier: RapidOcrTier) {
  const ticket = epoch
  return ocrDownloadsStore.run(`replace-install:${ticket}`, async () => {
    if (ticket !== epoch) return
    replaceFlightActive = true
    syncProgressSubscription(port)
    ocrDownloadsStore.setState((state) => ({
      ...state,
      replaceDownload: reduceReplacePackProgress(state.replaceDownload, { type: 'start' }),
    }))
    let succeeded = false
    try {
      const result = await port.replaceTranslationPackInstall(selectedTier)
      if (ticket !== epoch) return
      if (result.success) {
        succeeded = true
        ocrDownloadsStore.setState((state) => ({
          ...state,
          replaceDownload: reduceReplacePackProgress(state.replaceDownload, { type: 'success' }),
        }))
      } else {
        ocrDownloadsStore.setState((state) => ({
          ...state,
          replaceDownload: reduceReplacePackProgress(state.replaceDownload, { type: 'failure', error: result.message }),
        }))
      }
    } catch (error) {
      if (ticket !== epoch) return
      ocrDownloadsStore.setState((state) => ({
        ...state,
        replaceDownload: reduceReplacePackProgress(state.replaceDownload, { type: 'failure', error: errorText(error) }),
      }))
    } finally {
      if (ticket === epoch) {
        replaceFlightActive = false
        if (!wantsProgressSubscription()) dropProgressSubscription()
      }
    }
    if (succeeded && ticket === epoch) {
      await Promise.all([
        pullReplaceStatus(port, enabled, selectedTier),
        pullRapidStatus(port, enabled),
      ])
    }
  })
}

/** Drops window-owned OCR flight state. Tests call this between cases; the app keeps the store for the window lifetime. */
export function resetSettingsOcrDownloadsForTests() {
  epoch += 1
  rapidRefreshSequence += 1
  replaceRefreshSequence += 1
  observedReplaceTier = 'standard'
  replaceRefreshEnabled = false
  progressConsumers = 0
  replaceFlightActive = false
  dropProgressSubscription()
  ocrDownloadsStore.setState({
    ...initialOcrDownloadsState,
    replaceDownload: { ...initialReplacePackProgressState },
  })
}

export function useSettingsOcrDownloads(
  enabled: boolean,
  tier: RapidOcrTier,
  port: SettingsOcrPort = api,
  options?: { observeReplacePack?: boolean },
) {
  const observeReplacePack = options?.observeReplacePack !== false
  if (observeReplacePack) {
    observedReplaceTier = tier
    replaceRefreshEnabled = enabled
  }
  const [snapshot] = useWindowStore(ocrDownloadsStore)

  const refreshRapid = useCallback(() => pullRapidStatus(port, enabled), [enabled, port])
  const refreshReplace = useCallback(
    (selectedTier: RapidOcrTier) => pullReplaceStatus(port, enabled, selectedTier),
    [enabled, port],
  )
  const downloadRapid = useCallback(
    (selectedTier: RapidOcrTier) => downloadRapidPack(port, enabled, selectedTier),
    [enabled, port],
  )
  const downloadReplace = useCallback(
    (selectedTier: RapidOcrTier) => downloadReplacePack(port, enabled, selectedTier),
    [enabled, port],
  )

  useEffect(() => { void refreshRapid() }, [refreshRapid])
  useEffect(() => {
    if (!observeReplacePack) return
    void refreshReplace(tier)
  }, [observeReplacePack, refreshReplace, tier])
  useEffect(() => {
    if (!observeReplacePack) return undefined
    retainProgressConsumer(port)
    return () => releaseProgressConsumer()
  }, [observeReplacePack, port])

  return {
    rapidStatus: snapshot.rapidStatus,
    rapidDownloadState: snapshot.rapidDownloadState,
    rapidDownloadError: snapshot.rapidDownloadError,
    replaceStatus: snapshot.replaceStatus,
    replaceDownload: snapshot.replaceDownload,
    refreshRapid,
    downloadRapid,
    refreshReplace,
    downloadReplace,
  }
}
