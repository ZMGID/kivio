import { useCallback, useEffect, useRef, useState } from 'react'
import type { Lang } from '../components/i18n'

export interface SettingsOnboardingPort {
  flush(): Promise<boolean>
  writePending(): Promise<unknown>
  committed(): void
  navigate(): void
}

/** Serializes the settings-to-onboarding transition without discarding editor drafts. */
export function useSettingsOnboardingController(port: SettingsOnboardingPort, lang: Lang) {
  const portRef = useRef(port)
  portRef.current = port
  const langRef = useRef(lang)
  langRef.current = lang
  const live = useRef(true)
  const inFlight = useRef(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  const restart = useCallback(async (): Promise<void> => {
    if (inFlight.current) return
    inFlight.current = true
    setBusy(true)
    setError('')
    try {
      const unsaved = () => new Error(langRef.current === 'zh'
        ? '仍有未保存的设置，请处理保存错误后重试。'
        : 'Unsaved settings remain. Resolve the save error and retry.')
      if (!await portRef.current.flush()) throw unsaved()
      await portRef.current.writePending()
      if (!live.current) return
      portRef.current.committed()
      // Settings may have been edited while the narrow write was in flight.
      // The cache subscription rebases those edits; flush them before leaving.
      if (!await portRef.current.flush()) throw unsaved()
      if (live.current) portRef.current.navigate()
    } catch (failure) {
      if (live.current) setError(failure instanceof Error ? failure.message : String(failure))
    } finally {
      inFlight.current = false
      if (live.current) setBusy(false)
    }
  }, [])

  useEffect(() => {
    live.current = true
    return () => { live.current = false }
  }, [])

  return { restart, busy, error }
}
