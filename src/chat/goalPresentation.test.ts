import { describe, expect, it } from 'vitest'
import type { GoalState } from './types'
import { composerGoal, goalCompletedAt } from './goalPresentation'

const goal: GoalState = {
  id: 'g', version: 1, objective: '检查商品', status: 'completed', criteria: [],
  completed_at: 100, completed_message_id: 'result', updated_at: 110,
}
const messages = [
  { id: 'request', role: 'user' as const, timestamp: 90 },
  { id: 'result', role: 'assistant' as const, timestamp: 95 },
]

describe('composerGoal', () => {
  it('keeps the completed receipt until a new user turn, including a same-second send', () => {
    expect(composerGoal(goal, messages)).toBe(goal)
    expect(composerGoal(goal, [...messages, { id: 'next', role: 'user', timestamp: 100 }])).toBeUndefined()
    expect(goal.status).toBe('completed')
  })

  it('does not hide the receipt for assistant summaries or system progress', () => {
    expect(composerGoal(goal, [...messages, { id: 'summary', role: 'assistant', timestamp: 120 }])).toBe(goal)
  })

  it.each(['active', 'verifying', 'waiting', 'paused', 'blocked'] as const)('keeps %s goals during follow-up input', (status) => {
    const current = { ...goal, status }
    expect(composerGoal(current, [...messages, { id: 'next', role: 'user', timestamp: 200 }])).toBe(current)
  })

  it('handles protocol casing and persisted state identically after reopening a window', () => {
    const eventGoal = { ...goal, completed_at: undefined, completed_message_id: undefined,
      completedAt: 100, completedMessageId: 'result' }
    const history = [...messages, { id: 'next', role: 'user' as const, timestamp: 100 }]
    expect(composerGoal(eventGoal, history)).toBeUndefined()
    expect(composerGoal(JSON.parse(JSON.stringify(goal)), history)).toBeUndefined()
  })

  it('supports old completed goals and cancelled goals using their saved end time', () => {
    for (const status of ['completed', 'cancelled'] as const) {
      const legacy = { ...goal, status, completed_at: undefined, completed_message_id: undefined }
      expect(composerGoal(legacy, messages)).toBe(legacy)
      expect(composerGoal(legacy, [...messages, { id: 'next', role: 'user', timestamp: 120 }])).toBeUndefined()
    }
    expect(composerGoal(undefined, messages)).toBeUndefined()
  })

  it('uses completion time independently of later progress updates', () => {
    expect(goalCompletedAt(goal)).toBe(100)
    expect(goalCompletedAt({ ...goal, completed_at: undefined })).toBe(110)
    expect(goalCompletedAt({ ...goal, status: 'active' })).toBeUndefined()
  })
})
