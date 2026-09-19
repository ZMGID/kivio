import type { ChatStreamPayload } from '../api/tauri'
import { createChatExecutionOwner, type ExternalTerminalPermit } from './chatExecutionOwner'
import type { ChatRunTerminal } from './chatRunSettlement'
import { getActiveGroup } from './groupStreamingStore'
import { isStreamTerminal, streamTerminalReason } from './streamApply'
import { createStreamPreviewOwner } from './streamPreviewOwner'

type ExecutionOwner = ReturnType<typeof createChatExecutionOwner>
type PreviewOwner = ReturnType<typeof createStreamPreviewOwner>

export type StreamLifecycleResult =
  | { kind: 'ignored' }
  | { kind: 'started'; conversationId: string; runId: string; external: boolean }
  | { kind: 'projected'; conversationId: string }
  | { kind: 'pending' | 'deferred'; terminal: ChatRunTerminal }
  | { kind: 'ready'; terminal: ChatRunTerminal; permit: ExternalTerminalPermit }

export type TerminalLoadResult<T> =
  | { kind: 'loaded'; value: T }
  | { kind: 'failed'; error: Error }

export type CancelRunResult =
  | { kind: 'ignored' | 'cancelled' | 'superseded' }
  | { kind: 'failed'; error: Error }

function asError(value: unknown, fallback = '会话回载失败，请重试'): Error {
  return value instanceof Error
    ? value
    : new Error(typeof value === 'string' ? value : fallback)
}

/** Translates accepted protocol packets into display projection and execution
 * settlement decisions. Run and group identities remain in executionOwner;
 * this Module keeps no second registry of recovered run IDs. */
export function createChatStreamLifecycleOwner(executionOwner: ExecutionOwner, previewOwner: PreviewOwner) {
  const classifyTerminal = (terminal: ChatRunTerminal, target: 'single' | 'group'): StreamLifecycleResult => {
    const decision = executionOwner.observeTerminal(terminal, target)
    if (decision.kind === 'ignored') return { kind: 'ignored' }
    if (decision.kind === 'ready') return { kind: 'ready', terminal, permit: decision.permit }
    return { kind: decision.kind, terminal }
  }
  return {
    /** One cancellation protocol for the main view and popout. The view only
     * projects pending/error state; this Module owns the fence and rollback. */
    async cancelRun(
      conversationId: string,
      cancel: () => Promise<void>,
      onPending?: () => void,
    ): Promise<CancelRunResult> {
      const permit = executionOwner.requestCancellation(
        conversationId, previewOwner.summary(conversationId)?.runId ?? null,
      )
      if (!permit) return { kind: 'ignored' }
      previewOwner.freezeForCancellation(conversationId)
      try {
        onPending?.()
        await cancel()
        return executionOwner.completeCancellation(permit, true)
          ? { kind: 'cancelled' }
          : { kind: 'superseded' }
      } catch (value) {
        if (!executionOwner.completeCancellation(permit, false)) return { kind: 'superseded' }
        previewOwner.resume(conversationId)
        return { kind: 'failed', error: asError(value, '停止生成失败') }
      }
    },
    receive(payload: ChatStreamPayload, options: { project?: boolean } = {}): StreamLifecycleResult {
      const id = payload.conversationId
      const started = payload.type === 'run_started'
      if (!started && !executionOwner.allowsStreamPayload(payload)) return { kind: 'ignored' }
      const wasInFlight = executionOwner.snapshot(id).inFlight
      if (!executionOwner.observe({
        kind: 'runEvent', conversationId: id, runId: payload.runId,
        started, groupId: started ? payload.recovery?.groupId : undefined,
        groupSize: started ? payload.recovery?.groupSize : undefined,
      })) return { kind: 'ignored' }
      if (started && !executionOwner.allowsStreamPayload(payload)) return { kind: 'ignored' }

      const terminal = isStreamTerminal(payload)
      const terminalPayload = terminal
        ? { conversationId: id, runId: payload.runId, reason: streamTerminalReason(payload), turnEpoch: executionOwner.turnEpoch(id) }
        : null
      if (options.project === false) {
        if (started) return { kind: 'started', conversationId: id, runId: payload.runId, external: !wasInFlight }
        return terminalPayload
          ? classifyTerminal(terminalPayload, 'single')
          : { kind: 'projected', conversationId: id }
      }
      if (!started && !previewOwner.summary(id) && !getActiveGroup(id)
        && !executionOwner.snapshot(id).inFlight) {
        return terminalPayload ? classifyTerminal(terminalPayload, 'single') : { kind: 'ignored' }
      }
      const projection = previewOwner.receive(payload)
      if (!projection.accepted) return { kind: 'ignored' }
      if (started) {
        return { kind: 'started', conversationId: id, runId: payload.runId, external: !wasInFlight }
      }
      if (!terminalPayload) return { kind: 'projected', conversationId: id }
      return classifyTerminal(terminalPayload, projection.target)
    },
    /** Load may be asynchronous; only a still-current external run may commit
     * its authoritative result or clear execution after the await. */
    async settleExternalTerminal<T>(
      permit: ExternalTerminalPermit,
      load: () => Promise<T>,
      commit: (outcome: TerminalLoadResult<T>) => void,
    ): Promise<boolean> {
      if (!executionOwner.isExternalTerminalCurrent(permit)) return false
      let outcome: TerminalLoadResult<T>
      try {
        outcome = { kind: 'loaded', value: await load() }
      } catch (value) {
        outcome = { kind: 'failed', error: asError(value) }
      }
      if (!executionOwner.isExternalTerminalCurrent(permit)) return false
      if (!executionOwner.completeExternalTerminal(permit)) return false
      commit(outcome)
      return true
    },
  }
}
