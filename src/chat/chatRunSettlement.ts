import type { Conversation } from './types'

export interface ChatRunTerminal {
  conversationId: string
  runId?: string | null
  reason?: string
  /** Execution generation captured before asynchronous invoke settlement. */
  turnEpoch?: number
}

interface SettlementPorts {
  completeWithConversation: (conversationId: string, conversation: Conversation) => void
  completeTerminal: (terminal: ChatRunTerminal) => Promise<void>
  abandonPreview: (conversationId: string) => void
  settleQueue: (conversationId: string, conversation?: Conversation | null) => void | Promise<unknown>
}

/** Owns the ordering between a terminal stream frame and the authoritative
 * send/regenerate/reply invoke result. UI navigation never clears this owner. */
export function createChatRunSettlement() {
  let sequence = 0
  const active = new Map<string, { token: number; runIds: Set<string> }>()
  const deferred = new Map<string, { token: number; terminal: ChatRunTerminal }>()
  const retired = new Map<string, Set<string>>()

  const retire = (conversationId: string, runIds: Iterable<string>) => {
    const history = retired.get(conversationId) ?? new Set<string>()
    for (const runId of runIds) history.add(runId)
    // Keep enough IDs to cover late protocol delivery without unbounded growth.
    while (history.size > 32) history.delete(history.values().next().value!)
    retired.set(conversationId, history)
  }

  const acceptRunEvent = (
    conversationId: string,
    runId: string | null | undefined,
    started = false,
  ): boolean => {
    if (!runId) return true
    if (retired.get(conversationId)?.has(runId)) return false
    if (started) active.get(conversationId)?.runIds.add(runId)
    return true
  }

  return {
    beginInvoke(conversationId: string): number {
      const token = ++sequence
      active.set(conversationId, { token, runIds: new Set() })
      deferred.delete(conversationId)
      return token
    },
    acceptRunEvent,
    deferTerminal(terminal: ChatRunTerminal): boolean {
      if (!acceptRunEvent(terminal.conversationId, terminal.runId)) return false
      const invocation = active.get(terminal.conversationId)
      if (!invocation) return false
      if (terminal.runId) invocation.runIds.add(terminal.runId)
      deferred.set(terminal.conversationId, { token: invocation.token, terminal })
      return true
    },
    clearConversation(conversationId: string): void {
      active.delete(conversationId)
      deferred.delete(conversationId)
      retired.delete(conversationId)
    },
    async settleInvoke(
      conversationId: string,
      token: number,
      persistedConversation: Conversation | null,
      ports: SettlementPorts,
    ): Promise<void> {
      const invocation = active.get(conversationId)
      if (!invocation || invocation.token !== token) return
      active.delete(conversationId)
      retire(conversationId, invocation.runIds)
      const pending = deferred.get(conversationId)
      deferred.delete(conversationId)
      const terminal = pending?.token === token ? pending.terminal : null
      if (persistedConversation) {
        ports.completeWithConversation(conversationId, persistedConversation)
        // A queued send may start immediately. Do not await its entire next run
        // inside the previous run's finally block.
        void ports.settleQueue(conversationId, persistedConversation)
        return
      }
      void ports.settleQueue(conversationId)
      if (terminal) await ports.completeTerminal(terminal)
      else ports.abandonPreview(conversationId)
    },
  }
}
