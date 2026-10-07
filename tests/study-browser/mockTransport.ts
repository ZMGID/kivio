import type { StudyCompletionInput, StudyCompletionResult } from '../../src/api/study'
export type { StudyHistoryMessage } from '../../src/api/study'
declare global { interface Window { __studyTest: { requests: StudyCompletionInput[]; failNext: boolean; hold: boolean; lesson?: 'mit-5b-13' | 'mit-5f-2a'; rawReply?: string } } }
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
  const check = input.systemPrompt.includes('CHECK MODE')
  const hint = input.systemPrompt.includes('HINT MODE')
  const lesson = window.__studyTest.lesson
  const generic = 'Simulated test reply: identify the variable that changes, then explain why that step is valid.'
  // Fixed teaching examples prove interface behavior, not live-model quality.
  const content = window.__studyTest.rawReply ?? (check
    ? lesson === 'mit-5b-13'
      ? '演示检查（非真实模型调用）：第一处问题在第三行，换元时漏掉了系数 $\\frac{1}{3}$。由 $du=3x^2\\,dx$，先把 $x^2\\,dx$ 改写成 $\\frac{1}{3}du$，再继续。'
      : lesson === 'mit-5f-2a'
        ? '演示检查（非真实模型调用）：第三行的加号应为减号。对照 $\\int u\\,dv=uv-\\int v\\,du$，自己修正第三行。'
        : generic
    : hint && lesson === 'mit-5b-13'
      ? '演示提示（非真实模型调用）：题目已经提示 $u=x^3$。下一步先写出 $du$ 与 $x^2\\,dx$ 的关系。'
      : generic)
  const split = Math.max(1, Math.floor(content.length / 2))
  onDelta(content.slice(0, split))
  while (window.__studyTest.hold) await wait()
  await wait()
  onDelta(content.slice(split))
  return { requestId: input.requestId, content }
}
