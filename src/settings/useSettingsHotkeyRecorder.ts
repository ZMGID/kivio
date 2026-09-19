import { useCallback, useEffect, useRef, useState } from 'react'
import type { Settings } from '../api/tauri'
import { buildHotkey } from './utils'

export type HotkeyScopeKey =
  | 'main'
  | 'chat'
  | 'closeChat'
  | 'screenshotTranslation'
  | 'screenshotTranslationText'
  | 'screenshotTranslationReplace'
  | 'screenshotAnnotate'
  | 'lens'

function withHotkey(settings: Settings, target: HotkeyScopeKey, hotkey: string): Settings {
  switch (target) {
    case 'main': return { ...settings, hotkey }
    case 'chat': return { ...settings, chatHotkey: hotkey }
    case 'closeChat': return { ...settings, closeChatHotkey: hotkey }
    case 'screenshotTranslation':
      return { ...settings, screenshotTranslation: { ...settings.screenshotTranslation, hotkey } }
    case 'screenshotTranslationText':
      return { ...settings, screenshotTranslation: { ...settings.screenshotTranslation, textHotkey: hotkey } }
    case 'screenshotTranslationReplace':
      return { ...settings, screenshotTranslation: { ...settings.screenshotTranslation, replaceHotkey: hotkey } }
    case 'screenshotAnnotate':
      return { ...settings, screenshotAnnotate: { ...settings.screenshotAnnotate, hotkey } }
    case 'lens': return { ...settings, lens: { ...settings.lens, hotkey } }
  }
}

/** Owns shortcut capture, cancellation, and the domain-specific settings update. */
export function useSettingsHotkeyRecorder(edit: (update: (settings: Settings) => Settings) => void) {
  const editRef = useRef(edit)
  editRef.current = edit
  const [target, setTarget] = useState<HotkeyScopeKey | null>(null)
  const toggle = useCallback((next: HotkeyScopeKey) => setTarget((current) => current === next ? null : next), [])

  useEffect(() => {
    if (!target) return
    const onKeyDown = (event: KeyboardEvent) => {
      event.preventDefault()
      event.stopPropagation()
      if (event.key === 'Escape') {
        setTarget(null)
        return
      }
      const hotkey = buildHotkey(event)
      if (!hotkey) return
      editRef.current((settings) => withHotkey(settings, target, hotkey))
      setTarget(null)
    }
    window.addEventListener('keydown', onKeyDown, true)
    return () => window.removeEventListener('keydown', onKeyDown, true)
  }, [target])

  return { target, toggle }
}
