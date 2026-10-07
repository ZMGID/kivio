import type { StudyCompletionInput, StudyCompletionResult } from '../../src/api/study'
export type { StudyHistoryMessage } from '../../src/api/study'
declare global { interface Window { __studyTest: { requests: StudyCompletionInput[]; failNext: boolean; hold: boolean; lesson?: 'mit-5b-13' } } }
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
  // Fixed, independently checked teaching examples for one public exercise.
  // These prove UI/request handling only, never a real model's grading ability.
  const lesson = input.systemPrompt.includes('CHECK MODE')
    ? '演示检查（非真实模型调用）：前两行换元和求微分正确。第一处问题在第三行：由 $du=3x^2\\,dx$ 可知 $x^2\\,dx=\\frac{1}{3}du$，换元时漏掉了系数 $\\frac{1}{3}$。请先把这个系数补回新积分，再继续算。'
    : '演示提示（非真实模型调用）：题目已经提示 $u=x^3$。下一步先写出 $du$ 与 $x^2\\,dx$ 的关系；换元时，分子和微分应一起替换。'
  const content = window.__studyTest.lesson === 'mit-5b-13'
    ? `Simulated test reply: ${lesson}`
    : 'Simulated test reply: identify the variable that changes, then explain why that step is valid.'
  onDelta(content.slice('Simulated test reply: '.length))
  return { requestId: input.requestId, content }
}
