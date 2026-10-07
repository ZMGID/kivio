import { describe, expect, it } from 'vitest'
import { parseStudyTeachingResponse, STUDY_TEACHING_RESPONSE_LIMITS, type StudyTeachingMode } from './studyTeachingResponse'

const hint = { version: 1, mode: 'hint', hint: 'What operation would isolate x?' }
const check = {
  version: 1,
  mode: 'check',
  verification: 'Substitution into the original equation does not reproduce the right-hand side.',
  firstIssue: 'The first rearrangement changes the sign incorrectly.',
  nextStep: 'Apply the same operation to both sides, then check that line again.',
}

describe('Study structured teaching response contract', () => {
  it.each([
    ['hint', hint],
    ['check', check],
    ['hint', { ...hint, withheldSolution: 'An optional answer kept separate from the hint.' }],
    ['check', { ...check, withheldSolution: 'An optional worked solution kept separate from feedback.' }],
  ] as const)('accepts complete v1 %s fields without merging withheld content into feedback', (mode, response) => {
    expect(parseStudyTeachingResponse(JSON.stringify(response), mode)).toEqual({ status: 'valid', response })
  })

  it.each(['json', 'JSON', ''])('accepts a single unambiguous outer %s fence', (language) => {
    expect(parseStudyTeachingResponse(` \n\`\`\`${language}\n${JSON.stringify(check)}\n\`\`\`\n `, 'check'))
      .toEqual({ status: 'valid', response: check })
  })

  it('preserves escaped math, Unicode, Markdown and key-like text inside fields', () => {
    const response = { ...hint, hint: '比较 \\(du\\) 的系数。\n`hint` and "mode": "check" are source text.' }
    expect(parseStudyTeachingResponse(JSON.stringify(response), 'hint')).toEqual({ status: 'valid', response })
  })

  it.each([
    ['plain prose', 'Try another step.'],
    ['empty output', '  \n '],
    ['truncated object', '{"version":1,"mode":"hint","hint":"Try'],
    ['introductory prose', `Here is your hint: ${JSON.stringify(hint)}`],
    ['trailing prose', `${JSON.stringify(hint)} The answer is 2.`],
    ['multiple objects', `${JSON.stringify(hint)}\n${JSON.stringify(hint)}`],
    ['comment', `${JSON.stringify(hint)} // Done`],
    ['fence with trailing prose', `\`\`\`json\n${JSON.stringify(hint)}\n\`\`\`\nThe answer is 2.`],
    ['multiple fences', `\`\`\`json\n${JSON.stringify(hint)}\n\`\`\`\n\`\`\`json\n${JSON.stringify(hint)}\n\`\`\``],
    ['wrong fence language', `\`\`\`javascript\n${JSON.stringify(hint)}\n\`\`\``],
    ['non-string input', hint],
    ['missing input', undefined],
  ])('falls back for %s, with no inferred feedback or raw-output field', (_name, content) => {
    expect(parseStudyTeachingResponse(content, 'hint')).toEqual({ status: 'fallback', reason: 'invalid-json' })
  })

  it.each([
    ['null', null],
    ['array', [hint]],
    ['string', 'A hint'],
    ['boolean', true],
    ['missing version', { mode: 'hint', hint: 'Try a step.' }],
    ['unknown version', { ...hint, version: 2 }],
    ['string version', { ...hint, version: '1' }],
    ['extra key', { ...hint, answer: '2' }],
    ['other mode field', { ...hint, nextStep: 'Try again.' }],
    ['missing hint', { version: 1, mode: 'hint' }],
    ['empty hint', { ...hint, hint: '' }],
    ['whitespace hint', { ...hint, hint: ' \t\n' }],
    ['numeric hint', { ...hint, hint: 1 }],
    ['array hint', { ...hint, hint: ['Try a step.'] }],
    ['nested hint', { ...hint, hint: { text: 'Try a step.' } }],
    ['null withheld solution', { ...hint, withheldSolution: null }],
    ['empty withheld solution', { ...hint, withheldSolution: '' }],
    ['whitespace withheld solution', { ...hint, withheldSolution: '\n\t' }],
    ['object withheld solution', { ...hint, withheldSolution: { answer: '2' } }],
  ])('rejects the invalid %s shape', (_name, response) => {
    expect(parseStudyTeachingResponse(JSON.stringify(response), 'hint')).toEqual({ status: 'fallback', reason: 'invalid-shape' })
  })

  it.each(['verification', 'firstIssue', 'nextStep'])('requires nonempty, bounded string check field %s', (field) => {
    for (const value of [undefined, '', ' \n ', null, 1, false, [], {}, 'x'.repeat(STUDY_TEACHING_RESPONSE_LIMITS.visibleField + 1)]) {
      expect(parseStudyTeachingResponse(JSON.stringify({ ...check, [field]: value }), 'check'))
        .toEqual({ status: 'fallback', reason: 'invalid-shape' })
    }
  })

  it.each([
    ['hint', check],
    ['check', hint],
    ['hint', { ...hint, mode: 'solution' }],
    ['hint', { ...hint, mode: 'explain' }],
    ['hint', { ...hint, mode: 1 }],
    ['hint', { ...hint, mode: undefined }],
  ] as const)('rejects a response that does not match requested %s mode', (mode, response) => {
    expect(parseStudyTeachingResponse(JSON.stringify(response), mode)).toEqual({ status: 'fallback', reason: 'mode-mismatch' })
  })

  it('fails closed for an unsupported requested mode at runtime', () => {
    expect(parseStudyTeachingResponse(JSON.stringify(hint), 'solution' as StudyTeachingMode))
      .toEqual({ status: 'fallback', reason: 'mode-mismatch' })
  })

  it.each([
    '{"version":1,"mode":"hint","hint":"Try a step.","hint":"A replacement."}',
    '{"version":2,"version":1,"mode":"hint","hint":"Try a step."}',
    '{"version":1,"mode":"check","mode":"hint","hint":"Try a step."}',
    '{"version":1,"mode":"hint","hint":"Try a step.","\\u0068int":"An escaped duplicate."}',
    '{"version":1,"mode":"hint","hint":"Try a step.","withheldSolution":"One answer.","withheldSolution":"Another answer."}',
  ])('rejects ambiguous duplicate keys: %s', (content) => {
    expect(parseStudyTeachingResponse(content, 'hint')).toEqual({ status: 'fallback', reason: 'invalid-shape' })
  })

  it('accepts field boundaries and rejects oversized visible or withheld fields without truncating', () => {
    const response = {
      ...hint,
      hint: 'x'.repeat(STUDY_TEACHING_RESPONSE_LIMITS.visibleField),
      withheldSolution: 'y'.repeat(STUDY_TEACHING_RESPONSE_LIMITS.withheldSolution),
    }
    expect(parseStudyTeachingResponse(JSON.stringify(response), 'hint')).toEqual({ status: 'valid', response })
    for (const oversized of [{ ...response, hint: response.hint + 'x' }, { ...response, withheldSolution: response.withheldSolution + 'y' }]) {
      expect(parseStudyTeachingResponse(JSON.stringify(oversized), 'hint')).toEqual({ status: 'fallback', reason: 'invalid-shape' })
    }
  })

  it('bounds the entire raw response before parsing, including whitespace and fences', () => {
    const content = JSON.stringify(hint).padEnd(STUDY_TEACHING_RESPONSE_LIMITS.total)
    expect(parseStudyTeachingResponse(content, 'hint')).toEqual({ status: 'valid', response: hint })
    expect(parseStudyTeachingResponse(content + ' ', 'hint')).toEqual({ status: 'fallback', reason: 'too-long' })
    expect(parseStudyTeachingResponse(`\`\`\`json\n${content}\n\`\`\``, 'hint')).toEqual({ status: 'fallback', reason: 'too-long' })
  })

  it('never upgrades a streamed JSON prefix to displayable structured feedback', () => {
    const content = JSON.stringify({ ...check, withheldSolution: 'An answer must stay hidden.' })
    for (let length = 0; length < content.length; length += 1) {
      expect(parseStudyTeachingResponse(content.slice(0, length), 'check').status).toBe('fallback')
    }
  })
})

// Exact public integral-exercise model responses from the 2026-10-07 probe and retest.
// These are synthetic exercise inputs, not user documents or provider credentials.
describe('observed model prose regressions', () => {
  it.each([
    { name: "contradictory initial verdict", content: "你的换元过程是正确的：\n\n\\[\nu=x^3,\\qquad du=3x^2\\,dx\n\\]\n\n因此\n\n\\[\n\\int \\frac{x^2}{1+x^6}\\,dx\n=\\int \\frac{du}{1+u^2}\n=\\arctan u+C\n=\\arctan(x^3)+C.\n\\]\n\n这里的关键是：分母中的 \\(x^6\\) 正好变成 \\(u^2\\)，而分子中的 \\(x^2\\,dx\\) 正好对应 \\(\\frac13du\\)，所以**不需要额外的 \\(\\frac13\\)**。\n\n可以反向验证：\n\n\\[\n\\frac{d}{dx}\\arctan(x^3)\n=\\frac{3x^2}{1+(x^3)^2}\n=\\frac{3x^2}{1+x^6},\n\\]\n\n这正是原被积函数的 \\(3\\) 倍。等等，这里说明若要验证，应比较：\n\n\\[\n\\frac{d}{dx}\\left[\\arctan(x^3)\\right]\n=\\frac{3x^2}{1+x^6},\n\\]\n\n而原积分的被积函数是 \\(\\frac{x^2}{1+x^6}\\)，因此这两者还差一个 \\(3\\)。这提示前面的换元中应仔细处理 \\(du=3x^2dx\\)，而不是直接把它当作 \\(x^2dx\\)。你的下一步是检查这一处系数。" },
    { name: "full solution after verification-prompt retest", content: "你的换元 \\(u=x^3\\) 是对的，但替换时漏掉了系数 \\(\\frac13\\)。\n\n因为\n\\[\ndu=3x^2\\,dx\\quad\\Rightarrow\\quad x^2\\,dx=\\frac13\\,du,\n\\]\n所以原积分应为\n\\[\n\\int\\frac{x^2}{1+x^6}\\,dx\n=\\frac13\\int\\frac{du}{1+u^2}\n=\\frac13\\arctan(u)+C\n=\\boxed{\\frac13\\arctan(x^3)+C}.\n\\]\n\n检验：你写的 \\(\\arctan(x^3)\\) 求导得到\n\\[\n\\frac{d}{dx}\\arctan(x^3)=\\frac{3x^2}{1+x^6},\n\\]\n比原函数多了 \\(3\\) 倍。因此第一处错误是把 \\(\\frac13\\,du\\) 写成了 \\(du\\)。下次换元时先把 \\(x^2dx\\) 单独写成 \\(\\frac13du\\)，再替换。" },
  ])('quarantines $name instead of guessing feedback sections', ({ content }) => {
    expect(parseStudyTeachingResponse(content, 'check')).toEqual({ status: 'fallback', reason: 'invalid-json' })
  })
})
