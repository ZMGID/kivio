import { useSyncExternalStore } from 'react'

const listeners = new Set<() => void>()
let observer: MutationObserver | null = null

function readDark(): boolean {
  return typeof document !== 'undefined' && document.documentElement.classList.contains('dark')
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener)
  if (!observer && typeof document !== 'undefined') {
    observer = new MutationObserver(() => {
      for (const notify of listeners) notify()
    })
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ['class'] })
  }
  return () => {
    listeners.delete(listener)
    if (listeners.size === 0) {
      observer?.disconnect()
      observer = null
    }
  }
}

/** For libraries whose light/dark mode is a prop rather than a CSS variable. */
export function useDocumentDark(): boolean {
  return useSyncExternalStore(subscribe, readDark, () => false)
}
