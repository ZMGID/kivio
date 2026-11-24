import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'
import type { ChatRunOutcome } from '../chatRunSettlement'
import { isChatWindowViewing } from '../notificationView'

export type ConversationRunMark = 'unread' | 'failed'

const storageKey = 'kivio-chat-conversation-run-marks'

function loadRunMarks(): ReadonlyMap<string, ConversationRunMark> {
  const marks = new Map<string, ConversationRunMark>()
  try {
    const stored: unknown = JSON.parse(localStorage.getItem(storageKey) ?? '{}')
    if (stored && typeof stored === 'object') {
      for (const [id, mark] of Object.entries(stored)) {
        if (mark === 'unread' || mark === 'failed') marks.set(id, mark)
      }
    }
  } catch {
    // Storage may be unavailable in the webview.
  }
  return marks
}

export function useConversationRunMarks(viewedConversationId: string | undefined) {
  const [marks, setMarks] = useState(loadRunMarks)
  const viewedId = useRef(viewedConversationId)
  const markRead = useCallback(() => {
    const id = viewedId.current
    if (!id || !isChatWindowViewing()) return
    setMarks((current) => {
      if (current.get(id) !== 'unread') return current
      const next = new Map(current)
      next.delete(id)
      return next
    })
  }, [])

  useLayoutEffect(() => {
    viewedId.current = viewedConversationId
    markRead()
  }, [viewedConversationId, markRead])

  useEffect(() => {
    window.addEventListener('focus', markRead)
    document.addEventListener('visibilitychange', markRead)
    return () => {
      window.removeEventListener('focus', markRead)
      document.removeEventListener('visibilitychange', markRead)
    }
  }, [markRead])

  useEffect(() => {
    try {
      localStorage.setItem(storageKey, JSON.stringify(Object.fromEntries(marks)))
    } catch {
      // Keep the marks usable for this session.
    }
  }, [marks])

  const recordOutcome = useCallback((id: string, outcome: ChatRunOutcome) => {
    const mark = outcome === 'error' ? 'failed'
      : outcome === 'done' && !(viewedId.current === id && isChatWindowViewing()) ? 'unread'
        : undefined
    setMarks((current) => {
      if (current.get(id) === mark) return current
      const next = new Map(current)
      if (mark) next.set(id, mark)
      else next.delete(id)
      return next
    })
  }, [])

  const clearMark = useCallback((id: string) => recordOutcome(id, 'cancelled'), [recordOutcome])

  return { marks, recordOutcome, clearMark }
}
