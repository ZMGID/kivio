import type { ChatStreamPayload } from '../api/tauri'
import {
  ensureGroupColumn,
  flushGroups,
  getActiveGroup,
  hasActiveGroup,
  restoreGroupArm,
  touchGroup,
  type ActiveGroupState,
  type GroupColumnSnapshot,
} from './groupStreamingStore'
import { type ConversationStreamSnapshot } from './conversationRuns'
import { hasStreamPreview, isStreamTerminal } from './streamApply'
import { applyConversationStreamEvent, beginRunSnapshot, restoreRunSnapshot } from './streamPresentation'
import { applyRunDisplayEvent, type RunDisplayEvent, type RunDisplayResult } from './runDisplayEvents'
import { reset, setCoarse, setSnapshot } from './streamingStore'
import type { ChatMessage } from './types'

type Completion =
  | { kind: 'persisted'; committedMessages: ChatMessage[] }
  | { kind: 'error' | 'cancelled' }

type Projection = { accepted: boolean; target: 'single' | 'group'; terminal: boolean }

type CancellationFreeze =
  | {
    kind: 'single'
    generation: number
    snapshot: ConversationStreamSnapshot
    reasoningStreaming: boolean
    terminal: boolean
  }
  | {
    kind: 'group'
    group: ActiveGroupState
    runningArms: Map<GroupColumnSnapshot, boolean>
  }

function frameInterval(snapshot: ConversationStreamSnapshot): number {
  const contentSize = snapshot.content.length + snapshot.reasoning.length
  const structuredSize = snapshot.toolCalls.length * 512 + snapshot.segments.length * 64
  const totalSize = contentSize + structuredSize
  const foreground = totalSize >= 250_000 ? 220
    : totalSize >= 120_000 ? 180
      : totalSize >= 60_000 ? 140
        : snapshot.toolCalls.length > 0 || snapshot.segments.length > 8 ? 120
          : totalSize >= 12_000 ? 80
            : 50
  if (typeof document !== 'undefined' && document.hidden) {
    return Math.min(750, Math.max(160, foreground * 5))
  }
  // Keep the existing visible-document test cadence until Chat migrates from
  // useStreamRenderFrame. Production retains its content-size backpressure.
  return import.meta.env.MODE === 'test' ? 0 : foreground
}

/**
 * Owns the display lifetime of a Chat run, not its execution lifetime.
 * The execution owner accepts/retires run IDs and begins/ends multi-answer
 * groups. This Module projects only already accepted stream packets.
 *
 * The two streaming stores are rendering Adapters: neither owns per-conversation
 * preview state or decides when a persisted twin may replace the live answer.
 */
export function createStreamPreviewOwner() {
  const snapshots = new Map<string, ConversationStreamSnapshot>()
  const groupStarts = new Map<string, number>()
  const generations = new Map<string, number>()
  const cancellationFreezes = new Map<string, CancellationFreeze>()
  let selectedId: string | null = null
  let pendingFrame: { conversationId: string; snapshot: ConversationStreamSnapshot } | null = null
  let frameHandle: number | null = null
  let delayHandle: ReturnType<typeof setTimeout> | null = null
  let lastFlushAt = 0
  let pendingTwin: {
    conversationId: string
    generation: number
    snapshot: ConversationStreamSnapshot
    messageId: string | null
    committedMessages: ChatMessage[]
    timeout: ReturnType<typeof setTimeout>
  } | null = null
  let disposed = false

  const cancelFrame = () => {
    if (frameHandle !== null) cancelAnimationFrame(frameHandle)
    if (delayHandle !== null) clearTimeout(delayHandle)
    frameHandle = null
    delayHandle = null
    pendingFrame = null
  }

  const cancelTwin = () => {
    if (pendingTwin) clearTimeout(pendingTwin.timeout)
    pendingTwin = null
  }

  const publish = (conversationId: string, snapshot: ConversationStreamSnapshot) => {
    if (disposed || selectedId !== conversationId) return
    lastFlushAt = performance.now()
    setSnapshot(snapshot)
    setCoarse({ streaming: snapshot.streaming, cancelling: false })
  }

  const flushFrame = () => {
    const next = pendingFrame
    cancelFrame()
    if (next) publish(next.conversationId, next.snapshot)
  }

  const scheduleFrame = () => {
    if (disposed || !pendingFrame || frameHandle !== null || delayHandle !== null) return
    const wait = Math.max(0, frameInterval(pendingFrame.snapshot) - (performance.now() - lastFlushAt))
    if (typeof document !== 'undefined' && document.hidden) {
      delayHandle = setTimeout(() => {
        delayHandle = null
        flushFrame()
      }, wait)
    } else if (wait > 0) {
      delayHandle = setTimeout(() => {
        delayHandle = null
        scheduleFrame()
      }, wait)
    } else {
      frameHandle = requestAnimationFrame(() => {
        frameHandle = null
        flushFrame()
      })
    }
  }

  const showLater = (conversationId: string, snapshot: ConversationStreamSnapshot, immediate: boolean) => {
    if (selectedId !== conversationId) return
    pendingFrame = { conversationId, snapshot }
    if (immediate) flushFrame()
    else scheduleFrame()
  }

  const clearVisible = () => {
    cancelFrame()
    reset() // intentionally preserves the independently owned stream error
  }

  const clearTwinIfCurrent = (twin: NonNullable<typeof pendingTwin>) => {
    if (disposed || pendingTwin !== twin) return
    pendingTwin = null
    clearTimeout(twin.timeout)
    if (selectedId === twin.conversationId
      && generations.get(twin.conversationId) === twin.generation) clearVisible()
  }

  const freeze = (conversationId: string): boolean => {
    if (disposed) return false
    cancellationFreezes.delete(conversationId)
    const group = getActiveGroup(conversationId)
    if (group) {
      for (const column of group.columns) {
        column.streaming = false
        column.reasoningStreaming = false
      }
      touchGroup(conversationId)
      flushGroups(conversationId)
      if (selectedId === conversationId) {
        cancelFrame()
        setCoarse({ streaming: false, streamFrozen: true })
      }
      return true
    }
    const snapshot = snapshots.get(conversationId)
    if (!snapshot || !hasStreamPreview(snapshot)) {
      snapshots.delete(conversationId)
      if (selectedId === conversationId) clearVisible()
      return false
    }
    snapshot.streaming = false
    snapshot.reasoningStreaming = false
    if (selectedId === conversationId) {
      cancelFrame()
      setSnapshot(snapshot)
      setCoarse({ streaming: false, streamFrozen: true })
    }
    return true
  }

  const freezeForCancellation = (conversationId: string): boolean => {
    if (disposed) return false
    const group = getActiveGroup(conversationId)
    if (group) {
      const runningArms = new Map<GroupColumnSnapshot, boolean>()
      for (const column of group.columns) {
        if (column.streaming) runningArms.set(column, column.reasoningStreaming)
        column.streaming = false
        column.reasoningStreaming = false
      }
      cancellationFreezes.set(conversationId, { kind: 'group', group, runningArms })
      touchGroup(conversationId)
      flushGroups(conversationId)
      if (selectedId === conversationId) {
        cancelFrame()
        setCoarse({ streaming: false, streamFrozen: true })
      }
      return true
    }
    const snapshot = snapshots.get(conversationId)
    if (!snapshot) return false
    cancellationFreezes.set(conversationId, {
      kind: 'single',
      generation: generations.get(conversationId) ?? 0,
      snapshot,
      reasoningStreaming: snapshot.reasoningStreaming,
      terminal: false,
    })
    snapshot.streaming = false
    snapshot.reasoningStreaming = false
    if (selectedId === conversationId) {
      cancelFrame()
      setSnapshot(snapshot)
      setCoarse({ streaming: false, streamFrozen: hasStreamPreview(snapshot) })
    }
    return true
  }

  return {
    /** React StrictMode replays effect cleanup/setup without constructing a new
     * owner. Re-arm presentation on setup after the simulated unmount. */
    attach(): void {
      disposed = false
    },
    summary(conversationId: string): Readonly<ConversationStreamSnapshot> | null {
      return snapshots.get(conversationId) ?? null
    },
    timing(conversationId: string) {
      const snapshot = snapshots.get(conversationId)
      if (snapshot) return snapshot
      const startedAt = groupStarts.get(conversationId)
      return startedAt == null ? null : {
        startedAt, content: '', reasoning: '',
        reasoningDurationMs: null,
        reasoningDurationMsBySegmentId: {} as Record<string, number>,
      }
    },
    isStreaming(conversationId: string): boolean {
      return snapshots.get(conversationId)?.streaming === true
    },
    streamingConversationIds(): string[] {
      return [...snapshots].filter(([, snapshot]) => snapshot.streaming).map(([id]) => id)
    },
    /** Switches only the visible projection; background snapshots keep accruing. */
    activate(conversationId: string | null): void {
      if (disposed) return
      cancelFrame()
      if (pendingTwin && pendingTwin.conversationId !== conversationId) cancelTwin()
      selectedId = conversationId
      const snapshot = conversationId
        ? snapshots.get(conversationId)
          ?? (pendingTwin?.conversationId === conversationId ? pendingTwin.snapshot : null)
        : null
      if (!snapshot) {
        clearVisible()
        const group = conversationId ? getActiveGroup(conversationId) : null
        if (group) {
          const streaming = group.columns.some((column) => column.streaming)
          setCoarse({ streaming, streamFrozen: !streaming })
        }
        return
      }
      setSnapshot(snapshot)
      setCoarse({ streaming: snapshot.streaming, streamFrozen: !snapshot.streaming && hasStreamPreview(snapshot), cancelling: false })
    },
    /** Begins a single preview. The execution owner separately reserves the run. */
    begin(conversationId: string, startedAt = Date.now(), mode: 'single' | 'group' = 'single'): void {
      if (disposed) return
      cancellationFreezes.delete(conversationId)
      generations.set(conversationId, (generations.get(conversationId) ?? 0) + 1)
      if (pendingTwin?.conversationId === conversationId) cancelTwin()
      if (mode === 'single') {
        groupStarts.delete(conversationId)
        snapshots.set(conversationId, beginRunSnapshot(startedAt))
      } else {
        snapshots.delete(conversationId)
        groupStarts.set(conversationId, startedAt)
      }
      if (selectedId === conversationId) {
        clearVisible()
        if (mode === 'single') setSnapshot(snapshots.get(conversationId)!)
        setCoarse({ streaming: true, streamFrozen: false })
      }
    },
    /** Only project an event after executionOwner.observe accepted its run ID. */
    receive(payload: ChatStreamPayload, now = Date.now()): Projection {
      const terminal = isStreamTerminal(payload)
      if (disposed) return { accepted: false, target: 'single', terminal }
      if (payload.type === 'run_started' && pendingTwin?.conversationId === payload.conversationId) {
        cancelTwin()
        if (selectedId === payload.conversationId) clearVisible()
      }
      if (payload.type === 'run_started' && !snapshots.has(payload.conversationId)
        && !groupStarts.has(payload.conversationId) && !payload.recovery) {
        // A recovered single run did not pass through begin(), but still needs
        // a generation for its persisted-twin fallback and later run turnover.
        generations.set(payload.conversationId, (generations.get(payload.conversationId) ?? 0) + 1)
      }
      if (payload.type === 'run_started' && payload.recovery) {
        if (!groupStarts.has(payload.conversationId)) groupStarts.set(payload.conversationId, now)
        restoreGroupArm(
          payload.conversationId,
          payload.recovery.groupId,
          payload.recovery.groupSize,
          payload.recovery.armIndex,
          payload.messageId,
          payload.recovery.providerId,
          payload.recovery.model,
        )
      }
      if (hasActiveGroup(payload.conversationId) && payload.messageId) {
        const column = ensureGroupColumn(payload.conversationId, payload.messageId)
        if (!column) return { accepted: false, target: 'group', terminal }
        if (payload.type === 'run_started') Object.assign(column, restoreRunSnapshot(payload, now))
        else if (!applyConversationStreamEvent(column, payload, now)) {
          return { accepted: false, target: 'group', terminal }
        }
        if (selectedId === payload.conversationId && !cancellationFreezes.has(payload.conversationId)) {
          setCoarse({ streaming: true, streamFrozen: false, cancelling: false })
        }
        if (terminal) {
          const frozen = cancellationFreezes.get(payload.conversationId)
          if (frozen?.kind === 'group' && frozen.group === getActiveGroup(payload.conversationId)) {
            frozen.runningArms.delete(column)
          }
          column.streaming = false
          flushGroups(payload.conversationId)
        } else touchGroup(payload.conversationId)
        return { accepted: true, target: 'group', terminal }
      }
      let snapshot = snapshots.get(payload.conversationId)
      if (payload.type === 'run_started' && snapshot && !snapshot.streaming
        && snapshot.runId !== payload.runId) {
        // A backend-owned wake/recovery run can start after the previous local
        // run failed and left a frozen partial answer. Only a stopped preview
        // may be replaced here; an active preview keeps rejecting foreign runs.
        generations.set(payload.conversationId, (generations.get(payload.conversationId) ?? 0) + 1)
        cancellationFreezes.delete(payload.conversationId)
        if (pendingTwin?.conversationId === payload.conversationId) cancelTwin()
        cancelFrame()
        snapshot = restoreRunSnapshot(payload, now)
        snapshots.set(payload.conversationId, snapshot)
        if (selectedId === payload.conversationId) {
          clearVisible()
          setCoarse({ streaming: true, streamFrozen: false })
        }
      }
      if (!snapshot) {
        snapshot = payload.type === 'run_started' ? restoreRunSnapshot(payload, now) : beginRunSnapshot(now)
        snapshots.set(payload.conversationId, snapshot)
      }
      if (!applyConversationStreamEvent(snapshot, payload, now)) {
        return { accepted: false, target: 'single', terminal }
      }
      if (terminal) {
        const frozen = cancellationFreezes.get(payload.conversationId)
        if (frozen?.kind === 'single' && frozen.snapshot === snapshot) frozen.terminal = true
      }
      showLater(payload.conversationId, snapshot, terminal || payload.type === 'run_started')
      return { accepted: true, target: 'single', terminal }
    },
    /** Projects run-scoped cards/status after the execution owner accepted them. */
    projectDisplay(event: RunDisplayEvent): RunDisplayResult {
      if (disposed) return { accepted: false }
      const id = event.kind === 'subagent' ? event.payload.parentConversationId : event.payload.conversationId
      if (event.kind === 'tool' && hasActiveGroup(id) && event.payload.messageId) {
        const column = ensureGroupColumn(id, event.payload.messageId)
        if (!column) return { accepted: false }
        const result = applyRunDisplayEvent(column, event)
        if (result.accepted) touchGroup(id)
        return result
      }
      let snapshot = snapshots.get(id)
      if (!snapshot) {
        if (event.kind === 'status') return { accepted: false }
        snapshot = beginRunSnapshot()
        snapshots.set(id, snapshot)
      }
      const result = applyRunDisplayEvent(snapshot, event)
      if (result.accepted) showLater(id, snapshot, false)
      return result
    },
    /** Immediate local freeze; terminal packets must still reach executionOwner. */
    freeze,
    /** Preserve which previews were live, so a rejected backend cancel can
     * reopen only arms that have not terminated in the meantime. */
    freezeForCancellation,
    /** A rejected cancel request did not stop the backend run. Reopen its live
     * preview so later accepted deltas remain visible and retry is possible. */
    resume(conversationId: string): boolean {
      if (disposed) return false
      const frozen = cancellationFreezes.get(conversationId)
      if (!frozen) return false
      cancellationFreezes.delete(conversationId)
      if (frozen.kind === 'group') {
        if (getActiveGroup(conversationId) !== frozen.group) return false
        let restored = 0
        for (const [column, reasoningStreaming] of frozen.runningArms) {
          if (!frozen.group.columns.includes(column)) continue
          column.streaming = true
          column.reasoningStreaming = reasoningStreaming
          restored += 1
        }
        if (restored === 0) return false
        touchGroup(conversationId)
        flushGroups(conversationId)
      } else {
        const snapshot = snapshots.get(conversationId)
        if (frozen.terminal || snapshot !== frozen.snapshot
          || generations.get(conversationId) !== frozen.generation) return false
        snapshot.streaming = true
        snapshot.reasoningStreaming = frozen.reasoningStreaming
        if (selectedId === conversationId) setSnapshot(snapshot)
      }
      if (selectedId === conversationId) {
        setCoarse({ streaming: true, streamFrozen: false, cancelling: false })
      }
      return true
    },
    /** Release display only; queue and execution settlement belong elsewhere. */
    complete(conversationId: string, completion: Completion): void {
      if (disposed) return
      cancellationFreezes.delete(conversationId)
      const snapshot = snapshots.get(conversationId)
      groupStarts.delete(conversationId)
      if (completion.kind !== 'persisted') {
        freeze(conversationId)
        return
      }
      snapshots.delete(conversationId)
      if (selectedId !== conversationId) return
      if (!hasStreamPreview(snapshot)) {
        clearVisible()
        return
      }
      cancelFrame()
      snapshot!.streaming = false
      snapshot!.reasoningStreaming = false
      const messageId = snapshot!.messageId
      const committedMessages = completion.committedMessages
      setSnapshot(snapshot!)
      setCoarse({ streaming: false, streamFrozen: true, cancelling: false })
      const generation = generations.get(conversationId) ?? 0
      if (messageId && committedMessages.some((message) => message.id === messageId)) {
        clearVisible()
        return
      }
      cancelTwin()
      const twin = {
        conversationId, generation, snapshot: snapshot!, messageId, committedMessages,
        timeout: setTimeout(() => { if (pendingTwin) clearTwinIfCurrent(twin) }, 1_500),
      }
      pendingTwin = twin
    },
    /** Call after React commits authoritative messages, not when invoke resolves. */
    reconcile(conversationId: string, messages: ChatMessage[]): void {
      const twin = pendingTwin
      if (!twin || twin.conversationId !== conversationId) return
      const landed = twin.messageId
        ? messages.some((message) => message.id === twin.messageId)
        : messages !== twin.committedMessages
      if (landed) clearTwinIfCurrent(twin)
    },
    drop(conversationId: string): void {
      if (disposed) return
      cancellationFreezes.delete(conversationId)
      snapshots.delete(conversationId)
      groupStarts.delete(conversationId)
      generations.delete(conversationId)
      if (pendingTwin?.conversationId === conversationId) cancelTwin()
      if (selectedId === conversationId) clearVisible()
      else if (pendingFrame?.conversationId === conversationId) cancelFrame()
    },
    /** Unmount only tears down presentation callbacks, never backend execution. */
    dispose(): void {
      disposed = true
      cancelFrame()
      cancelTwin()
      snapshots.clear()
      groupStarts.clear()
      generations.clear()
      cancellationFreezes.clear()
    },
  }
}
