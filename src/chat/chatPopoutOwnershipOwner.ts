import { chatApi } from './api'

type Port = Pick<typeof chatApi,
  'listConversationPopouts' | 'openConversationPopout' | 'closeConversationPopout'>

type OwnershipChange = {
  previous: ReadonlySet<string>
  next: ReadonlySet<string>
  entered: ReadonlySet<string>
  exited: ReadonlySet<string>
}

function difference(previous: ReadonlySet<string>, next: ReadonlySet<string>): OwnershipChange {
  return {
    previous: new Set(previous), next: new Set(next),
    entered: new Set([...next].filter((id) => !previous.has(id))),
    exited: new Set([...previous].filter((id) => !next.has(id))),
  }
}

/** Owns popout membership. Runs outlive their display window. */
export function createChatPopoutOwnershipOwner(port: Port = chatApi) {
  let ids = new Set<string>()
  let listed = false
  let epoch = 0
  let pending: { epoch: number; promise: Promise<ReadonlySet<string>> } | null = null
  const listeners = new Set<() => void>()
  const setIds = (nextIds: Iterable<string>): OwnershipChange => {
    const previous = ids
    const next = new Set(nextIds)
    ids = next
    listed = true
    epoch += 1 // Invalidate a slower list request or open/close response.
    listeners.forEach((listener) => listener())
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
    membership: () => ids as ReadonlySet<string>,
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
  }
}
