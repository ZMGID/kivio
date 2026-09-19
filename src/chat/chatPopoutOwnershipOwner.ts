import { chatApi } from './api'

type Port = Pick<typeof chatApi,
  'listConversationPopouts' | 'openConversationPopout' | 'closeConversationPopout'>

type OwnershipChange = {
  previous: ReadonlySet<string>
  next: ReadonlySet<string>
  entered: ReadonlySet<string>
  exited: ReadonlySet<string>
}

type RunPayload = {
  conversationId: string
  runId?: string | null
  type: string
}

type RunProjection = {
  suppressMainProjection: boolean
  running: boolean
  effect: 'started' | 'finished' | 'none'
}

const terminal = (type: string) => type === 'run_completed'
  || type === 'run_failed' || type === 'run_cancelled'

function difference(previous: ReadonlySet<string>, next: ReadonlySet<string>): OwnershipChange {
  return {
    previous: new Set(previous), next: new Set(next),
    entered: new Set([...next].filter((id) => !previous.has(id))),
    exited: new Set([...previous].filter((id) => !next.has(id))),
  }
}

/** Owns popout membership and only the run IDs observed while popped out.
 * It never commits a route or stops execution when membership changes. */
export function createChatPopoutOwnershipOwner(port: Port = chatApi) {
  let ids = new Set<string>()
  let listed = false
  let epoch = 0
  let revision = 0
  let pending: { epoch: number; promise: Promise<ReadonlySet<string>> } | null = null
  const runs = new Map<string, Set<string>>()
  const listeners = new Set<() => void>()
  const publish = () => {
    revision += 1
    listeners.forEach((listener) => listener())
  }
  const setIds = (nextIds: Iterable<string>): OwnershipChange => {
    const previous = ids
    const next = new Set(nextIds)
    ids = next
    listed = true
    epoch += 1 // Invalidate a slower list request or open/close response.
    publish()
    return difference(previous, next)
  }
  const fetchIds = (requestEpoch: number): Promise<ReadonlySet<string>> => {
    if (pending?.epoch === requestEpoch) return pending.promise
    const request = port.listConversationPopouts().then((listedIds) => {
      if (epoch === requestEpoch) setIds(listedIds)
      return new Set(ids)
    }).finally(() => {
      if (pending?.promise === request) pending = null
    })
    pending = { epoch: requestEpoch, promise: request }
    return request
  }
  const refresh = async (): Promise<OwnershipChange> => {
    const previous = new Set(ids)
    listed = false
    epoch += 1
    await fetchIds(epoch)
    return difference(previous, ids)
  }

  return {
    subscribe(listener: () => void) {
      listeners.add(listener)
      return () => { listeners.delete(listener) }
    },
    getRevision: () => revision,
    membership: () => ids as ReadonlySet<string>,
    snapshot: () => ({
      ids: new Set(ids) as ReadonlySet<string>,
      runningConversationIds: new Set(runs.keys()) as ReadonlySet<string>,
      revision,
    }),
    owns: (conversationId: string) => ids.has(conversationId),
    list: () => listed ? Promise.resolve(new Set(ids) as ReadonlySet<string>) : fetchIds(epoch),
    refresh,
    changed: setIds,
    async open(conversationId: string): Promise<OwnershipChange> {
      const startedAtEpoch = epoch
      await port.openConversationPopout(conversationId)
      if (epoch !== startedAtEpoch) return refresh()
      return setIds([...ids, conversationId])
    },
    async close(conversationId: string): Promise<OwnershipChange> {
      const startedAtEpoch = epoch
      await port.closeConversationPopout(conversationId)
      if (epoch !== startedAtEpoch) return refresh()
      return setIds([...ids].filter((id) => id !== conversationId))
    },
    observeRun(payload: RunPayload): RunProjection {
      const id = payload.conversationId
      const owned = ids.has(id)
      const active = runs.get(id)
      const wasTracked = Boolean(payload.runId && active?.has(payload.runId))
      let effect: RunProjection['effect'] = 'none'
      if (owned && payload.type === 'run_started' && payload.runId) {
        const next = active ?? new Set<string>()
        const wasRunning = next.size > 0
        next.add(payload.runId)
        runs.set(id, next)
        if (!wasRunning) effect = 'started'
      } else if (active && terminal(payload.type)) {
        if (payload.runId) active.delete(payload.runId)
        else if (active.size === 1) active.clear()
        if (active.size === 0) {
          runs.delete(id)
          effect = 'finished'
        }
      }
      if (effect !== 'none') publish()
      return {
        suppressMainProjection: owned || wasTracked,
        running: runs.has(id),
        effect,
      }
    },
  }
}
