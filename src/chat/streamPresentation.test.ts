import { describe, expect, it } from 'vitest'
import type { ChatStreamPayload } from '../api/tauri'
import { createEmptyStreamSnapshot } from './conversationRuns'
import { applyConversationStreamEvent, beginRunSnapshot, restoreRunSnapshot } from './streamPresentation'

const event = (type: string, runId: string, delta?: string): ChatStreamPayload => ({
  type, runId, delta, conversationId: 'c1', messageId: 'm1',
} as ChatStreamPayload)

describe('conversation stream presentation', () => {
  it('binds the first run and leaves a later foreign run unable to overwrite the preview', () => {
    const snapshot = createEmptyStreamSnapshot()
    expect(applyConversationStreamEvent(snapshot, event('text_delta', 'run-new', 'hello'))).toBe(true)
    expect(applyConversationStreamEvent(snapshot, event('text_delta', 'run-old', 'stale'))).toBe(false)
    expect(snapshot.content).toBe('hello')
    expect(snapshot.runId).toBe('run-new')
  })

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
