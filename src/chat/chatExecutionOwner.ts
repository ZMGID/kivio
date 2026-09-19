import { beginGroup, endGroup, type GroupArmSeed } from './groupStreamingStore'
import { createChatRunSettlement, type ChatRunTerminal } from './chatRunSettlement'
import { createChatSendReservations } from './chatSendReservations'
import { createOptimisticUserPresentation } from './optimisticUserPresentation'
import { chatApi } from './api'
import type { ChatMessage, Conversation, PendingAttachment } from './types'

type Reservation = NonNullable<ReturnType<ReturnType<typeof createChatSendReservations>['claim']>>
type SettlementPorts = Parameters<ReturnType<typeof createChatRunSettlement>['settleInvoke']>[3]
type SingleRunPort = Pick<typeof chatApi, 'sendMessage'>
declare const sendClaimBrand: unique symbol
export type SendClaim = { readonly [sendClaimBrand]: true }

export interface ExecutionLease {
  readonly conversationId: string
  readonly token: number
}

export type PreparedSingleRunOutcome =
  | { kind: 'persisted'; conversation: Conversation }
  | { kind: 'persisted_error'; conversation: Conversation; error: Error }
  | { kind: 'not_committed'; error: Error }

type PreparedSingleRunIntent = {
  lease: ExecutionLease
  content: string
  attachments: PendingAttachment[]
  attachmentSkillId: string | null
  planMessageId?: string
}

type SingleRunEffects = SettlementPorts & {
  onOutcome: (outcome: PreparedSingleRunOutcome) => void | Promise<void>
}

type BeginIntent = {
  conversationId: string
  kind: 'send' | 'regenerate' | 'replyWithModel'
  startedAt: number
  optimistic?: { content: string; attachments: PendingAttachment[]; stored: ChatMessage[] }
  group?: { groupId: string; arms: GroupArmSeed[] }
  claim?: SendClaim
}

type ExecutionEvent =
  | { kind: 'runEvent'; conversationId: string; runId: string | null | undefined; started?: boolean }
  | { kind: 'deferTerminal'; terminal: ChatRunTerminal }
  | { kind: 'externalStarted' | 'externalEnded' | 'drop'; conversationId: string }

type GroupStore = { begin: typeof beginGroup; end: typeof endGroup }

/** Owns the identity and lifetime of a Chat execution. The high-frequency
 * stream/group content stores remain presentation adapters; neither they nor
 * the current route may decide whether an invoke is still active. */
export function createChatExecutionOwner(
  groups: GroupStore = { begin: beginGroup, end: endGroup },
  singleRunPort: SingleRunPort = chatApi,
) {
  const settlement = createChatRunSettlement()
  const reservations = createChatSendReservations()
  const optimistic = createOptimisticUserPresentation()
  const active = new Map<string, {
    lease: ExecutionLease
    optimisticToken: number | null
    groupId: string | null
    runIds: Set<string>
    startedAt: number
    claim?: SendClaim
  }>()
  const claims = new Map<SendClaim, Reservation>()
  const external = new Set<string>()
  const listeners = new Set<() => void>()
  let revision = 0
  const publish = () => {
    revision += 1
    listeners.forEach((listener) => listener())
  }
  const abandonSend = (claim: SendClaim) => {
    const reservation = claims.get(claim)
    if (!reservation) return
    claims.delete(claim)
    reservation.release()
  }

  const snapshot = (conversationId: string) => {
    const invocation = active.get(conversationId)
    return {
      conversationId,
      inFlight: Boolean(invocation) || external.has(conversationId),
      startedAt: invocation?.startedAt ?? null,
      groupId: invocation?.groupId ?? null,
      runIds: invocation ? [...invocation.runIds] : [],
      revision,
    }
  }

  return {
    subscribe: (listener: () => void) => {
      listeners.add(listener)
      return () => { listeners.delete(listener) }
    },
    getRevision: () => revision,
    snapshot,
    activeConversationIds: () => [...new Set([...active.keys(), ...external])],
    overlayMessages: optimistic.overlay,
    claimSend: (conversationId: string | null): SendClaim | null => {
      const reservation = reservations.claim(conversationId)
      if (!reservation) return null
      const claim = {} as SendClaim
      claims.set(claim, reservation)
      return claim
    },
    bindSend: (claim: SendClaim, conversationId: string) => claims.get(claim)?.bind(conversationId) ?? false,
    abandonSend,
    begin(intent: BeginIntent): ExecutionLease | null {
      const id = intent.conversationId
      if (active.has(id) || external.has(id)) return null
      if (intent.kind !== 'send' && intent.optimistic) {
        throw new Error('Only a send may own an optimistic user message')
      }
      if (intent.kind === 'regenerate' && intent.group) {
        throw new Error('Regeneration cannot begin a multi-answer group')
      }
      const token = settlement.beginInvoke(id)
      const lease = { conversationId: id, token }
      const optimisticToken = intent.optimistic
        ? optimistic.begin(
          id, intent.optimistic.content, intent.optimistic.attachments,
          intent.startedAt, intent.optimistic.stored,
        ).token
        : null
      active.set(id, {
        lease, optimisticToken, groupId: intent.group?.groupId ?? null,
        runIds: new Set(), startedAt: intent.startedAt, claim: intent.claim,
      })
      try {
        if (intent.group) groups.begin(id, intent.group.groupId, intent.group.arms)
      } catch (error) {
        active.delete(id)
        if (optimisticToken != null) optimistic.settle(id, optimisticToken)
        if (intent.claim) abandonSend(intent.claim)
        settlement.clearConversation(id)
        throw error
      }
      publish()
      return lease
    },
    observe(event: ExecutionEvent): boolean {
      if (event.kind === 'deferTerminal') return settlement.deferTerminal(event.terminal)
      const id = event.conversationId
      if (event.kind === 'runEvent') {
        if (!settlement.acceptRunEvent(id, event.runId, event.started)) return false
        if (event.started && event.runId) {
          const invocation = active.get(id)
          if (invocation) invocation.runIds.add(event.runId)
          else external.add(id)
          publish()
        }
        return true
      }
      if (event.kind === 'externalStarted' && !active.has(id)) external.add(id)
      if (event.kind === 'externalEnded') external.delete(id)
      if (event.kind === 'drop') {
        const invocation = active.get(id)
        if (invocation?.claim) abandonSend(invocation.claim)
        if (invocation?.optimisticToken != null) optimistic.settle(id, invocation.optimisticToken)
        if (invocation?.groupId) groups.end(id)
        active.delete(id)
        external.delete(id)
        settlement.clearConversation(id)
        optimistic.clear(id)
      }
      publish()
      return true
    },
    async finish(lease: ExecutionLease, persisted: Conversation | null, ports: SettlementPorts): Promise<void> {
      const id = lease.conversationId
      const invocation = active.get(id)
      if (!invocation || invocation.lease.token !== lease.token) return
      active.delete(id)
      if (invocation.optimisticToken != null) optimistic.settle(id, invocation.optimisticToken)
      if (invocation.groupId) groups.end(id)
      if (invocation.claim) abandonSend(invocation.claim)
      publish()
      await settlement.settleInvoke(id, lease.token, persisted, ports)
    },
    /** Prepared, single-model only. The caller owns conversation preparation,
     * canonical fan-out selection and UI projection; this method owns invoke
     * classification and the release-before-queue settlement order. */
    async submitPreparedSingleRun(
      intent: PreparedSingleRunIntent,
      effects: SingleRunEffects,
    ): Promise<PreparedSingleRunOutcome> {
      if (!active.get(intent.lease.conversationId)
        || active.get(intent.lease.conversationId)?.lease.token !== intent.lease.token) {
        return { kind: 'not_committed', error: new Error('该对话没有活跃发送') }
      }
      let outcome: PreparedSingleRunOutcome
      let persistedForSettlement: Conversation | null = null
      try {
        const conversation = await singleRunPort.sendMessage(
          intent.lease.conversationId,
          intent.content,
          intent.attachments,
          intent.attachmentSkillId,
          intent.planMessageId,
        )
        persistedForSettlement = conversation
        outcome = { kind: 'persisted', conversation }
      } catch (value) {
        const error = value instanceof Error
          ? value
          : new Error(typeof value === 'string'
            ? value
            : typeof (value as { message?: unknown } | null)?.message === 'string'
              ? (value as { message: string }).message
              : '发送失败')
        const kept = (value as { conversation?: Conversation } | null)?.conversation
        outcome = kept
          ? { kind: 'persisted_error', conversation: kept, error }
          : { kind: 'not_committed', error }
      }
      try {
        await effects.onOutcome(outcome)
      } catch (error) {
        // The backend commit is authoritative even if a view projection fails.
        // Keep the original three-state result so the composer cannot restore
        // a message that was already persisted; settlement still applies it.
        console.error('Failed to present single Chat run outcome:', error)
      } finally {
        await this.finish(intent.lease, persistedForSettlement, effects)
      }
      return outcome
    },
  }
}
