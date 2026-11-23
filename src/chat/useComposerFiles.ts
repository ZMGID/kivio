import { useEffect, useState } from 'react'
import { dockApi } from './dock/api'

/** Search the same effective directory and visible paths as the conversation's file tree. */
export function useComposerFiles(query: string | null, conversationId?: string | null, projectId?: string) {
  const [result, setResult] = useState<{
    query: string
    conversationId?: string | null
    projectId?: string
    paths: string[]
    error?: string
  } | null>(null)

  useEffect(() => {
    if (query === null) {
      setResult(null)
      return
    }
    let cancelled = false
    const search = async () => {
      try {
        const cwd = await dockApi.resolveCwd(conversationId, projectId)
        if (cancelled) return
        if (!cwd) {
          setResult({ query, conversationId, projectId, paths: [] })
          return
        }
        const { entries } = query
          ? await dockApi.fsSearch(cwd, query, 40)
          : await dockApi.fsList(cwd, '', 100)
        if (cancelled) return
        const paths = entries.filter(entry => entry.kind === 'file').slice(0, 20).map(entry => entry.path)
        setResult({ query, conversationId, projectId, paths })
      } catch (error) {
        if (!cancelled) {
          const message = error instanceof Error ? error.message : String(error)
          setResult({ query, conversationId, projectId, paths: [], error: message })
        }
      }
    }
    const timer = window.setTimeout(() => void search(), query ? 180 : 0)
    return () => {
      cancelled = true
      window.clearTimeout(timer)
    }
  }, [query, conversationId, projectId])

  return result && result.query === query && result.conversationId === conversationId && result.projectId === projectId
    ? result : null
}
