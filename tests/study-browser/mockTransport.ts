import type { StudyCompletionInput, StudyCompletionResult } from '../../src/api/study'
export type { StudyHistoryMessage } from '../../src/api/study'
// Public-source reading examples are deliberately fixed, visibly simulated replies.
// Amano et al. (2023), CC BY, https://doi.org/10.1371/journal.pbio.3002184.
// They exercise UI/source contracts and are not evidence of model comprehension.
const readingReplies = {
  'amano-abstract': '演示阅读（非真实模型调用）：前两句可译为：英语作为科学界的通用语言，是非英语母语者充分贡献于科学的一大障碍；但很少有研究量化这些语言障碍对他们职业发展的影响。impediment 在这里是“阻碍、障碍”。原文主张：对 908 名环境科学研究者的调查发现，非英语母语者，尤其职业早期的研究者，在用英语开展科研活动时付出更多努力。背景解释：这里说的是这项调查中的负担，不能直接推广成所有人的固定差距。',
  'amano-figure': '演示读图（非真实模型调用）：原图信息：横轴是已发表英文论文数，图注说明采用 log10 变换；A、B 的纵轴是分钟，C、D 是天，E、F 是百分比。粉色、绿色、深蓝色分别表示英语母语、中等和较低熟练度；实线圆点代表高收入组，虚线三角代表中低收入组。阴影表示拟合关系的 95% 置信区间。解释：A 图呈现英语阅读时间、发表经验及语言组别之间的关系；这些关系本身不能证明因果，也不能从图片编造精确效应值或 p 值。',
  'amano-whole-paper': '演示范围说明（非真实模型调用）：我目前只收到第 1 页的摘要选区，不能据此总结整篇论文的全部方法、结果和局限。可先概括当前摘要；要讨论具体方法或局限，请打开相应页面或框选相关段落。',
  'amano-missing-legend': '演示缺失上下文（非真实模型调用）：当前选区只有 A 面板，缺少图例和完整图注，不能仅凭颜色判断组别。请扩大选区以包括图例、横轴说明和图注，再解释各条线与阴影的含义。',
} as const

declare global { interface Window { __studyTest: { requests: StudyCompletionInput[]; failNext: boolean; hold: boolean; lesson?: 'mit-5b-13' | 'mit-5f-2a' | keyof typeof readingReplies; rawReply?: string } } }
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
  const read = input.systemPrompt.includes('READ MODE')
  const lesson = window.__studyTest.lesson
  const generic = 'Simulated test reply: identify the variable that changes, then explain why that step is valid.'
  // Fixed teaching examples prove interface behavior, not live-model quality.
  const readingReply = read && lesson && lesson in readingReplies ? readingReplies[lesson as keyof typeof readingReplies] : undefined
  const content = window.__studyTest.rawReply ?? readingReply ?? (check
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
