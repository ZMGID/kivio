import { useCallback, useEffect, useRef, useState } from 'react'
import type { ReadingOpenTarget } from '../api/reading'
import { readingOperationsApi, type ReadingPdfKind, type ReadingSession } from '../api/readingOperations'

/** One subscription for the document's job and displayed PDF version. Backend
 * snapshots decide which version to show, including after navigation/restart. */
export function useReadingSession(id: string, initialView?: ReadingOpenTarget['view']) {
  const [session, setSession] = useState<ReadingSession | null>(null)
  const [error, setError] = useState('')
  const requestedView = useRef(initialView)
  const initialRequest = useRef<Promise<ReadingSession> | null>(null)
  const revision = useRef(0), alive = useRef(true)
  const running = session?.translation?.phase === 'running'
  useEffect(() => { alive.current = true; ++revision.current; return () => { alive.current = false } }, [id])
  useEffect(() => {
    let live = true
    let timer: ReturnType<typeof setTimeout> | undefined
    const refresh = async () => {
      const current = revision.current
      try {
        const target = requestedView.current
        // Reuse admission across effect restarts; a status read must not race
        // the explicit child version requested by library navigation.
        const value = target ? await (initialRequest.current ??= readingOperationsApi.selectView(id, target.kind, target.runId)) : await readingOperationsApi.status(id)
        if (live && current === revision.current) {
          requestedView.current = undefined; initialRequest.current = null
          setSession(value); setError('')
        }
      } catch (error) { if (live && current === revision.current) setError(String(error)) }
      if (live && running) timer = setTimeout(() => void refresh(), 900)
    }
    void refresh()
    return () => { live = false; clearTimeout(timer) }
  }, [id, running])
  const commit = useCallback(async (action: () => Promise<ReadingSession>) => {
    requestedView.current = undefined; initialRequest.current = null
    const current = ++revision.current
    const value = await action()
    if (alive.current && current === revision.current) { setSession(value); setError(''); return value }
  }, [])
  const selectView = useCallback((kind: ReadingPdfKind, runId?: string) =>
    commit(() => readingOperationsApi.selectView(id, kind, runId)), [id, commit])
  const translate = useCallback((providerId: string, model: string, source: string, target: string, page: number | null) =>
    commit(() => readingOperationsApi.translate(id, providerId, model, source, target, page)), [id, commit])
  const cancel = useCallback(async () => { await readingOperationsApi.cancel(id) }, [id])
  return { session, error, selectView, translate, cancel }
}
