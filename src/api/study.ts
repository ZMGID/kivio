import { Channel, invoke } from '@tauri-apps/api/core'
import { isTauriRuntime } from './tauri'

export interface StudyHistoryMessage {
  role: 'user' | 'assistant'
  content: string
}

export interface StudyCompletionInput {
  requestId: string
  providerId: string
  model: string
  systemPrompt: string
  userPrompt: string
  history: StudyHistoryMessage[]
  imageDataUrl?: string
}

export interface StudyCompletionResult {
  requestId: string
  content: string
}

/** A channel belongs to one invocation, independently of Lens and the shared Chat protocol. */
export type StudyStreamEvent = {
  requestId: string
  transportId: string
} & ({ type: 'started' } | { type: 'delta'; delta: string })

function abortError(): DOMException {
  return new DOMException('Study request cancelled.', 'AbortError')
}

/** Transport lifecycle only; the Study feature owns prompt rules and persisted results. */
export function streamStudyCompletion(
  input: StudyCompletionInput,
  onDelta: (delta: string) => void,
  signal: AbortSignal,
): Promise<StudyCompletionResult> {
  if (signal.aborted) return Promise.reject(abortError())
  // Other Chat APIs have scripted browser-preview fallbacks. Study must never present those as AI output.
  if (!isTauriRuntime()) return Promise.reject(new Error('Real study answers require the desktop app and a configured model.'))
  const transportId = crypto.randomUUID()
  const request = { ...input, history: input.history.map(message => ({ ...message })), transportId }

  return new Promise((resolve, reject) => {
    let settled = false
    let started = false
    let cancelSent = false
    const channel = new Channel<StudyStreamEvent>()
    const finish = (error?: unknown, result?: StudyCompletionResult) => {
      if (settled) return
      settled = true
      signal.removeEventListener('abort', onAbort)
      if (error !== undefined) reject(error)
      else if (result) resolve(result)
    }
    const cancel = () => {
      // Wait for registration acknowledgement. Cancelling before backend registration could be lost.
      if (!started || cancelSent) return
      cancelSent = true
      void invoke('study_cancel_request', { transportId }).catch(error => {
        // The provider request is additionally bounded by a backend timeout.
        console.warn('Study cancellation could not be delivered', error)
      })
    }
    const onAbort = () => {
      cancel()
      finish(abortError())
    }
    channel.onmessage = event => {
      if (event.requestId !== request.requestId || event.transportId !== transportId) return
      if (event.type === 'started') {
        started = true
        if (signal.aborted || settled) cancel()
        return
      }
      if (!settled && !signal.aborted && event.type === 'delta') {
        try {
          onDelta(event.delta)
        } catch (error) {
          cancel()
          finish(error)
        }
      }
    }
    signal.addEventListener('abort', onAbort, { once: true })
    // Handle a signal aborted while the channel and listener were being installed.
    if (signal.aborted) {
      onAbort()
      return
    }
    void invoke<StudyCompletionResult>('study_request_help', { input: request, channel }).then(result => {
      if (signal.aborted) finish(abortError())
      else if (result.requestId !== request.requestId) finish(new Error('The study response belongs to another request. Please retry.'))
      else finish(undefined, result)
    }, error => {
      finish(signal.aborted ? abortError() : error instanceof Error ? error : new Error(String(error)))
    })
  })
}
