import type { ConversationStreamSnapshot } from './conversationRuns'
import { patchSnapshot, setCoarse } from './streamingStore'

export function isLocallyCancelledPayload(
  payload: { conversationId: string; runId?: string; type?: string },
  cancelledConversationId: string | null,
  cancelledRunId: string | null,
): boolean {
  // Cancellation suppresses late content, never the authoritative end of a run.
  if (payload.type === 'run_cancelled' || payload.type === 'run_completed' || payload.type === 'run_failed') return false
  if (cancelledConversationId !== payload.conversationId) return false
  return !cancelledRunId || !payload.runId || payload.runId === cancelledRunId
}

export function freezeCancelledStream(
  snapshot: ConversationStreamSnapshot | undefined,
  cancelPendingFrame: () => void,
): void {
  cancelPendingFrame()
  if (snapshot) {
    snapshot.streaming = false
    snapshot.reasoningStreaming = false
  }
  setCoarse({ streaming: false, streamFrozen: true })
  patchSnapshot({ streaming: false, reasoningStreaming: false })
}
