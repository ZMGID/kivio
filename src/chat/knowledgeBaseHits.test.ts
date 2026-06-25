import { describe, expect, it } from 'vitest'
import { knowledgeSearchHits } from './knowledgeBaseHits'
import type { ToolCallRecord } from './types'

function record(partial: Partial<ToolCallRecord>): ToolCallRecord {
  return { tool_name: 'knowledge_search', status: 'success', ...partial } as ToolCallRecord
}

describe('knowledgeSearchHits', () => {
  it('returns null for non-knowledge tools', () => {
    expect(knowledgeSearchHits(record({ tool_name: 'web_search' }))).toBeNull()
  })

  it('parses camelCase structured hits', () => {
    const hits = knowledgeSearchHits(
      record({
        structured_content: {
          hits: [
            { n: 1, docName: 'a.md', headingPath: 'Intro > Setup', score: 0.91, text: 'install it' },
            { n: 2, docName: 'b.txt', score: 0.4, text: 'use it' },
          ],
        },
      })
    )
    expect(hits).toHaveLength(2)
    expect(hits![0]).toMatchObject({ n: 1, docName: 'a.md', headingPath: 'Intro > Setup', text: 'install it' })
    expect(hits![1].headingPath).toBeUndefined()
  })

  it('falls back to snake_case keys', () => {
    const hits = knowledgeSearchHits(
      record({ structured_content: { hits: [{ n: 3, doc_name: 'c.pdf', heading_path: 'Ch 1', score: 0.7, text: 'x' }] } })
    )
    expect(hits![0]).toMatchObject({ docName: 'c.pdf', headingPath: 'Ch 1' })
  })

  it('drops hits without text and returns null when none remain', () => {
    expect(
      knowledgeSearchHits(record({ structured_content: { hits: [{ n: 1, docName: 'a', score: 1 }] } }))
    ).toBeNull()
    expect(knowledgeSearchHits(record({ structured_content: { hits: [] } }))).toBeNull()
    expect(knowledgeSearchHits(record({}))).toBeNull()
  })
})
