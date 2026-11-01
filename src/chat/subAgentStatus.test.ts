import { expect, it } from 'vitest'
import { subAgentStatusLabel } from './subAgentStatus'

it('keeps interrupted executions stopped even when a stop message or partial output exists', () => {
  for (const lang of ['zh', 'en'] as const) {
    const stopped = subAgentStatusLabel({ status: 'interrupted' }, lang)
    expect(stopped).not.toBe(subAgentStatusLabel({ status: 'completed' }, lang))
    expect(stopped).not.toBe(subAgentStatusLabel({ status: 'running' }, lang))
    expect(stopped).not.toBe(subAgentStatusLabel({ status: 'failed' }, lang))
    expect(subAgentStatusLabel({ status: 'interrupted', outputAvailable: false, result: 'stop message' }, lang)).toBe(stopped)
    expect(subAgentStatusLabel({ status: 'interrupted', outputAvailable: true, result: 'Partial findings' }, lang)).toBe(stopped)
  }
})

it('shows execution facts and ignores legacy acceptance judgments', () => {
  const returned = subAgentStatusLabel({ status: 'completed' }, 'zh')
  const failed = subAgentStatusLabel({ status: 'failed' }, 'zh')
  const stopping = subAgentStatusLabel({ status: 'stopping' }, 'zh')
  expect(returned).not.toBe(failed)
  expect(stopping).not.toBe(returned)
  expect(subAgentStatusLabel({ status: 'completed', requiresReview: true }, 'zh')).toBe(returned)
  expect(subAgentStatusLabel({ status: 'failed', error: 'recovered: Report' }, 'zh')).toBe(returned)
  expect(subAgentStatusLabel({ status: 'failed', requiresReview: true }, 'zh')).toBe(failed)
  expect(subAgentStatusLabel({ status: 'failed', resolution: { outcome: 'completed_by_parent' } }, 'zh')).toBe(failed)
  expect(subAgentStatusLabel({ status: 'completed', resolution: { outcome: 'blocked' } }, 'zh')).toBe(returned)
  expect(subAgentStatusLabel({ status: 'stopping', resolution: { outcome: 'blocked' } }, 'zh')).toBe(stopping)
})
