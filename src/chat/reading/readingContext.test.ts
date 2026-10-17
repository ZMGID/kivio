import { describe, expect, it } from 'vitest'
import { readingContext, readingCitation } from './readingContext'

describe('document grounding', () => {
  it('provides page-labelled text and a resolvable citation contract', () => {
    const attachment = readingContext(
      'paper',
      ['Introduction', 'Specific result'],
      'result?',
      2,
    )
    expect(attachment.content).toContain('第 1 页 ---\nIntroduction')
    expect(attachment.content).toContain('第 2 页 ---\nSpecific result')
    expect(attachment.content).toContain('所有可提取文字')
    expect(readingCitation('#reading-page-2', 2)).toBe(2)
    expect(readingCitation('#reading-page-3', 2)).toBeNull()
    expect(readingCitation('https://example.com/#other', 2)).toBeNull()
  })
  it('bounds long context, includes a relevant late page, and discloses excerpts', () => {
    const pages = Array.from({ length: 100 }, (_, index) =>
      index === 98
        ? 'unique_evidence '.repeat(1000)
        : 'background '.repeat(1000),
    )
    const result = readingContext(
      'long paper',
      pages,
      'unique_evidence',
      1,
    ).content!
    expect(result).toContain('第 99 页')
    expect(result).toContain('不代表全文')
    expect(result.length).toBeLessThan(62000)
  })
  it('rejects image-only material instead of sending an ungrounded request', () => {
    expect(() => readingContext('scan', ['', ' '], 'summary', 1)).toThrow(
      '没有可提取的文字',
    )
  })
})
