import { describe, expect, it } from 'vitest'
import type { ChatStreamPayload } from '../api/tauri'
import { createEmptyStreamSnapshot } from './conversationRuns'
import { applyConversationStreamEvent, beginRunSnapshot, restoreRunSnapshot } from './streamPresentation'

const event = (type: string, runId: string, delta?: string): ChatStreamPayload => ({
  type, runId, delta, conversationId: 'c1', messageId: 'm1',
} as ChatStreamPayload)

describe('conversation stream presentation', () => {

  it('clears retry status when content resumes and finalizes reasoning duration on terminal', () => {
    const snapshot = createEmptyStreamSnapshot()
    snapshot.statusNote = 'retrying'
    expect(applyConversationStreamEvent(snapshot, event('reasoning_delta', 'run-a', 'thinking'), 100)).toBe(true)
    expect(snapshot.statusNote).toBeNull()
    expect(snapshot.reasoningDurationMs).toBe(0)
    expect(applyConversationStreamEvent(snapshot, event('run_completed', 'run-a'), 145)).toBe(true)
    expect(snapshot.reasoningDurationMs).toBe(45)
    expect(snapshot.messageId).toBe('m1')
  })

  it('rolls back only a failed attempt, keeps completed work and rejects late rollback', () => {
    const snapshot = createEmptyStreamSnapshot()
    snapshot.runId = 'run-a'
    snapshot.content = 'earlier work失败🌱'
    snapshot.reasoning = 'earlier thought残余'
    snapshot.segments = [
      { id: 'earlier', kind: 'text', phase: 'tool_loop', order: 1, text: 'earlier work' },
      { id: 'draft', kind: 'text', phase: 'synthesis', order: 2, text: '失败🌱' },
    ]
    snapshot.toolCalls = [
      { id: 'finished', name: 'read', status: 'success' },
      { id: 'pending', name: 'write', status: 'pending' },
    ]
    const rollback = { ...event('stream_attempt_discarded', 'run-a'), textChars: 3,
      reasoningChars: 2, segmentIds: ['draft'], toolIds: ['pending'] } as ChatStreamPayload
    expect(applyConversationStreamEvent(snapshot, { ...rollback, runId: 'old-run' })).toBe(false)
    expect(snapshot.content).toBe('earlier work失败🌱')
    expect(applyConversationStreamEvent(snapshot, rollback)).toBe(true)
    expect(snapshot.content).toBe('earlier work')
    expect(snapshot.reasoning).toBe('earlier thought')
    expect(snapshot.segments.map((segment) => segment.id)).toEqual(['earlier'])
    expect(snapshot.toolCalls.map((tool) => tool.id)).toEqual(['finished'])
    applyConversationStreamEvent(snapshot, event('text_delta', 'run-a', 'recovered'))
    expect(snapshot.content).toBe('earlier workrecovered')
  })

  it('rejects a late run_started packet after a new run has claimed a preview', () => {
    const snapshot = createEmptyStreamSnapshot()
    snapshot.content = 'new'
    snapshot.runId = 'run-new'
    const next = applyConversationStreamEvent(snapshot, event('run_started', 'run-old'), 200)
    expect(next).toBe(false)
    expect(snapshot.content).toBe('new')
  })

  it('starts a clean outgoing preview and restores a backend run with message identity', () => {
    const outgoing = beginRunSnapshot(200)
    expect(outgoing).toMatchObject({ runId: null, messageId: null, content: '', startedAt: 200 })
    const restored = restoreRunSnapshot(event('run_started', 'run-a'), 300)
    expect(restored).toMatchObject({ runId: 'run-a', messageId: 'm1', content: '', startedAt: 300 })
  })
})
