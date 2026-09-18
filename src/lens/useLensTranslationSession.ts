import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type {
  LensReplaceGroup,
  LensReplaceRenderSlot,
  LensReplaceStreamPayload,
  LensTranslateStreamPayload,
} from '../api/tauri'

type TranslationUpdate = Pick<LensTranslateStreamPayload, 'imageId' | 'kind' | 'delta' | 'done' | 'error'>
type ReplacementUpdate = Pick<LensReplaceStreamPayload, 'imageId' | 'phase'> & Partial<
  Pick<LensReplaceStreamPayload, 'groups' | 'slots' | 'cleanedImage' | 'error' | 'warning'>
>

type LensTranslationSessionOptions = {
  onFinished?: () => void
}

/** Owns the transient output and timing resources for one Lens translation. */
export function useLensTranslationSession(options: LensTranslationSessionOptions = {}) {
  const [translateOriginal, setTranslateOriginal] = useState('')
  const [translateText, setTranslateText] = useState('')
  const [translateError, setTranslateError] = useState('')
  const [replaceGroups, setReplaceGroups] = useState<LensReplaceGroup[]>([])
  const [replaceSlots, setReplaceSlots] = useState<LensReplaceRenderSlot[]>([])
  const [replaceCleanedImage, setReplaceCleanedImage] = useState('')
  const [replacePhase, setReplacePhase] = useState<LensReplaceStreamPayload['phase'] | ''>('')
  const [replaceError, setReplaceError] = useState('')
  const [replaceWarning, setReplaceWarning] = useState('')
  const [durationMs, setDurationMs] = useState<number | null>(null)
  const [active, setActive] = useState(false)
  const [now, setNow] = useState(() => Date.now())
  const activeRef = useRef(false)
  const startedAtRef = useRef<number | null>(null)
  const onFinishedRef = useRef(options.onFinished)
  onFinishedRef.current = options.onFinished

  const reset = useCallback(() => {
    activeRef.current = false
    startedAtRef.current = null
    setActive(false)
    setTranslateOriginal('')
    setTranslateText('')
    setTranslateError('')
    setReplaceGroups([])
    setReplaceSlots([])
    setReplaceCleanedImage('')
    setReplacePhase('')
    setReplaceError('')
    setReplaceWarning('')
    setDurationMs(null)
  }, [])

  const begin = useCallback(() => {
    reset()
    const startedAt = Date.now()
    startedAtRef.current = startedAt
    activeRef.current = true
    setNow(startedAt)
    setActive(true)
  }, [reset])

  const complete = useCallback(() => {
    if (!activeRef.current) return false
    activeRef.current = false
    setActive(false)
    const startedAt = startedAtRef.current
    startedAtRef.current = null
    setDurationMs(startedAt === null ? 0 : Math.max(0, Date.now() - startedAt))
    onFinishedRef.current?.()
    return true
  }, [])

  const beginTranslation = begin
  const beginReplacement = begin

  const applyTranslationPayload = useCallback((payload: TranslationUpdate) => {
    if (!activeRef.current) return false
    if (payload.done) {
      if (payload.error) setTranslateError(payload.error)
      return complete()
    }
    if (!payload.delta) return false
    if (payload.kind === 'original') setTranslateOriginal(previous => previous + payload.delta)
    if (payload.kind === 'translated') setTranslateText(previous => previous + payload.delta)
    return false
  }, [complete])

  const failTranslation = useCallback((error: string) => {
    if (!activeRef.current) return false
    setTranslateError(error)
    return complete()
  }, [complete])

  const applyReplacementPayload = useCallback((payload: ReplacementUpdate) => {
    if (!activeRef.current) return false
    if (payload.error) setReplaceError(payload.error)
    if (payload.warning) setReplaceWarning(payload.warning)
    if (payload.groups?.length) setReplaceGroups(payload.groups)
    if (payload.slots?.length) setReplaceSlots(payload.slots)
    if (payload.cleanedImage) setReplaceCleanedImage(payload.cleanedImage)
    setReplacePhase(payload.phase)
    if (payload.phase === 'done' || payload.phase === 'error') return complete()
    return false
  }, [complete])

  const failReplacement = useCallback((error: string) => {
    if (!activeRef.current) return false
    setReplaceError(error)
    setReplacePhase('error')
    return complete()
  }, [complete])

  useEffect(() => {
    if (!active) return
    const interval = window.setInterval(() => setNow(Date.now()), 1000)
    return () => window.clearInterval(interval)
  }, [active])

  const elapsedMs = useMemo(() => {
    if (!active || startedAtRef.current === null) return durationMs
    return Math.max(0, now - startedAtRef.current)
  }, [active, durationMs, now])

  return {
    applyReplacementPayload,
    applyTranslationPayload,
    beginReplacement,
    beginTranslation,
    durationMs,
    elapsedMs,
    failReplacement,
    failTranslation,
    replaceCleanedImage,
    replaceError,
    replaceGroups,
    replacePhase,
    replaceSlots,
    replaceWarning,
    reset,
    translateError,
    translateOriginal,
    translateText,
  }
}
