import type { ChatStreamPayload } from '../api/tauri'
import { acceptStreamRun, createEmptyStreamSnapshot, type ConversationStreamSnapshot } from './conversationRuns'
import {
  applyStreamDeltaToSnapshot,
  finalizeReasoningDurationOnDone,
  isStreamTerminal,
  streamPayloadToSegment,
  streamReasoningDelta,
  streamTextDelta,
} from './streamApply'

/** One run's display projection. The caller owns scheduling and persistence;
 * this boundary owns which event is allowed to mutate the preview and how
 * text, reasoning, segments and terminal timing are accumulated. */
export function applyConversationStreamEvent(
  snapshot: ConversationStreamSnapshot,
  payload: ChatStreamPayload,
  now = Date.now(),
): boolean {
  if (!acceptStreamRun(snapshot, payload.runId)) return false
  if (payload.messageId) snapshot.messageId = payload.messageId
  if (streamTextDelta(payload) || streamReasoningDelta(payload)) snapshot.statusNote = null
  applyStreamDeltaToSnapshot(snapshot, payload, streamPayloadToSegment(payload), now)
  if (isStreamTerminal(payload)) finalizeReasoningDurationOnDone(snapshot, now)
  return true
}

export function beginRunSnapshot(now = Date.now()): ConversationStreamSnapshot {
  return { ...createEmptyStreamSnapshot(), startedAt: now }
}

export function restoreRunSnapshot(payload: ChatStreamPayload, now = Date.now()): ConversationStreamSnapshot {
  return { ...beginRunSnapshot(now), runId: payload.runId, messageId: payload.messageId }
}
