import { useEffect, useState } from 'react'
import { Button } from '../components/Button'
import { useT } from '../components/i18n'
import { chatApi } from './api'
import { formatToolResultPreview } from './toolResultPreview'
import type { DeferredToolResult } from './types'

// Share the request across disclosure remounts, including StrictMode's effect
// replay. Only the formatted preview lives as long as its history reference.
const previews = new WeakMap<DeferredToolResult, Promise<string>>()

function readPreview(reference: DeferredToolResult): Promise<string> {
  const cached = previews.get(reference)
  if (cached) return cached
  const pending = chatApi.getToolResult(reference).then(raw => formatToolResultPreview(raw)).catch(error => {
    previews.delete(reference)
    throw error
  })
  previews.set(reference, pending)
  return pending
}

export function HistoricalToolResult({ reference }: { reference: DeferredToolResult }) {
  const t = useT()
  const [attempt, setAttempt] = useState(0)
  const [result, setResult] = useState<{ text: string } | { error: string }>()
  useEffect(() => {
    let active = true
    void readPreview(reference).then(text => {
      if (active) setResult({ text })
    }, (error: unknown) => {
      if (active) setResult({ error: error instanceof Error ? error.message : String(error) })
    })
    return () => { active = false }
  }, [reference, attempt])

  if (!result) return <span role="status">{t.chatLoading}</span>
  if ('text' in result) return <>{result.text}</>
  return (
    <div>
      <span role="alert">{result.error}</span>
      <Button variant="ghost" size="sm" onClick={() => {
        setResult(undefined)
        setAttempt(n => n + 1)
      }}>{t.chatRetry}</Button>
    </div>
  )
}
