import { describe, expect, it } from 'vitest'
import { subagentProgress } from './subagents'

describe('subagent metric boundary', () => {
  it('retains valid metrics without confusing cumulative token usage and current context', () => {
    const progress = { tokens: 10000, contextTokens: 2000, contextWindow: 32000, requests: 3, cost: 0.1, recentOutput: ['read source'] }
    expect(subagentProgress(progress)).toEqual(progress)
  })
  it.each([null, [], 'metrics', { requests: '3' }, { contextTokens: -1 }, { recentOutput: [false] }, { steps: [1] }, { recentTools: [{ tool: 'read', args: {}, endMs: 1 }] }])('rejects invalid metric payload %j', value => {
    expect(subagentProgress(value)).toBeNull()
  })
})
