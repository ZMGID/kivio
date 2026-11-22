import type { ChatMessage, CompactionBoundaryRecord, ConversationContextState } from './types'

export interface CompactionBoundaryView {
  afterIndex: number
  record: CompactionBoundaryRecord
}

function readUntilId(record: CompactionBoundaryRecord): string | null {
  return record.source_until_message_id ?? record.sourceUntilMessageId ?? null
}

/** Timeline anchor: where the divider renders (falls back to the context split point
 * for records persisted before `display_after_message_id` existed). */
function readDisplayAfterId(record: CompactionBoundaryRecord): string | null {
  return (
    record.display_after_message_id
    ?? record.displayAfterMessageId
    ?? readUntilId(record)
  )
}

function readSummaryContent(record: CompactionBoundaryRecord): string {
  return record.summary_content ?? record.summaryContent ?? ''
}

function readTokenBefore(record: CompactionBoundaryRecord): number {
  return record.token_estimate_before ?? record.tokenEstimateBefore ?? 0
}

function readTokenAfter(record: CompactionBoundaryRecord): number {
  return record.token_estimate_after ?? record.tokenEstimateAfter ?? 0
}

function legacySummaryToRecord(
  summary: NonNullable<ConversationContextState['summary']>,
): CompactionBoundaryRecord | null {
  const sourceUntil = summary.source_until_message_id ?? summary.sourceUntilMessageId
  if (!sourceUntil || summary.stale) return null
  return {
    id: summary.id,
    source_until_message_id: sourceUntil,
    token_estimate_before: summary.token_estimate_before ?? summary.tokenEstimateBefore ?? 0,
    token_estimate_after: summary.token_estimate_after ?? summary.tokenEstimateAfter ?? 0,
    summary_content: summary.content,
    trigger: 'auto',
    created_at: summary.created_at ?? summary.createdAt,
  }
}

export function collectCompactionRecords(
  contextState?: ConversationContextState | null,
): CompactionBoundaryRecord[] {
  if (!contextState) return []
  const explicit = contextState.compaction_boundaries ?? contextState.compactionBoundaries ?? []
  if (explicit.length > 0) return explicit
  const summary = contextState.summary
  if (!summary) return []
  const legacy = legacySummaryToRecord(summary)
  return legacy ? [legacy] : []
}

export function resolveCompactionBoundaries(
  messages: ChatMessage[],
  contextState?: ConversationContextState | null,
): CompactionBoundaryView[] {
  const records = collectCompactionRecords(contextState)
  const views: CompactionBoundaryView[] = []

  for (const record of records) {
    const anchorId = readDisplayAfterId(record)
    if (!anchorId) continue
    let afterIndex = messages.findIndex((message) => message.id === anchorId)
    if (afterIndex < 0) {
      // The trigger-time anchor is usually the latest assistant message, which can be
      // truncated away by regenerate/delete. Fall back to the context split point so
      // the timeline marker degrades gracefully instead of vanishing.
      const untilId = readUntilId(record)
      if (untilId && untilId !== anchorId) {
        afterIndex = messages.findIndex((message) => message.id === untilId)
      }
    }
    if (afterIndex < 0) continue
    views.push({ afterIndex, record })
  }

  views.sort((a, b) => a.afterIndex - b.afterIndex || (a.record.created_at ?? 0) - (b.record.created_at ?? 0))
  return views
}

export function hasCompactionTokenDetail(record: CompactionBoundaryRecord): boolean {
  return readTokenBefore(record) > 0 || readTokenAfter(record) > 0
}

export function compactionTriggerLabel(trigger: string | undefined, t: { contextCompactionTriggerManual: string; contextCompactionTriggerAuto: string; contextCompactionTriggerAgentLoop: string; contextCompactionTriggerGeneric: string }): string {
  if (trigger === 'manual') return t.contextCompactionTriggerManual
  if (trigger === 'auto') return t.contextCompactionTriggerAuto
  if (trigger === 'agent_loop') return t.contextCompactionTriggerAgentLoop
  return t.contextCompactionTriggerGeneric
}

export function compactionRecordTokens(record: CompactionBoundaryRecord) {
  return {
    before: readTokenBefore(record),
    after: readTokenAfter(record),
    summary: readSummaryContent(record),
  }
}

/** Preserve compaction timeline markers when a partial context refresh omits them. */
export function mergeCompactionContextState(
  prev: ConversationContextState | null | undefined,
  next: ConversationContextState,
): ConversationContextState {
  const prevRecords = collectCompactionRecords(prev)
  const nextRecords = collectCompactionRecords(next)
  if (nextRecords.length >= prevRecords.length) return next
  const byId = new Map<string, CompactionBoundaryRecord>()
  for (const record of prevRecords) byId.set(record.id, record)
  for (const record of nextRecords) byId.set(record.id, record)
  const merged = [...byId.values()].sort(
    (a, b) => (a.created_at ?? a.createdAt ?? 0) - (b.created_at ?? b.createdAt ?? 0),
  )
  return {
    ...next,
    compaction_boundaries: merged,
    compactionBoundaries: merged,
  }
}
