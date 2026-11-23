import { describe, expect, it } from 'vitest'
import { findActiveComposerToken, findComposerCommands } from './slashCommands'

const commands = [
  { id: 'plan', slash: '/plan', kind: 'action' },
  { id: 'skill:review', slash: '/review', kind: 'skill' },
  { id: 'cli:claude:foo', slash: '/plugin:foo', kind: 'cli' },
] as const

describe('inline slash commands', () => {
  it('finds queries after Chinese, whitespace and line breaks', () => {
    for (const prefix of ['', 'please ', '请使用', '正文\n']) {
      expect(findActiveComposerToken(`${prefix}/rev`, prefix.length + 4)).toEqual({
        trigger: '/', start: prefix.length, end: prefix.length + 4, query: 'rev',
      })
    }
  })
  it('replaces the entire token when completing inside a word', () => {
    expect(findActiveComposerToken('please /review later', 11)).toEqual({ trigger: '/', start: 7, end: 14, query: 'rev' })
  })
  it('does not open inside links, paths or code', () => {
    for (const value of ['https://host/rev', '/tmp/rev', 'C:/rev', 'user@example.com', 'cost $25', '`@src/file.ts', '> $review', '`/rev', '```\n/rev']) {
      expect(findActiveComposerToken(value, value.length)).toBeNull()
    }
  })
  it('recognizes multiple commands of every kind in the middle of text', () => {
    expect(findComposerCommands('请用/review 检查 /plan 再用 /plugin:foo', commands).map(x => x.command.id))
      .toEqual(['skill:review', 'plan', 'cli:claude:foo'])
  })
  it('requires exact known commands and ignores code, URLs and paths', () => {
    expect(findComposerCommands('/planner /unknown https://host/plan /plan/file `/review`', commands)).toEqual([])
  })
})
