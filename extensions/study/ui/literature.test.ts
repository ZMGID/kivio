import { describe, expect, it } from 'vitest'
import { emptyBibliography, type ReadingDocument } from '../api/reading'
import { filterLiterature, mergeBibliography, literatureLayout } from './literature'
const base: ReadingDocument = {
  id: 'one',
  name: 'Attention research',
  conversationIds: ['chat'],
  activeConversationId: 'chat',
  page: 4,
  offset: 0,
  notes: [{ id: 'n', content: 'variance explanation', kind: 'note' }],
  noteDraft: '',
  tags: ['AI', 'theory'],
  collectionIds: ['a', 'b'],
  status: 'reading',
}
describe('literature browsing', () => {
  it('keeps one reference in multiple collections and finds its notes with intersecting tags', () => {
    const collections = [
      { id: 'a', name: 'Project', parentId: null },
      { id: 'b', name: 'Topic', parentId: null },
    ]
    expect(
      filterLiterature([base], collections, 'collection:a', 'variance', [
        'AI',
        'theory',
      ]).map((d) => d.id),
    ).toEqual(['one'])
    expect(
      filterLiterature([base], collections, 'collection:b', '', [
        'AI',
        'missing',
      ]),
    ).toEqual([])
    expect(filterLiterature([base], collections, 'all', '', [], true)).toEqual([])
    expect(
      filterLiterature(
        [base],
        collections.map((c) => ({ ...c, deleted: true })),
        'all',
        '',
        [],
        true,
      ).map((d) => d.id),
    ).toEqual(['one'])
  })
  it('isolates trash and preserves searchability after restore', () => {
    const trashed = { ...base, trashedAt: '2026-10-18' }
    expect(filterLiterature([trashed], [], 'all', '', [])).toEqual([])
    expect(
      filterLiterature([trashed], [], 'trash', 'variance', [])[0]
        .conversationIds,
    ).toEqual(['chat'])
    expect(
      filterLiterature(
        [{ ...trashed, trashedAt: null }],
        [],
        'all',
        '',
        [],
      )[0].page,
    ).toBe(4)
  })
})

it('retains manually completed fields when a DOI response omits them', () => {
  const result = mergeBibliography(
    {
      ...emptyBibliography,
      abstractText: 'My reviewed summary',
      publication: 'Collected papers',
      year: '2024',
      authors: ['Author'],
    },
    { ...emptyBibliography, doi: '10.1000/paper' },
  )
  expect(result.abstractText).toBe('My reviewed summary')
  expect(result.publication).toBe('Collected papers')
  expect(result.year).toBe('2024')
  expect(result.authors).toEqual(['Author'])
  expect(result.doi).toBe('10.1000/paper')
})

it('reserves readable list width even when both side panels request their maximum', () => {
  for (const width of [600, 700, 899, 900, 960, 1100, 1440]) {
    const layout = literatureLayout(width, 1000, 1000)
    const occupied = (layout.showNavigation ? layout.navigationWidth + 6 : 0)
      + (layout.showInspector ? layout.inspectorWidth + 6 : 0)
    expect(width - occupied).toBeGreaterThanOrEqual(360)
    expect(layout.navigationWidth).toBeGreaterThanOrEqual(200)
    expect(layout.inspectorWidth).toBeGreaterThanOrEqual(280)
  }
  expect(literatureLayout(500).showNavigation).toBe(false)
  expect(literatureLayout(899).showInspector).toBe(false)
})
