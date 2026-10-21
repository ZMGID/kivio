import { describe, expect, it } from 'vitest'
import { formatCitations, type Cluster, type Reference } from './citations'
const references: Reference[] = [
  { id: 'a', name: 'Alpha study', bibliography: { authors: ['Smith, Ada'], year: '2020', publication: 'Journal', doi: '', url: '', abstractText: '', itemType: 'article', volume: '12', issue: '2', pages: '10–20' } },
  { id: 'b', name: 'Beta study', bibliography: { authors: ['Chen, Li'], year: '2022', publication: 'Science', doi: '', url: '', abstractText: '', itemType: 'article' } },
]
const cluster = (citationID: string, id: string): Cluster => ({ citationID, citationItems: [{ id }], properties: { noteIndex: 0 } })
const plain = (text: string) => text.replace(/<[^>]*>/g, '')
describe('Word citation formatting with the packaged CSL processor', () => {
  it('renumbers by current document order after moving or deleting citations and deduplicates bibliography', () => {
    const original = formatCitations(references, [cluster('1', 'a'), cluster('2', 'b'), cluster('3', 'a')], 'ieee')
    expect(plain(original.citations.get('1')!)).toBe('[1]')
    expect(plain(original.citations.get('2')!)).toBe('[2]')
    expect(plain(original.citations.get('3')!)).toBe('[1]')
    const moved = formatCitations(references, [cluster('2', 'b'), cluster('1', 'a')], 'ieee')
    expect(plain(moved.citations.get('2')!)).toBe('[1]')
    expect(moved.bibliography.indexOf('Beta study')).toBeLessThan(moved.bibliography.indexOf('Alpha study'))
    const deleted = formatCitations(references, [cluster('2', 'b')], 'ieee')
    expect(plain(deleted.citations.get('2')!)).toBe('[1]')
    expect(deleted.bibliography).not.toContain('Alpha study')
  })
  it('uses real APA formatting, page locators and updated metadata on reload', () => {
    const result = formatCitations(references, [{ ...cluster('1', 'a'), citationItems: [{ id: 'a', locator: '12', label: 'page' }] }], 'apa')
    expect(plain(result.citations.get('1')!)).toBe('(Smith, 2020, p. 12)')
    expect(plain(result.bibliography)).toContain('Smith, A.')
    const changed = structuredClone(references); changed[0].name = 'Corrected title'
    expect(formatCitations(changed, [cluster('1', 'a')], 'apa').bibliography).toContain('Corrected title')
  })
  it('renders Chinese bibliographies and escapes metadata rather than injecting HTML', () => {
    const items = structuredClone(references); items[0].name = '<img src=x onerror=alert(1)> 中文研究'
    const result = formatCitations(items, [cluster('1', 'a')], 'gbt')
    expect(result.bibliography).toContain('中文研究')
    expect(result.bibliography).not.toContain('<img')
    expect(plain(result.citations.get('1')!)).toContain('1')
  })
  it('rejects a reference missing from both live library and document snapshot', () => {
    expect(() => formatCitations([], [cluster('1', 'a')], 'apa')).toThrow('引用资料缺失')
  })
})
