import { describe, expect, it, vi } from 'vitest'
import { createChatExecutionOwner } from './chatExecutionOwner'
import { createStreamPreviewOwner } from './streamPreviewOwner'
import { getActiveGroup } from './groupStreamingStore'
import { createChatRunCommands, type RunCommandPresentationEvent } from './chatRunCommands'
import type { ChatMessage, Conversation } from './types'

const user = (id: string, content: string): ChatMessage => ({ id, role: 'user', content, timestamp: 1 })
const assistant = (id: string, content: string, groupId?: string): ChatMessage => ({
  id, role: 'assistant', content, timestamp: 2,
  ...(groupId ? { group_id: groupId } : {}),
})
const conversation = (messages: ChatMessage[]): Conversation => ({
  id: 'a', revision: 1, title: 'a', provider_id: 'provider', model: 'model',
  messages, created_at: 1, updated_at: 1, agent_runtime: { kind: 'builtin' },
})

describe('chat run commands', () => {
  it('regenerates an edited user turn after optimistic truncation and settles its queue', async () => {
    const existing = conversation([
      user('u1', 'first'), assistant('a1', 'answer'), user('u2', 'old'), assistant('a2', 'old answer'),
    ])
    const persisted = conversation([user('u1', 'first'), assistant('a1', 'answer'), user('u2', 'new')])
    const regenerateMessage = vi.fn().mockResolvedValue(persisted)
    const persistence = { regenerateMessage, replyWithModel: vi.fn() }
    const executionOwner = createChatExecutionOwner()
    const previewOwner = createStreamPreviewOwner()
    const events: RunCommandPresentationEvent[] = []
    const order: string[] = []
    const commands = createChatRunCommands({
      executionOwner, previewOwner, persistence, now: () => 100,
      settlementPorts: {
        completeWithConversation: vi.fn(), completeTerminal: vi.fn().mockResolvedValue(undefined),
        abandonPreview: vi.fn(), settleQueue: vi.fn(() => { order.push('queue') }),
      },
      presentation: { present: (event) => { events.push(event); if (event.kind === 'persisted') order.push('persisted') } },
    })

    const result = await commands.regenerate({ conversation: existing, messageId: 'u2', newContent: ' new ' })
    expect(result).toMatchObject({ kind: 'persisted', conversation: persisted })
    expect(regenerateMessage).toHaveBeenCalledWith('a', 'u2', 'new')
    expect(events.find((event) => event.kind === 'truncated')).toMatchObject({
      kind: 'truncated', removedMessageIds: ['a2'],
      conversation: { messages: [
        { id: 'u1', content: 'first' }, { id: 'a1', content: 'answer' }, { id: 'u2', content: 'new' },
      ] },
    })
    expect(order).toEqual(['persisted', 'queue'])
    expect(executionOwner.snapshot('a').inFlight).toBe(false)
    previewOwner.dispose()
  })

  it('replies with another model only to the last turn, retaining all sibling arms until queue settlement', async () => {
    const existing = conversation([
      user('u1', 'question'),
      { ...assistant('a1', 'first answer', 'group-1'), provider_id: 'p1', model: 'm1' },
      { ...assistant('a2', 'second answer', 'group-1'), provider_id: 'p2', model: 'm2' },
    ])
    let resolveReply!: (value: Conversation) => void
    const replyWithModel = vi.fn(() => new Promise<Conversation>((resolve) => { resolveReply = resolve }))
    const executionOwner = createChatExecutionOwner()
    const previewOwner = createStreamPreviewOwner()
    const order: string[] = []
    const commands = createChatRunCommands({
      executionOwner, previewOwner,
      persistence: { regenerateMessage: vi.fn(), replyWithModel }, now: () => 200,
      newGroupId: () => 'must-not-be-used',
      settlementPorts: {
        completeWithConversation: vi.fn(), completeTerminal: vi.fn().mockResolvedValue(undefined),
        abandonPreview: vi.fn(), settleQueue: vi.fn(() => {
          expect(getActiveGroup('a')).toBeUndefined()
          order.push('queue')
        }),
      },
      presentation: { present: (event) => { if (event.kind === 'persisted') order.push('persisted') } },
    })
    const running = commands.replyWithModel({
      conversation: existing, messageId: 'a1', providerId: 'p3', model: 'm3',
    })
    expect(replyWithModel).toHaveBeenCalledWith('a', 'a1', 'p3', 'm3', 'group-1')
    expect(getActiveGroup('a')?.columns).toMatchObject([
      { messageId: 'a1', providerId: 'p1', model: 'm1', content: 'first answer' },
      { messageId: 'a2', providerId: 'p2', model: 'm2', content: 'second answer' },
      { providerId: 'p3', model: 'm3' },
    ])
    resolveReply(existing)
    expect((await running).kind).toBe('persisted')
    expect(order).toEqual(['persisted', 'queue'])
    previewOwner.dispose()
  })

  it('rejects a busy regeneration and a model reply to a non-final assistant turn before changing the timeline', async () => {
    const existing = conversation([
      user('u1', 'first'), assistant('a1', 'one'), user('u2', 'second'), assistant('a2', 'two'),
    ])
    const executionOwner = createChatExecutionOwner()
    const previewOwner = createStreamPreviewOwner()
    const events: RunCommandPresentationEvent[] = []
    const regenerateMessage = vi.fn()
    const replyWithModel = vi.fn()
    const commands = createChatRunCommands({
      executionOwner, previewOwner,
      persistence: { regenerateMessage, replyWithModel },
      settlementPorts: {
        completeWithConversation: vi.fn(), completeTerminal: vi.fn().mockResolvedValue(undefined),
        abandonPreview: vi.fn(), settleQueue: vi.fn(),
      },
      presentation: { present: (event) => { events.push(event) } },
    })
    const rejectedReply = await commands.replyWithModel({
      conversation: existing, messageId: 'a1', providerId: 'p3', model: 'm3',
    })
    expect(rejectedReply).toEqual({ kind: 'rejected', reason: 'not_last_turn' })
    expect(events).toMatchObject([{ kind: 'rejected', error: { message: '只能对最后一轮回答换模型' } }])

    const lease = executionOwner.begin({ conversationId: 'a', kind: 'send', startedAt: 1 })!
    const rejectedRegenerate = await commands.regenerate({ conversation: existing, messageId: 'u2' })
    expect(rejectedRegenerate).toEqual({ kind: 'rejected', reason: 'busy' })
    expect(events.some((event) => event.kind === 'truncated')).toBe(false)
    expect(regenerateMessage).not.toHaveBeenCalled()
    expect(replyWithModel).not.toHaveBeenCalled()
    await executionOwner.finish(lease, null, {
      completeWithConversation: vi.fn(), completeTerminal: vi.fn().mockResolvedValue(undefined),
      abandonPreview: vi.fn(), settleQueue: vi.fn(),
    })
    previewOwner.dispose()
  })

  it('reports a failed regeneration for reload, clears an empty preview, and settles the queue', async () => {
    const existing = conversation([user('u1', 'question'), assistant('a1', 'answer')])
    const report = vi.spyOn(console, 'error').mockImplementation(() => {})
    const executionOwner = createChatExecutionOwner()
    const previewOwner = createStreamPreviewOwner()
    const events: RunCommandPresentationEvent[] = []
    const settlementPorts = {
      completeWithConversation: vi.fn(), completeTerminal: vi.fn().mockResolvedValue(undefined),
      abandonPreview: vi.fn(), settleQueue: vi.fn(),
    }
    const commands = createChatRunCommands({
      executionOwner, previewOwner,
      persistence: { regenerateMessage: vi.fn().mockRejectedValue(new Error('model failed')), replyWithModel: vi.fn() },
      settlementPorts,
      presentation: { present: (event) => { events.push(event) } },
    })
    const result = await commands.regenerate({ conversation: existing, messageId: 'a1', newContent: '   ' })
    expect(result).toMatchObject({ kind: 'failed', error: { message: 'model failed' } })
    expect(events.find((event) => event.kind === 'truncated')).toMatchObject({
      kind: 'truncated', conversation: { messages: [{ id: 'u1' }] }, removedMessageIds: ['a1'],
    })
    expect(events.find((event) => event.kind === 'failed')).toMatchObject({
      kind: 'failed', command: 'regenerate', clearPreview: true,
    })
    expect(settlementPorts.settleQueue).toHaveBeenCalledWith('a')
    expect(executionOwner.snapshot('a').inFlight).toBe(false)
    report.mockRestore()
    previewOwner.dispose()
  })

  it('does not let an old done terminal settle the next regeneration', async () => {
    const existing = conversation([user('u1', 'question'), assistant('a1', 'answer')])
    const pending: Array<(value: Conversation) => void> = []
    const regenerateMessage = vi.fn(() => new Promise<Conversation>((resolve) => { pending.push(resolve) }))
    const executionOwner = createChatExecutionOwner()
    const previewOwner = createStreamPreviewOwner()
    const settlementPorts = {
      completeWithConversation: vi.fn(), completeTerminal: vi.fn().mockResolvedValue(undefined),
      abandonPreview: vi.fn(), settleQueue: vi.fn(),
    }
    const commands = createChatRunCommands({
      executionOwner, previewOwner,
      persistence: { regenerateMessage, replyWithModel: vi.fn() },
      settlementPorts, presentation: { present: vi.fn() },
    })
    const first = commands.regenerate({ conversation: existing, messageId: 'a1' })
    executionOwner.observe({ kind: 'runEvent', conversationId: 'a', runId: 'old', started: true })
    executionOwner.observe({ kind: 'deferTerminal', terminal: { conversationId: 'a', runId: 'old', reason: 'done' } })
    pending[0](existing)
    expect((await first).kind).toBe('persisted')

    const second = commands.regenerate({ conversation: existing, messageId: 'a1' })
    expect(executionOwner.snapshot('a').inFlight).toBe(true)
    expect(executionOwner.observe({
      kind: 'deferTerminal', terminal: { conversationId: 'a', runId: 'old', reason: 'done' },
    })).toBe(false)
    expect(executionOwner.snapshot('a').inFlight).toBe(true)
    pending[1](existing)
    expect((await second).kind).toBe('persisted')
    expect(settlementPorts.completeTerminal).not.toHaveBeenCalled()
    expect(settlementPorts.settleQueue).toHaveBeenCalledTimes(2)
    previewOwner.dispose()
  })

  it('releases the execution when optimistic presentation fails before invoke', async () => {
    const existing = conversation([user('u1', 'question'), assistant('a1', 'answer')])
    const executionOwner = createChatExecutionOwner()
    const previewOwner = createStreamPreviewOwner()
    const regenerateMessage = vi.fn()
    const report = vi.spyOn(console, 'error').mockImplementation(() => {})
    const settlementPorts = {
      completeWithConversation: vi.fn(), completeTerminal: vi.fn().mockResolvedValue(undefined),
      abandonPreview: vi.fn(), settleQueue: vi.fn(),
    }
    const commands = createChatRunCommands({
      executionOwner, previewOwner,
      persistence: { regenerateMessage, replyWithModel: vi.fn() },
      settlementPorts,
      presentation: { present: (event) => {
        if (event.kind === 'truncated') throw new Error('view failed')
      } },
    })
    const result = await commands.regenerate({ conversation: existing, messageId: 'a1' })
    expect(result).toMatchObject({ kind: 'failed', error: { message: 'view failed' } })
    expect(regenerateMessage).not.toHaveBeenCalled()
    expect(executionOwner.snapshot('a').inFlight).toBe(false)
    expect(settlementPorts.settleQueue).toHaveBeenCalledWith('a')
    report.mockRestore()
    previewOwner.dispose()
  })

  it('creates a group for an ungrouped last answer and reloads after model failure', async () => {
    const existing = conversation([user('u1', 'question'), assistant('a1', 'answer')])
    const report = vi.spyOn(console, 'error').mockImplementation(() => {})
    const executionOwner = createChatExecutionOwner()
    const previewOwner = createStreamPreviewOwner()
    const events: RunCommandPresentationEvent[] = []
    const newGroupId = vi.fn(() => 'group-new')
    const replyWithModel = vi.fn().mockRejectedValue(new Error('model unavailable'))
    const settlementPorts = {
      completeWithConversation: vi.fn(), completeTerminal: vi.fn().mockResolvedValue(undefined),
      abandonPreview: vi.fn(), settleQueue: vi.fn(() => { expect(getActiveGroup('a')).toBeUndefined() }),
    }
    const commands = createChatRunCommands({
      executionOwner, previewOwner, newGroupId,
      persistence: { regenerateMessage: vi.fn(), replyWithModel },
      settlementPorts,
      presentation: { present: (event) => { events.push(event) } },
    })
    const result = await commands.replyWithModel({
      conversation: existing, messageId: 'a1', providerId: 'p2', model: 'm2',
    })
    expect(result).toMatchObject({ kind: 'failed', error: { message: 'model unavailable' } })
    expect(newGroupId).toHaveBeenCalledTimes(1)
    expect(replyWithModel).toHaveBeenCalledWith('a', 'a1', 'p2', 'm2', 'group-new')
    expect(events.find((event) => event.kind === 'failed')).toMatchObject({
      kind: 'failed', command: 'replyWithModel', clearPreview: false,
    })
    expect(settlementPorts.settleQueue).toHaveBeenCalledWith('a')
    report.mockRestore()
    previewOwner.dispose()
  })
})
