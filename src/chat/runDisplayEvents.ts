import type { ChatSubagentPayload, ChatToolProgressPayload, ChatUserPromptPayload } from '../api/tauri'
import { acceptStreamRun, type ConversationStreamSnapshot } from './conversationRuns'
import { userFollowUpId, userSteerId } from './segments'
import {
  applyToolRecordToSnapshot,
  findSubagentToolIndex,
  mergeSubagentProgress,
  toolEventToRecord,
  userPromptEventToRecord,
} from './streamApply'
import { mergeToolRecord } from './conversationRuns'

export type RunDisplayEvent =
  | { kind: 'tool'; payload: ChatToolProgressPayload }
  | { kind: 'status'; payload: { conversationId: string; runId: string; note: string | null } }
  | { kind: 'subagent'; payload: ChatSubagentPayload }
  | { kind: 'userPrompt'; payload: ChatUserPromptPayload }

export type RunDisplayResult = { accepted: boolean; confirmedQueueMessageId?: string }

/** Applies a run-scoped UI event to its live projection. Routing, persistence,
 * queue confirmation and React scheduling remain with the caller. */
export function applyRunDisplayEvent(
  snapshot: ConversationStreamSnapshot,
  event: RunDisplayEvent,
): RunDisplayResult {
  const runId = event.kind === 'subagent' ? event.payload.parentRunId : event.payload.runId
  if (!acceptStreamRun(snapshot, runId)) return { accepted: false }
  if (event.kind === 'status') {
    snapshot.statusNote = event.payload.note
    return { accepted: true }
  }
  if (event.kind === 'subagent') {
    const index = findSubagentToolIndex(snapshot.toolCalls, event.payload)
    if (index < 0) return { accepted: false }
    snapshot.toolCalls = snapshot.toolCalls.map((item, i) => (
      i === index ? mergeSubagentProgress(item, event.payload) : item
    ))
    return { accepted: true }
  }
  if (event.kind === 'userPrompt') {
    const record = userPromptEventToRecord(event.payload)
    snapshot.streaming = true
    snapshot.reasoningStreaming = false
    const index = snapshot.toolCalls.findIndex((item) => item.id === record.id)
    snapshot.toolCalls = index < 0
      ? [...snapshot.toolCalls, record]
      : snapshot.toolCalls.map((item, i) => (i === index ? mergeToolRecord(item, record) : item))
    return { accepted: true }
  }
  const record = toolEventToRecord(event.payload)
  applyToolRecordToSnapshot(snapshot, record)
  const confirmedQueueMessageId = userSteerId(record) ?? userFollowUpId(record)
  return confirmedQueueMessageId ? { accepted: true, confirmedQueueMessageId } : { accepted: true }
}
