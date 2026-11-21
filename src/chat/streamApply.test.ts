import { expect, it } from 'vitest'
import { upsertStreamSegment } from './streamApply'

it('keeps published segment text intact and reorders a corrected tail step', () => {
  const first = { id: 'first', kind: 'text', phase: 'plain', order: 1, text: 'One' } as const
  const tail = { id: 'tail', kind: 'text', phase: 'plain', order: 2, text: 'Two' } as const
  const published = [first, tail]
  const appended = upsertStreamSegment(published, tail, '!')
  expect(published.map(item => item.text)).toEqual(['One', 'Two'])
  expect(appended.map(item => item.text)).toEqual(['One', 'Two!'])
  const corrected = upsertStreamSegment(appended, { ...tail, order: 0 }, '?')
  expect(corrected.map(item => item.text)).toEqual(['Two!?', 'One'])
})
