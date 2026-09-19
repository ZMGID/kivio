import { useCallback, useRef } from 'react'
import { api } from './tauri'

export function useWindowInteractionFocus() {
  const lastFocusRequestAtRef = useRef(0)

  return useCallback(() => {
    if (document.hasFocus()) return

    const now = Date.now()
    if (now - lastFocusRequestAtRef.current < 300) return
    lastFocusRequestAtRef.current = now

    void api.focusWindow().catch((err) => {
      console.error('[window-focus] failed:', err)
    })
  }, [])
}
