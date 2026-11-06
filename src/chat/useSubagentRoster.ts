import { useEffect, useState } from 'react'
import { subagentsApi, type SubagentRoster } from '../api/subagents'

export function useSubagentRoster(conversationId: string | null) {
  const [snapshot, setSnapshot] = useState<{ id: string; roster: SubagentRoster }>()
  const [failure, setFailure] = useState<{ id: string; error: string }>()
  const [revision, setRevision] = useState(0)
  useEffect(() => {
    if (!conversationId) return
    const id = conversationId
    let active = true
    let timer: ReturnType<typeof setTimeout> | undefined
    async function poll() {
      let delay = 2500
      try {
        const roster = await subagentsApi.snapshot(id)
        if (!active) return
        setSnapshot({ id, roster })
        setFailure(undefined)
        if (roster.runningCount) delay = 500
      } catch (error) {
        if (active) setFailure({ id, error: error instanceof Error ? error.message : String(error) })
      }
      if (active) timer = setTimeout(() => void poll(), delay)
    }
    void poll()
    return () => { active = false; clearTimeout(timer) }
  }, [conversationId, revision])
  return {
    roster: snapshot?.id === conversationId ? snapshot.roster : null,
    error: failure?.id === conversationId ? failure.error : '',
    refresh: () => setRevision(value => value + 1),
  }
}
