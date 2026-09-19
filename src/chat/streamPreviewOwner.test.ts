import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { ChatStreamPayload, ChatToolProgressPayload } from '../api/tauri'
import { beginGroup, getActiveGroup, resetGroups } from './groupStreamingStore'
import { getCoarse, getSnapshot, reset as resetStreamStore } from './streamingStore'
import { createStreamPreviewOwner } from './streamPreviewOwner'
import type { ChatMessage } from './types'

const packet = (conversationId: string, runId: string, type: string, delta?: string): ChatStreamPayload => ({
  type, conversationId, runId, messageId: `${conversationId}-assistant`, delta,
} as ChatStreamPayload)

describe('stream preview owner', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) =>
      setTimeout(() => callback(performance.now()), 0) as unknown as number)
    vi.stubGlobal('cancelAnimationFrame', (handle: number) => clearTimeout(handle))
    resetStreamStore()
    resetGroups()
  })

  afterEach(() => {
    resetGroups()
    resetStreamStore()
    vi.unstubAllGlobals()
    vi.useRealTimers()
  })

  it('publishes the local start time before the first backend packet', () => {
    const owner = createStreamPreviewOwner()
    owner.activate('a')
    owner.begin('a', 1234)
    expect(getCoarse().streaming).toBe(true)
    expect(getSnapshot().startedAt).toBe(1234)
    owner.dispose()
  })

  it('keeps background deltas and stale frames out of the selected conversation', () => {
    const owner = createStreamPreviewOwner()
    owner.activate('a')
    owner.begin('a', 100)
    owner.receive(packet('a', 'run-a', 'text_delta', 'hello'))
    owner.activate('b')
    owner.begin('b', 101)
    owner.receive(packet('a', 'run-a', 'text_delta', ' world'))
    owner.receive(packet('b', 'run-b', 'text_delta', 'fresh'))
    vi.runAllTimers()
    expect(getSnapshot().content).toBe('fresh')
    owner.activate('a')
    expect(getSnapshot().content).toBe('hello world')
    owner.dispose()
  })

  it('rejects a late packet from an older run after a new preview begins', () => {
    const owner = createStreamPreviewOwner()
    owner.activate('a')
    owner.begin('a', 100)
    owner.receive(packet('a', 'old', 'text_delta', 'old'))
    owner.begin('a', 200)
    owner.receive(packet('a', 'new', 'text_delta', 'new'))
    expect(owner.receive(packet('a', 'old', 'text_delta', 'late')).accepted).toBe(false)
    vi.runAllTimers()
    expect(getSnapshot().content).toBe('new')
    owner.dispose()
  })

  it('restores a new backend run over an old frozen failed preview', () => {
    const owner = createStreamPreviewOwner()
    owner.activate('a')
    owner.begin('a', 100)
    owner.receive(packet('a', 'old', 'text_delta', 'partial'))
    owner.complete('a', { kind: 'error' })
    expect(owner.receive(packet('a', 'new', 'run_started')).accepted).toBe(true)
    owner.receive(packet('a', 'new', 'text_delta', 'fresh'))
    vi.runAllTimers()
    expect(getSnapshot().content).toBe('fresh')
    expect(getCoarse().streamFrozen).toBe(false)
    owner.dispose()
  })

  it('holds the visible answer until its committed twin appears, then clears it', () => {
    const owner = createStreamPreviewOwner()
    owner.activate('a')
    owner.begin('a', 100)
    owner.receive(packet('a', 'run-a', 'text_delta', 'answer'))
    owner.receive(packet('a', 'run-a', 'run_completed'))
    const oldMessages: ChatMessage[] = []
    owner.complete('a', { kind: 'persisted', committedMessages: oldMessages })
    expect(getSnapshot().content).toBe('answer')
    expect(getCoarse()).toMatchObject({ streaming: false, streamFrozen: true })
    owner.reconcile('a', oldMessages)
    expect(getSnapshot().content).toBe('answer')
    owner.reconcile('a', [{ id: 'a-assistant', role: 'assistant', content: 'answer', timestamp: 1 }])
    expect(getSnapshot().content).toBe('')
    expect(getCoarse().streamFrozen).toBe(false)
    owner.dispose()
  })

  it('uses a bounded fallback when a persisted answer never appears in React', () => {
    const owner = createStreamPreviewOwner()
    owner.activate('a')
    owner.begin('a', 100)
    owner.receive(packet('a', 'run-a', 'text_delta', 'answer'))
    owner.complete('a', { kind: 'persisted', committedMessages: [] })
    vi.advanceTimersByTime(1_499)
    expect(getSnapshot().content).toBe('answer')
    vi.advanceTimersByTime(1)
    expect(getSnapshot().content).toBe('')
    owner.dispose()
  })

  it('uses the committed messages reference when the stream had no message ID', () => {
    const owner = createStreamPreviewOwner()
    owner.activate('a')
    owner.begin('a', 100)
    owner.receive({ ...packet('a', 'run-a', 'text_delta', 'answer'), messageId: '' })
    const oldMessages: ChatMessage[] = []
    owner.complete('a', { kind: 'persisted', committedMessages: oldMessages })
    owner.reconcile('a', oldMessages)
    expect(getSnapshot().content).toBe('answer')
    owner.reconcile('a', [])
    expect(getSnapshot().content).toBe('')
    owner.dispose()
  })

  it('resumes a frozen preview when backend cancellation is rejected', () => {
    const owner = createStreamPreviewOwner()
    owner.activate('a')
    owner.begin('a', 100)
    owner.receive(packet('a', 'run-a', 'text_delta', 'partial'))
    expect(owner.freezeForCancellation('a')).toBe(true)
    expect(getCoarse()).toMatchObject({ streaming: false, streamFrozen: true })
    expect(owner.resume('a')).toBe(true)
    expect(getCoarse()).toMatchObject({ streaming: true, streamFrozen: false })
    owner.receive(packet('a', 'run-a', 'text_delta', ' answer'))
    vi.runAllTimers()
    expect(getSnapshot().content).toBe('partial answer')
    owner.dispose()
  })

  it('reopens an empty single preview when cancellation fails without another delta', () => {
    const owner = createStreamPreviewOwner()
    owner.activate('a')
    owner.begin('a', 100)
    expect(owner.freezeForCancellation('a')).toBe(true)
    expect(getCoarse()).toMatchObject({ streaming: false, streamFrozen: false })
    expect(owner.resume('a')).toBe(true)
    expect(getCoarse()).toMatchObject({ streaming: true, streamFrozen: false })
    owner.dispose()
  })

  it('does not let an old twin timeout erase a newer run', () => {
    const owner = createStreamPreviewOwner()
    owner.activate('a')
    owner.begin('a', 100)
    owner.receive(packet('a', 'old', 'text_delta', 'old'))
    owner.complete('a', { kind: 'persisted', committedMessages: [] })
    owner.begin('a', 200)
    owner.receive(packet('a', 'new', 'text_delta', 'new'))
    vi.advanceTimersByTime(2_000)
    expect(getSnapshot().content).toBe('new')
    expect(getCoarse().streaming).toBe(true)
    owner.dispose()
  })

  it('preserves a frozen twin when the same conversation is reactivated before commit', () => {
    const owner = createStreamPreviewOwner()
    owner.activate('a')
    owner.begin('a', 100)
    owner.receive(packet('a', 'run-a', 'text_delta', 'answer'))
    owner.complete('a', { kind: 'persisted', committedMessages: [] })
    owner.activate('a')
    expect(getSnapshot().content).toBe('answer')
    expect(getCoarse().streamFrozen).toBe(true)
    owner.dispose()
  })

  it('clears an empty failed preview instead of leaving a busy placeholder', () => {
    const owner = createStreamPreviewOwner()
    owner.activate('a')
    owner.begin('a', 100)
    owner.complete('a', { kind: 'error' })
    expect(getCoarse()).toMatchObject({ streaming: false, streamFrozen: false })
    owner.dispose()
  })

  it('projects fan-out columns without deciding group execution or ending the group', () => {
    const owner = createStreamPreviewOwner()
    owner.activate('a')
    beginGroup('a', 'group-a', [{ providerId: 'p1', model: 'm1' }, { providerId: 'p2', model: 'm2' }])
    const first = owner.receive(packet('a', 'run-1', 'text_delta', 'first'))
    expect(first).toMatchObject({ accepted: true, target: 'group' })
    const done = owner.receive(packet('a', 'run-1', 'run_completed'))
    expect(done).toMatchObject({ accepted: true, target: 'group', terminal: true })
    expect(getActiveGroup('a')?.columns[0].content).toBe('first')
    expect(getActiveGroup('a')?.columns[0].streaming).toBe(false)
    expect(getActiveGroup('a')).toBeDefined()
    owner.dispose()
  })

  it('starts a fan-out view without creating a stray single-answer preview', () => {
    const owner = createStreamPreviewOwner()
    owner.activate('a')
    beginGroup('a', 'group-a', [{ providerId: 'p1', model: 'm1' }, { providerId: 'p2', model: 'm2' }])
    owner.begin('a', 100, 'group')
    expect(owner.timing('a')).toMatchObject({ startedAt: 100, content: '', reasoning: '' })
    expect(getCoarse().streaming).toBe(true)
    expect(getSnapshot().content).toBe('')
    owner.receive(packet('a', 'run-1', 'text_delta', 'first'))
    expect(getSnapshot().content).toBe('')
    expect(getActiveGroup('a')?.columns[0].content).toBe('first')
    owner.dispose()
  })

  it('restores an active fan-out view after navigating away and back', () => {
    const owner = createStreamPreviewOwner()
    beginGroup('a', 'group-a', [{ providerId: 'p1', model: 'm1' }, { providerId: 'p2', model: 'm2' }])
    owner.activate('a')
    owner.begin('a', 100, 'group')
    owner.receive(packet('a', 'run-1', 'text_delta', 'first'))
    owner.activate(null)
    owner.activate('a')
    expect(getCoarse().streaming).toBe(true)
    expect(getActiveGroup('a')?.columns[0].content).toBe('first')
    owner.dispose()
  })

  it('freezes every visible fan-out arm locally on cancel without ending the group', () => {
    const owner = createStreamPreviewOwner()
    owner.activate('a')
    beginGroup('a', 'group-a', [{ providerId: 'p1', model: 'm1' }, { providerId: 'p2', model: 'm2' }])
    owner.begin('a', 100, 'group')
    owner.receive(packet('a', 'run-1', 'text_delta', 'first'))
    expect(owner.freezeForCancellation('a')).toBe(true)
    expect(getActiveGroup('a')?.columns.every((column) => !column.streaming)).toBe(true)
    expect(getCoarse()).toMatchObject({ streaming: false, streamFrozen: true })
    expect(getActiveGroup('a')).toBeDefined()
    owner.dispose()
  })

  it('reopens running fan-out arms when cancellation fails without another delta', () => {
    const owner = createStreamPreviewOwner()
    owner.activate('a')
    beginGroup('a', 'group-a', [{ providerId: 'p1', model: 'm1' }, { providerId: 'p2', model: 'm2' }])
    owner.begin('a', 100, 'group')
    owner.receive(packet('a', 'run-1', 'text_delta', 'first'))
    owner.receive({ ...packet('a', 'run-2', 'text_delta', 'second'), messageId: 'a-second' })
    expect(owner.freezeForCancellation('a')).toBe(true)
    expect(getActiveGroup('a')?.columns.every((column) => !column.streaming)).toBe(true)
    expect(owner.resume('a')).toBe(true)
    expect(getActiveGroup('a')?.columns.every((column) => column.streaming)).toBe(true)
    expect(getCoarse()).toMatchObject({ streaming: true, streamFrozen: false })
    owner.dispose()
  })

  it('does not revive a fan-out arm that terminates while cancellation is pending', () => {
    const owner = createStreamPreviewOwner()
    owner.activate('a')
    beginGroup('a', 'group-a', [{ providerId: 'p1', model: 'm1' }, { providerId: 'p2', model: 'm2' }])
    owner.begin('a', 100, 'group')
    owner.receive(packet('a', 'run-1', 'text_delta', 'first'))
    owner.receive({ ...packet('a', 'run-2', 'text_delta', 'second'), messageId: 'a-second' })
    expect(owner.freezeForCancellation('a')).toBe(true)
    owner.receive(packet('a', 'run-1', 'run_completed'))
    expect(owner.resume('a')).toBe(true)
    expect(getActiveGroup('a')?.columns[0].streaming).toBe(false)
    expect(getActiveGroup('a')?.columns[1].streaming).toBe(true)
    expect(getCoarse()).toMatchObject({ streaming: true, streamFrozen: false })
    owner.dispose()
  })

  it('does not reopen a fan-out group when all arms terminated during cancellation', () => {
    const owner = createStreamPreviewOwner()
    owner.activate('a')
    beginGroup('a', 'group-a', [{ providerId: 'p1', model: 'm1' }, { providerId: 'p2', model: 'm2' }])
    owner.begin('a', 100, 'group')
    owner.receive(packet('a', 'run-1', 'text_delta', 'first'))
    owner.receive({ ...packet('a', 'run-2', 'text_delta', 'second'), messageId: 'a-second' })
    owner.freezeForCancellation('a')
    owner.receive(packet('a', 'run-1', 'run_completed'))
    owner.receive({ ...packet('a', 'run-2', 'run_completed'), messageId: 'a-second' })
    expect(owner.resume('a')).toBe(false)
    expect(getActiveGroup('a')?.columns.every((column) => !column.streaming)).toBe(true)
    expect(getCoarse().streaming).toBe(false)
    owner.dispose()
  })

  it('does not revive an old fan-out group after a new group takes its place', () => {
    const owner = createStreamPreviewOwner()
    owner.activate('a')
    beginGroup('a', 'group-a', [{ providerId: 'p1', model: 'm1' }])
    owner.begin('a', 100, 'group')
    owner.receive(packet('a', 'run-1', 'text_delta', 'first'))
    owner.freezeForCancellation('a')
    beginGroup('a', 'group-b', [{ providerId: 'p2', model: 'm2', streaming: false }])
    expect(owner.resume('a')).toBe(false)
    expect(getActiveGroup('a')?.groupId).toBe('group-b')
    expect(getActiveGroup('a')?.columns[0].streaming).toBe(false)
    owner.dispose()
  })

  it('routes a fan-out tool card to its column and returns queue confirmation once', () => {
    const owner = createStreamPreviewOwner()
    beginGroup('a', 'group-a', [{ providerId: 'p1', model: 'm1' }, { providerId: 'p2', model: 'm2' }])
    const payload = {
      id: 'tool-1', toolCallId: 'tool-1', conversationId: 'a', runId: 'run-1',
      messageId: 'a-assistant', name: 'user_steer', source: 'native', status: 'success',
      argumentsPreview: '{}', round: 1, sensitive: false, artifacts: [],
      structuredContent: { type: 'user_steer', steer_id: 'queued-1', text: 'continue' },
    } as ChatToolProgressPayload
    const result = owner.projectDisplay({ kind: 'tool', payload })
    expect(result).toMatchObject({ accepted: true, confirmedQueueMessageId: 'queued-1' })
    expect(getActiveGroup('a')?.columns[0].toolCalls).toHaveLength(1)
    expect(getSnapshot().content).toBe('')
    owner.dispose()
  })

  it('disposal cancels presentation callbacks without ending a backend run', () => {
    const owner = createStreamPreviewOwner()
    owner.activate('a')
    owner.begin('a', 100)
    owner.receive(packet('a', 'run-a', 'text_delta', 'hello'))
    const visibleBefore = getSnapshot()
    owner.dispose()
    vi.runAllTimers()
    expect(getSnapshot()).toBe(visibleBefore)
  })
})
