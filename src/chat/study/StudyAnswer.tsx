import { useState } from 'react'
import { Button } from '../../components/Button'
import { useLang } from '../../components/i18n'
import { ChatMarkdown } from '../ChatMarkdown'
import { parseStudyTeachingResponse } from './studyTeachingResponse'
import type { StudyTurn } from './studyStorage'

/** Keep raw streaming/model output out of the reading surface until its shape is checked. */
export function StudyAnswer({ turn }: { turn: Pick<StudyTurn, 'mode' | 'answer' | 'status'> }) {
  const zh = useLang() === 'zh'
  const text = (cn: string, en: string) => zh ? cn : en
  const [revealed, setRevealed] = useState(false)
  const [showVerification, setShowVerification] = useState(false)
  if (!turn.answer) return null
  const guarded = turn.mode === 'hint' || turn.mode === 'check'
  if (guarded && turn.status === 'streaming') return null
  const markdown = (content: string) => <ChatMarkdown content={content} readOnly />
  const reveal = (content: string, label: string) => <div className="kv-study-answer-reveal">
    <Button size="sm" variant="ghost" aria-expanded={revealed} onClick={() => setRevealed(value => !value)}>{revealed ? text('收起解答', 'Hide response') : label}</Button>
    {revealed && markdown(content)}
  </div>
  if (turn.mode === 'solution') return reveal(turn.answer, text('展开完整解答', 'Reveal full solution'))
  if (!guarded) return markdown(turn.answer)
  const parsed = turn.status === 'complete' ? parseStudyTeachingResponse(turn.answer, turn.mode as 'hint' | 'check') : null
  if (!parsed || parsed.status === 'fallback') return <div className="kv-study-answer-guard">
    <p className="kv-study-muted">{text('这条回复未完成或没有按分步格式返回，可能包含完整答案。已收起原始回复，你可以自行决定是否查看。', 'This reply is incomplete or did not follow the step-by-step format and may contain the full answer. The original response stays hidden until you choose to view it.')}</p>
    {reveal(turn.answer, text('查看原始回复（可能含完整答案）', 'View original reply (may reveal the answer)'))}
  </div>
  const response = parsed.response
  return <div className="kv-study-teaching-response">
    {response.mode === 'hint' ? markdown(response.hint) : <>
      <section aria-label={text('需要注意的一步', 'Step to review')}><h4>{text('需要注意的一步', 'Step to review')}</h4>{markdown(response.firstIssue)}</section>
      <section aria-label={text('自己试下一步', 'Try the next step')}><h4>{text('自己试下一步', 'Try the next step')}</h4>{markdown(response.nextStep)}</section>
      <Button size="sm" variant="ghost" aria-expanded={showVerification} onClick={() => setShowVerification(value => !value)}>{showVerification ? text('收起检查依据', 'Hide verification') : text('查看检查依据', 'View verification')}</Button>
      {showVerification && markdown(response.verification)}
    </>}
    {response.withheldSolution && reveal(response.withheldSolution, text('展开额外解答（可能含完整答案）', 'Reveal extra explanation (may include the answer)'))}
    <small className="kv-study-muted">{text('分步显示不代表结论已被验证，请对照题目核对。', 'Step-by-step display does not certify correctness. Check the reasoning against the problem.')}</small>
  </div>
}
