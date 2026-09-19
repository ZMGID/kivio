import { beginGroup, endGroup, type GroupArmSeed } from './groupStreamingStore'
import { createChatRunSettlement, type ChatRunTerminal } from './chatRunSettlement'
import { createChatSendReservations } from './chatSendReservations'
import { createOptimisticUserPresentation } from './optimisticUserPresentation'
import { chatApi } from './api'
import type { ChatMessage, Conversation, PendingAttachment } from './types'

type Reservation = NonNullable<ReturnType<ReturnType<typeof createChatSendReservations>['claim']>>
type SettlementPorts = Parameters<ReturnType<typeof createChatRunSettlement>['settleInvoke']>[3]
type SendPort = Pick<typeof chatApi, 'sendMessage'>
declare const sendClaimBrand: unique symbol
export type SendClaim = { readonly [sendClaimBrand]: true }

export interface ExecutionLease {
  readonly conversationId: string
  readonly token: number
}

declare const cancellationPermitBrand: unique symbol
export type CancellationPermit = {
  readonly conversationId: string
  readonly [cancellationPermitBrand]: true
}

declare const externalTerminalPermitBrand: unique symbol
export type ExternalTerminalPermit = {
  readonly conversationId: string
  readonly token: number
  readonly [externalTerminalPermitBrand]: true
}

export type TerminalDisposition =
  | { kind: 'ignored' | 'pending' | 'deferred' }
  | { kind: 'ready'; permit: ExternalTerminalPermit }

type StreamPayloadIdentity = {
  conversationId: string
  runId?: string | null
  type?: string
}

export type PreparedRunOutcome =
  | { kind: 'persisted'; conversation: Conversation }
  | { kind: 'persisted_error'; conversation: Conversation; error: Error }
  | { kind: 'not_committed'; error: Error }

type PreparedRunIntent = {
  lease: ExecutionLease
  content: string
  attachments: PendingAttachment[]
  attachmentSkillId: string | null
  planMessageId?: string
}

type PreparedRunEffects = SettlementPorts & {
  onOutcome: (outcome: PreparedRunOutcome) => void | Promise<void>
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
  | { kind: 'runEvent'; conversationId: string; runId: string | null | undefined; started?: boolean; groupId?: string; groupSize?: number }
  | { kind: 'deferTerminal'; terminal: ChatRunTerminal }
  | { kind: 'externalStarted' | 'externalEnded' | 'drop'; conversationId: string }

type GroupStore = { begin: typeof beginGroup; end: typeof endGroup }

/** Owns the identity and lifetime of a Chat execution. The high-frequency
 * stream/group content stores remain presentation adapters; neither they nor
 * the current route may decide whether an invoke is still active. */
export function createChatExecutionOwner(
  groups: GroupStore = { begin: beginGroup, end: endGroup },
  sendPort: SendPort = chatApi,
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
  const externalRuns = new Map<string, {
    token: number
    groupId: string | null
    expectedArms: number
    runIds: Set<string>
    terminalIds: Set<string>
    ready: boolean
    groupEnded: boolean
  }>()
  const retiredExternalRunIds = new Map<string, Set<string>>()
  let externalSequence = 0
  const cancellations = new Map<string, { permit: CancellationPermit; runId: string | null; groupId: string | null }>()
  const turnEpochs = new Map<string, number>()
  const advanceTurn = (conversationId: string) => {
    turnEpochs.set(conversationId, (turnEpochs.get(conversationId) ?? 0) + 1)
  }
  const listeners = new Set<() => void>()
  let revision = 0
  const publish = () => {
    revision += 1
    listeners.forEach((listener) => listener())
  }
  const retireExternal = (conversationId: string) => {
    const run = externalRuns.get(conversationId)
    if (!run) return
    const retired = retiredExternalRunIds.get(conversationId) ?? new Set<string>()
    for (const runId of run.runIds) retired.add(runId)
    while (retired.size > 32) retired.delete(retired.values().next().value!)
    retiredExternalRunIds.set(conversationId, retired)
    externalRuns.delete(conversationId)
  }
  const isExternalTerminalCurrent = (permit: ExternalTerminalPermit) => {
    const run = externalRuns.get(permit.conversationId)
    return Boolean(run?.ready && run.token === permit.token)
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
    turnEpoch: (conversationId: string) => turnEpochs.get(conversationId) ?? 0,
    snapshot,
    activeConversationIds: () => [...new Set([...active.keys(), ...external])],
    overlayMessages: optimistic.overlay,
    /** Grants one cancellation attempt for the current execution. The permit
     * remains the identity of its content fence until failure or run turnover. */
    requestCancellation(conversationId: string, runId: string | null = null): CancellationPermit | null {
      if ((!active.has(conversationId) && !external.has(conversationId))
        || cancellations.has(conversationId)) return null
      const permit = { conversationId } as CancellationPermit
      const activeRunIds = active.get(conversationId)?.runIds
      const externalRunIds = externalRuns.get(conversationId)?.runIds
      const observedRunId = activeRunIds?.size
        ? [...activeRunIds][activeRunIds.size - 1]
        : externalRunIds?.size ? [...externalRunIds][externalRunIds.size - 1] : undefined
      cancellations.set(conversationId, {
        permit,
        runId: runId ?? observedRunId ?? null,
        groupId: active.get(conversationId)?.groupId ?? externalRuns.get(conversationId)?.groupId ?? null,
      })
      publish()
      return permit
    },
    /** A failed backend request reopens cancellation and content delivery.
     * A successful request keeps the fence until terminal settlement/new run. */
    completeCancellation(permit: CancellationPermit, succeeded: boolean): boolean {
      const current = cancellations.get(permit.conversationId)
      if (!current || current.permit !== permit) return false
      if (!succeeded) {
        cancellations.delete(permit.conversationId)
        publish()
      }
      return true
    },
    /** Terminal events are authoritative even after local cancellation. */
    allowsStreamPayload(payload: StreamPayloadIdentity): boolean {
      if (payload.type === 'run_cancelled' || payload.type === 'run_completed' || payload.type === 'run_failed') return true
      const cancelled = cancellations.get(payload.conversationId)
      if (!cancelled) return true
      // cancelStream is conversation-level: every arm of this group stops,
      // even when only one arm's run ID was known at cancellation time.
      if (cancelled.groupId) return false
      return Boolean(cancelled.runId && payload.runId && cancelled.runId !== payload.runId)
    },
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
      cancellations.delete(id)
      retireExternal(id)
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
      advanceTurn(id)
      publish()
      return lease
    },
    observe(event: ExecutionEvent): boolean {
      if (event.kind === 'deferTerminal') return settlement.deferTerminal(event.terminal)
      const id = event.conversationId
      if (event.kind === 'runEvent') {
        if (event.runId && retiredExternalRunIds.get(id)?.has(event.runId)) return false
        if (!settlement.acceptRunEvent(id, event.runId, event.started)) return false
        if (event.started && event.runId) {
          const invocation = active.get(id)
          if (invocation) {
            invocation.runIds.add(event.runId)
            const cancelled = cancellations.get(id)
            if (cancelled && !cancelled.runId) cancelled.runId = event.runId
          }
          else {
            const cancelled = cancellations.get(id)
            if (cancelled) {
              if (!cancelled.runId) {
                cancelled.runId = event.runId
                cancelled.groupId = event.groupId ?? cancelled.groupId
              } else if (cancelled.runId !== event.runId
                && event.groupId && cancelled.groupId !== event.groupId) {
                // Different run IDs may be arms of one restored group. Only an
                // explicit different group identity proves a new execution.
                cancellations.delete(id)
              }
            }
            let run = externalRuns.get(id)
            const newGroup = Boolean(run && event.groupId && event.groupId !== run.groupId)
            const replacedRun = Boolean(run && run.ready && !run.runIds.has(event.runId))
            if (newGroup || replacedRun) {
              retireExternal(id)
              run = undefined
            }
            if (!run) {
              advanceTurn(id)
              run = {
                token: ++externalSequence,
                groupId: event.groupId ?? null,
                expectedArms: Math.max(1, event.groupSize ?? 1),
                runIds: new Set(), terminalIds: new Set(), ready: false, groupEnded: false,
              }
              externalRuns.set(id, run)
            }
            run.runIds.add(event.runId)
            if (event.groupSize) run.expectedArms = Math.max(run.expectedArms, event.groupSize)
            external.add(id)
          }
          publish()
        }
        return true
      }
      if (event.kind === 'externalStarted' && !active.has(id)) {
        if (!external.has(id)) advanceTurn(id)
        external.add(id)
      }
      if (event.kind === 'externalEnded') {
        external.delete(id)
        retireExternal(id)
        cancellations.delete(id)
      }
      if (event.kind === 'drop') {
        advanceTurn(id)
        const invocation = active.get(id)
        if (invocation?.claim) abandonSend(invocation.claim)
        if (invocation?.optimisticToken != null) optimistic.settle(id, invocation.optimisticToken)
        if (invocation?.groupId) groups.end(id)
        active.delete(id)
        external.delete(id)
        retireExternal(id)
        cancellations.delete(id)
        settlement.clearConversation(id)
        optimistic.clear(id)
      }
      publish()
      return true
    },
    /** Terminal ownership is decided here, where invoke and recovered-run
     * identities already live. A restored group settles once, after all arms. */
    observeTerminal(terminal: ChatRunTerminal, target: 'single' | 'group'): TerminalDisposition {
      const id = terminal.conversationId
      const invocation = active.get(id)
      if (invocation) {
        if (target === 'group' || invocation.groupId) return { kind: 'pending' }
        return settlement.deferTerminal(terminal) ? { kind: 'deferred' } : { kind: 'ignored' }
      }
      let run = externalRuns.get(id)
      if (!run && terminal.runId
        && !retiredExternalRunIds.get(id)?.has(terminal.runId)) {
        // A terminal can be the first observed packet after a protocol gap.
        advanceTurn(id)
        run = {
          token: ++externalSequence, groupId: null, expectedArms: 1,
          runIds: new Set([terminal.runId]), terminalIds: new Set(),
          ready: false, groupEnded: false,
        }
        externalRuns.set(id, run)
        external.add(id)
      }
      if (!run || !terminal.runId || !run.runIds.has(terminal.runId)
        || run.terminalIds.has(terminal.runId)) return { kind: 'ignored' }
      run.terminalIds.add(terminal.runId)
      if (run.terminalIds.size < run.expectedArms) return { kind: 'pending' }
      run.ready = true
      if (run.groupId && !run.groupEnded) {
        groups.end(id)
        run.groupEnded = true
      }
      return {
        kind: 'ready',
        permit: { conversationId: id, token: run.token } as ExternalTerminalPermit,
      }
    },
    isExternalTerminalCurrent,
    completeExternalTerminal(permit: ExternalTerminalPermit): boolean {
      if (!isExternalTerminalCurrent(permit)) return false
      external.delete(permit.conversationId)
      retireExternal(permit.conversationId)
      cancellations.delete(permit.conversationId)
      publish()
      return true
    },
    async finish(lease: ExecutionLease, persisted: Conversation | null, ports: SettlementPorts): Promise<void> {
      const id = lease.conversationId
      const invocation = active.get(id)
      if (!invocation || invocation.lease.token !== lease.token) return
      active.delete(id)
      cancellations.delete(id)
      if (invocation.optimisticToken != null) optimistic.settle(id, invocation.optimisticToken)
      if (invocation.groupId) groups.end(id)
      if (invocation.claim) abandonSend(invocation.claim)
      publish()
      await settlement.settleInvoke(id, lease.token, persisted, ports)
    },
    /** Prepared send, whether single- or multi-answer. The caller owns conversation preparation,
     * canonical fan-out selection and UI projection; this method owns invoke
     * classification and the release-before-queue settlement order. */
    async submitPreparedRun(
      intent: PreparedRunIntent,
      effects: PreparedRunEffects,
    ): Promise<PreparedRunOutcome> {
      if (!active.get(intent.lease.conversationId)
        || active.get(intent.lease.conversationId)?.lease.token !== intent.lease.token) {
        return { kind: 'not_committed', error: new Error('该对话没有活跃发送') }
      }
      let outcome: PreparedRunOutcome
      let persistedForSettlement: Conversation | null = null
      try {
        const conversation = await sendPort.sendMessage(
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
        console.error('Failed to present Chat run outcome:', error)
      } finally {
        await this.finish(intent.lease, persistedForSettlement, effects)
      }
      return outcome
    },
  }
}
