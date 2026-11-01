import { describe, expect, it } from 'vitest'
import { formatToolResultPreview } from './toolResultPreview'

describe('formatToolResultPreview', () => {
  it('returns empty string for blank input', () => {
    expect(formatToolResultPreview('')).toBe('')
    expect(formatToolResultPreview('   ')).toBe('')
  })

  it('compacts plain text previews', () => {
    expect(formatToolResultPreview('  hello\n   world  ')).toBe('hello world')
  })

  it('extracts Tavily answer from JSON stdout', () => {
    const raw = JSON.stringify({ answer: 'Paris is the capital of France.' })
    expect(formatToolResultPreview(raw)).toContain('Paris is the capital')
  })

  it('summarizes Tavily search results', () => {
    const raw = JSON.stringify({
      query: 'kivio',
      results: [{ title: 'Kivio Docs', content: 'Screen-level AI assistant', url: 'https://example.com' }],
    })
    const preview = formatToolResultPreview(raw)
    expect(preview).toContain('1')
    expect(preview).toContain('Kivio Docs')
  })

  it('handles stdout: prefixed JSON bodies', () => {
    const raw = `stdout: ${JSON.stringify({ answer: 'done' })}`
    expect(formatToolResultPreview(raw)).toContain('done')
  })
})
