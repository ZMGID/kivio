import { expect, it } from 'vitest'
import { readingCitation } from './readingContext'

it('resolves only existing PDF pages', () => {
  expect(readingCitation('#reading-page-2', 2)).toBe(2)
  expect(readingCitation('#reading-page-3', 2)).toBeNull()
  expect(readingCitation('https://example.com/#other', 2)).toBeNull()
})

it('keeps explicit source links separate from translation runs and rejects invalid regions', async () => {
  const { readingReference, readingReferenceAnchor } = await import('./readingContext')
  const anchor = { page: 3, text: '', rect: { x: 0.1, y: 0.2, w: 0.3, h: 0.4 }, view: { kind: 'dual' as const, runId: 'run-1' } }
  expect(readingReferenceAnchor(readingReference(anchor))).toEqual(anchor)
  expect(readingReferenceAnchor('#reading-ref?page=2&kind=source')).toEqual({ page: 2, text: '', view: undefined })
  expect(readingReferenceAnchor('#reading-ref?page=2&kind=mono')).toBeNull()
  expect(readingReferenceAnchor('#reading-ref?page=2&kind=source&rect=0,0,NaN,1')).toBeNull()
  expect(readingReferenceAnchor('#reading-ref?page=2&kind=source&rect=0,0,2,1')).toBeNull()
})

it('resolves legacy answers from their preceding source and refuses an unknown old translation', async () => {
  const { readingMessageAnchor } = await import('./readingContext')
  const source = { id: 'source', role: 'user', attachments: [{ name: 'Paper · 译文', content: '[Kivio Study source: hash]\n引用使用 [第 N 页](#reading-ref?page=N&kind=mono&run=first)' }] }
  const messages = [source, { id: 'answer', role: 'assistant' }, { ...source, id: 'later', attachments: [{ ...source.attachments[0], content: source.attachments[0].content.replace('run=first', 'run=second') }] }]
  expect(readingMessageAnchor('#reading-page-3', messages, 'answer')).toMatchObject({ page: 3, view: { kind: 'mono', runId: 'first' } })
  source.attachments[0].content = '[Kivio Study source: hash]\nLegacy source without version'
  expect(readingMessageAnchor('#reading-page-3', messages, 'answer')).toBeNull()
  source.attachments[0].name = 'Paper · 原文'
  expect(readingMessageAnchor('#reading-page-3', messages, 'answer')).toEqual({ page: 3, text: '' })
})
