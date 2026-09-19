import { describe, expect, it } from 'vitest'
import { createEmptyStreamSnapshot } from './conversationRuns'
import { applyRunDisplayEvent } from './runDisplayEvents'
import type { ChatSubagentPayload, ChatToolProgressPayload, ChatUserPromptPayload } from '../api/tauri'

const tool = (runId: string, status: string, structuredContent?: object): ChatToolProgressPayload => ({
  id: 'tool-1', toolCallId: 'tool-1', conversationId: 'c1', runId,
  messageId: 'm1', name: 'ask_user', source: 'native', status,
  argumentsPreview: '{}', round: 1, sensitive: false, artifacts: [], structuredContent,
} as ChatToolProgressPayload)

describe('run display events', () => {
  it('retains a tool card payload when a later result update omits it', () => {
    const snapshot = createEmptyStreamSnapshot()
    applyRunDisplayEvent(snapshot, { kind: 'tool', payload: tool('run-a', 'running', { askUser: { title: 'Question' } }) })
    applyRunDisplayEvent(snapshot, { kind: 'tool', payload: tool('run-a', 'success') })
    expect(snapshot.toolCalls[0].structuredContent).toEqual({ askUser: { title: 'Question' } })
    expect(snapshot.toolCalls[0].status).toBe('completed')
  })

  it('drops a late tool event from a different run without changing the preview', () => {
    const snapshot = createEmptyStreamSnapshot()
    applyRunDisplayEvent(snapshot, { kind: 'tool', payload: tool('run-new', 'running') })
    expect(applyRunDisplayEvent(snapshot, { kind: 'tool', payload: tool('run-old', 'success') }).accepted).toBe(false)
    expect(snapshot.toolCalls[0].status).toBe('running')
    expect(snapshot.runId).toBe('run-new')
  })

  it('keeps status notes scoped to the run that owns the preview', () => {
    const snapshot = createEmptyStreamSnapshot()
    snapshot.runId = 'run-new'
    expect(applyRunDisplayEvent(snapshot, { kind: 'status', payload: { conversationId: 'c1', runId: 'run-old', note: 'stale' } }).accepted).toBe(false)
    expect(snapshot.statusNote).toBeNull()
    expect(applyRunDisplayEvent(snapshot, { kind: 'status', payload: { conversationId: 'c1', runId: 'run-new', note: 'retrying' } }).accepted).toBe(true)
    expect(snapshot.statusNote).toBe('retrying')
  })

  it('adds a user prompt only to its run preview', () => {
    const snapshot = createEmptyStreamSnapshot()
    const payload = {
      conversationId: 'c1', runId: 'run-a', messageId: 'm1', toolCallId: 'ask-1',
      name: 'ask_user', source: 'native', prompt: { title: 'Choose', questions: [] },
    } as ChatUserPromptPayload
    applyRunDisplayEvent(snapshot, { kind: 'userPrompt', payload })
    applyRunDisplayEvent(snapshot, { kind: 'userPrompt', payload })
    expect(snapshot.toolCalls).toHaveLength(1)
    expect(snapshot.toolCalls[0].structuredContent).toMatchObject({ askUser: { title: 'Choose' } })
  })

  it('updates only the matched parent subagent tool card', () => {
    const snapshot = createEmptyStreamSnapshot()
    snapshot.runId = 'run-a'
    snapshot.toolCalls = [{ id: 'parent-1', name: 'Agent', status: 'running' } as never]
    const payload = {
      parentConversationId: 'c1', parentRunId: 'run-a', parentToolCallId: 'parent-1',
      taskId: 'task-1', name: 'worker', depth: 1, status: 'running', preview: 'step',
    } as ChatSubagentPayload
    expect(applyRunDisplayEvent(snapshot, { kind: 'subagent', payload }).accepted).toBe(true)
    expect(snapshot.toolCalls[0].structuredContent).toMatchObject({ subagentProgress: { preview: 'step' } })
  })
})
