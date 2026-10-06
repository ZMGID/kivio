import type { StudyCompletionInput, StudyCompletionResult } from '../../src/api/study'
export type { StudyHistoryMessage } from '../../src/api/study'
declare global { interface Window { __studyTest: { requests: StudyCompletionInput[]; failNext: boolean; hold: boolean } } }
window.__studyTest = { requests: [], failNext: false, hold: false }
export async function streamStudyCompletion(input: StudyCompletionInput, onDelta: (delta: string) => void, signal: AbortSignal): Promise<StudyCompletionResult> {
  window.__studyTest.requests.push(structuredClone(input))
  const wait = () => new Promise<void>((resolve, reject) => {
    const timer = window.setTimeout(() => { signal.removeEventListener('abort', cancel); resolve() }, 80)
    const cancel = () => { clearTimeout(timer); reject(new DOMException('Cancelled', 'AbortError')) }
    signal.addEventListener('abort', cancel, { once: true })
    if (signal.aborted) cancel()
  })
  await wait()
  if (window.__studyTest.failNext) { window.__studyTest.failNext = false; throw new Error('Simulated provider unavailable. Try again.') }
  onDelta('Simulated test reply: ')
  while (window.__studyTest.hold) await wait()
  await wait()
  const content = 'Simulated test reply: identify the variable that changes, then explain why that step is valid.'
  onDelta(content.slice('Simulated test reply: '.length))
  return { requestId: input.requestId, content }
}
