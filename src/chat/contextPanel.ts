import type { ChatContextLiveUsage } from '../api/tauri'
import { mergeCompactionContextState } from './compactionBoundary'
import { mergeClearContextState } from './contextClearBoundary'
import type { Conversation, ConversationContextState } from './types'

export const CONTEXT_WARNING_PERCENT = 70
export const CONTEXT_CRITICAL_PERCENT = 95

/** Old estimated/mixed snapshots are deliberately not promoted to API reports. */
export function reportedContextTokens(state: ConversationContextState | null | undefined): number | null {
  const source = state?.token_count_source ?? state?.tokenCountSource
  const tokens = source === 'provider_context_reported'
    ? state?.reported_context_tokens ?? state?.reportedContextTokens
    : source === 'cli_reported'
      ? state?.estimated_input_tokens ?? state?.estimatedInputTokens
      : undefined
  return tokens != null && Number.isFinite(tokens) && tokens >= 0 ? tokens : null
}

/** Backend measurement order. Missing means the payload predates the contract. */
export function readMeasurementSeq(
  value: { measurement_seq?: number | null; measurementSeq?: number | null } | null | undefined,
): number | null {
  const seq = value?.measurement_seq ?? value?.measurementSeq
  return typeof seq === 'number' && Number.isFinite(seq) && seq >= 0 ? seq : null
}

/** A sequenced measurement replaces the applied one only at an equal or newer
 * sequence. An unsequenced payload must not roll back a sequenced measure. */
export function measurementSupersedes(current: number | null, incoming: number | null): boolean {
  if (incoming == null) return current == null
  if (current == null) return true
  return incoming >= current
}

export function readLifecycleId(
  value: { lifecycle_id?: number | null; lifecycleId?: number | null } | null | undefined,
): number | null {
  const id = value?.lifecycle_id ?? value?.lifecycleId
  return typeof id === 'number' && Number.isFinite(id) && id >= 0 ? id : null
}

/** Full snapshot or live report. Main, popout, and restore all apply this. */
export type ContextMeasurementIncoming =
  | { kind: 'snapshot'; state: ConversationContextState }
  | { kind: 'live'; usage: ChatContextLiveUsage }

/** Shared measurement merge.
 * A lower `measurementSeq` leaves the meter, cache, and categories untouched.
 * A higher `lifecycleId` drops omitted categories and cache; the backend sequence
 * still moves forward. Live `segments` null or omitted keeps the previous
 * categories, and `[]` replaces them. A provider report that omits cache keeps
 * the previous rate; a full snapshot with cache null clears it. */
export function mergeContextMeasurement(
  prev: ConversationContextState | null | undefined,
  incoming: ContextMeasurementIncoming,
): ConversationContextState {
  return incoming.kind === 'snapshot'
    ? mergeSnapshot(prev, incoming.state)
    : mergeLive(prev, incoming.usage)
}

function mergeSnapshot(
  prev: ConversationContextState | null | undefined,
  next: ConversationContextState,
): ConversationContextState {
  const snapshot = clearExplicitCache(next)
  if (!prev) return snapshot
  if (!measurementSupersedes(readMeasurementSeq(prev), readMeasurementSeq(next))) return prev
  return clearExplicitCache(mergeClearContextState(prev, mergeCompactionContextState(prev, snapshot)))
}

/** `null` on either cache alias clears both, so a stale twin cannot survive `??`. */
function clearExplicitCache(state: ConversationContextState): ConversationContextState {
  if (state.cache_hit_rate !== null && state.cacheHitRate !== null) return state
  if (state.cache_hit_rate === null || state.cacheHitRate === null) {
    return { ...state, cache_hit_rate: null, cacheHitRate: null }
  }
  return state
}

/** Full snapshots (refresh, restore, compaction, clear, model change). */
export function mergeContextSnapshot(
  prev: ConversationContextState | null | undefined,
  next: ConversationContextState,
): ConversationContextState {
  return mergeContextMeasurement(prev, { kind: 'snapshot', state: next })
}

/** Restored conversations may carry an older persisted meter under a newer
 * conversation revision. Keep the fresher measurement on the incoming shell. */
export function keepNewerContextMeasurement<T extends Conversation>(
  incoming: T,
  previous: Conversation | null,
): T {
  if (!previous || previous.id !== incoming.id) return incoming
  const prevState = previous.context_state ?? previous.contextState ?? null
  if (!prevState) return incoming
  const nextState = incoming.context_state ?? incoming.contextState
  if (!nextState) {
    return readMeasurementSeq(prevState) == null
      ? incoming
      : { ...incoming, context_state: prevState, contextState: prevState }
  }
  const merged = mergeContextMeasurement(prevState, { kind: 'snapshot', state: nextState })
  if (merged === nextState) return incoming
  return { ...incoming, context_state: merged, contextState: merged }
}

/** Only provider reports fill the meter. A source-less event leaves it unknown
 * and never imports the internal budget. Request segments on that event still
 * show; omitted segments stay, and an empty array replaces them. */
function mergeLive(
  prev: ConversationContextState | null | undefined,
  live: ChatContextLiveUsage,
): ConversationContextState {
  const incomingSeq = readMeasurementSeq(live)
  if (prev && !measurementSupersedes(readMeasurementSeq(prev), incomingSeq)) return prev
  const incomingLifecycle = readLifecycleId(live)
  const prevLifecycle = readLifecycleId(prev)
  const newLifecycle = prevLifecycle != null && incomingLifecycle != null && incomingLifecycle > prevLifecycle
  const lifecycle = incomingLifecycle ?? prevLifecycle ?? undefined
  const reported = live.tokenCountSource === 'provider_context_reported'
    && Number.isFinite(live.usedTokens) && live.usedTokens >= 0
  const tokens = reported ? Math.round(live.usedTokens) : undefined
  const window = live.contextWindowTokens
    ?? prev?.context_window_tokens ?? prev?.contextWindowTokens ?? null
  const ratio = tokens != null && window != null && window > 0 ? tokens / window : null
  const cacheMissing = live.cacheInputTokens == null && live.cacheReadTokens == null
  const cacheInput = live.cacheInputTokens
  const cacheRead = live.cacheReadTokens
  const providedRate = !cacheMissing && cacheInput != null && cacheRead != null
    && Number.isFinite(cacheInput) && Number.isFinite(cacheRead)
    && cacheInput > 0 && cacheRead >= 0 && cacheRead <= cacheInput
    ? cacheRead / cacheInput : undefined
  const cacheRate = providedRate != null
    ? providedRate
    : reported && !newLifecycle && cacheMissing
      ? prev?.cache_hit_rate ?? prev?.cacheHitRate
      : undefined
  const segments = live.segments == null
    ? (newLifecycle ? [] : prev?.segments ?? [])
    : live.segments
  return {
    ...prev,
    context_source: 'kivio_builtin',
    contextSource: 'kivio_builtin',
    context_window_tokens: window,
    contextWindowTokens: window,
    reported_context_tokens: tokens,
    reportedContextTokens: tokens,
    cache_hit_rate: cacheRate,
    cacheHitRate: cacheRate,
    usage_ratio: ratio,
    usageRatio: ratio,
    token_count_source: reported ? 'provider_context_reported' : undefined,
    tokenCountSource: reported ? 'provider_context_reported' : undefined,
    measurement_seq: incomingSeq ?? undefined,
    measurementSeq: incomingSeq ?? undefined,
    lifecycle_id: lifecycle,
    lifecycleId: lifecycle,
    status: ratio == null ? 'unknown' : ratio >= 0.95 ? 'critical' : ratio >= 0.7 ? 'warning' : 'normal',
    segments,
  }
}

export function applyLiveContextUsage(
  prev: ConversationContextState | null | undefined,
  live: ChatContextLiveUsage,
): ConversationContextState {
  return mergeContextMeasurement(prev, { kind: 'live', usage: live })
}

/** The backend owns model-dependent input reserves. */
export function autoCompactPercent(state: ConversationContextState | null | undefined): number | null {
  const budget = state?.auto_compact_threshold_tokens ?? state?.autoCompactThresholdTokens
  const window = state?.context_window_tokens ?? state?.contextWindowTokens
  return budget != null && window != null && window > 0
    ? Math.round(budget / window * 100) : null
}

const CATEGORY_IDS = ['system_prompt', 'tools', 'conversation'] as const
type CategoryId = typeof CATEGORY_IDS[number]

/** Character shares and independently estimated tokens; neither is scaled to API totals. */
export function contextBreakdown(segments: ConversationContextState['segments']) {
  const groups = new Map<CategoryId, { chars: number; estimatedTokens: number | null }>()
  for (const segment of segments ?? []) {
    const chars = Number.isFinite(segment.chars) && segment.chars! > 0 ? segment.chars! : 0
    const estimate = segment.estimated_tokens ?? segment.estimatedTokens
    const tokens = estimate != null && Number.isFinite(estimate) && estimate >= 0 ? estimate : null
    if (!chars && !tokens) continue
    const id: CategoryId = ['native_tools', 'tool_definitions', 'tools', 'mcp', 'skills'].includes(segment.id)
      ? 'tools'
      : ['conversation', 'summarized_conversation', 'attachments'].includes(segment.id)
        ? 'conversation'
        : 'system_prompt'
    const group = groups.get(id) ?? { chars: 0, estimatedTokens: 0 }
    group.chars += chars
    group.estimatedTokens = group.estimatedTokens != null && tokens != null
      ? group.estimatedTokens + tokens : null
    groups.set(id, group)
  }
  const total = [...groups.values()].reduce((sum, group) => sum + group.chars, 0)
  if (!total) return []
  return CATEGORY_IDS.filter((id) => groups.has(id))
    .map((id) => ({ id, percent: groups.get(id)!.chars / total, estimatedTokens: groups.get(id)!.estimatedTokens }))
    .sort((a, b) => b.percent - a.percent)
    .map((segment, rank) => ({
      ...segment,
      color: `color-mix(in srgb, var(--accent) ${[100, 78, 58][rank]}%, var(--theme-surface))`,
    }))
}
