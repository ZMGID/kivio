import { useState } from 'react'
import { Button } from '../../components/Button'
import { useLang } from '../../components/i18n'
import { ChatMarkdown } from '../ChatMarkdown'
import type { StudyTurn } from './studyStorage'
import { readLegacyStudyAnswer } from './studyLegacyAnswer'

/** Ordinary model replies stream directly; full solutions remain an explicit choice. */
export function StudyAnswer({ turn }: { turn: Pick<StudyTurn, 'mode' | 'answer' | 'status'> }) {
  const zh = useLang() === 'zh'
  const [revealed, setRevealed] = useState(false)
  if (!turn.answer) return null
  const legacy = turn.status !== 'streaming' ? readLegacyStudyAnswer(turn.answer) : undefined
  if (turn.mode !== 'solution') return <>
    <ChatMarkdown content={legacy?.visibleText ?? turn.answer} readOnly />
    {legacy?.withheldSolution && <div className="kv-study-answer-reveal"><Button size="sm" variant="ghost" aria-expanded={revealed} onClick={() => setRevealed(value => !value)}>{revealed ? (zh ? '收起解答' : 'Hide response') : (zh ? '展开以前保存的完整解答' : 'Reveal previously saved solution')}</Button>{revealed && <ChatMarkdown content={legacy.withheldSolution} readOnly />}</div>}
  </>
  return <div className="kv-study-answer-reveal">
    <Button size="sm" variant="ghost" aria-expanded={revealed} onClick={() => setRevealed(value => !value)}>{revealed ? (zh ? '收起解答' : 'Hide response') : (zh ? '展开完整解答' : 'Reveal full solution')}</Button>
    {revealed && <ChatMarkdown content={turn.answer} readOnly />}
  </div>
}
