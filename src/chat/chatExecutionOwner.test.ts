import { describe, expect, it, vi } from 'vitest'
import { createChatExecutionOwner } from './chatExecutionOwner'
import type { Conversation } from './types'

const conversation = (id: string): Conversation => ({
  id, revision: 1, title: id, provider_id: 'p', model: 'm',
  messages: [], created_at: 1, updated_at: 1,
} as Conversation)
const ports = () => ({
  completeWithConversation: vi.fn(),
  completeTerminal: vi.fn().mockResolvedValue(undefined),
  abandonPreview: vi.fn(),
  settleQueue: vi.fn(),
})

describe('chat execution owner', () => {
  it('owns parallel conversations independently and publishes their execution snapshots', async () => {
    const owner = createChatExecutionOwner()
    const changed = vi.fn()
    owner.subscribe(changed)
    const a = owner.begin({ conversationId: 'a', kind: 'send', startedAt: 100 })
    const b = owner.begin({ conversationId: 'b', kind: 'regenerate', startedAt: 101 })
    expect(a).not.toBeNull()
    expect(b).not.toBeNull()
    expect(owner.snapshot('a').inFlight).toBe(true)
    expect(owner.snapshot('b').inFlight).toBe(true)
    expect(owner.begin({ conversationId: 'a', kind: 'send', startedAt: 102 })).toBeNull()
    await owner.finish(a!, conversation('a'), ports())
    expect(owner.snapshot('a').inFlight).toBe(false)
    expect(owner.snapshot('b').inFlight).toBe(true)
    expect(changed).toHaveBeenCalled()
  })

  it('retires a completed run and releases the send reservation before queue settlement', async () => {
    const owner = createChatExecutionOwner()
    const claim = owner.claimSend(null)
    expect(owner.bindSend(claim!, 'a')).toBe(true)
    const lease = owner.begin({ conversationId: 'a', kind: 'send', startedAt: 100, claim: claim! })!
    expect(owner.observe({ kind: 'runEvent', conversationId: 'a', runId: 'run-a', started: true })).toBe(true)
    const effects = ports()
    effects.settleQueue.mockImplementation(() => {
      expect(owner.claimSend('a')).not.toBeNull()
    })
    await owner.finish(lease, conversation('a'), effects)
    expect(owner.observe({ kind: 'runEvent', conversationId: 'a', runId: 'run-a' })).toBe(false)
    expect(effects.completeWithConversation).toHaveBeenCalledTimes(1)
  })

  it('does not clear a newer run when an older lease finishes late', async () => {
    const owner = createChatExecutionOwner()
    const old = owner.begin({ conversationId: 'a', kind: 'send', startedAt: 100 })!
    owner.observe({ kind: 'drop', conversationId: 'a' })
    const newer = owner.begin({ conversationId: 'a', kind: 'send', startedAt: 200 })!
    await owner.finish(old, null, ports())
    expect(owner.snapshot('a').inFlight).toBe(true)
    await owner.finish(newer, null, ports())
    expect(owner.snapshot('a').inFlight).toBe(false)
  })

  it('keeps a background run alive when the view unsubscribes', async () => {
    const owner = createChatExecutionOwner()
    const changed = vi.fn()
    const unsubscribe = owner.subscribe(changed)
    const lease = owner.begin({ conversationId: 'a', kind: 'send', startedAt: 100 })!
    unsubscribe()
    expect(owner.snapshot('a').inFlight).toBe(true)
    await owner.finish(lease, conversation('a'), ports())
    expect(owner.snapshot('a').inFlight).toBe(false)
  })

  it('ends a multi-answer group before allowing the next queued send', async () => {
    const order: string[] = []
    const owner = createChatExecutionOwner({
      begin: vi.fn(() => { order.push('group-begin') }),
      end: vi.fn(() => { order.push('group-end') }),
    })
    const lease = owner.begin({
      conversationId: 'a', kind: 'replyWithModel', startedAt: 100,
      group: { groupId: 'g1', arms: [{ providerId: 'p', model: 'm' }] },
    })!
    const effects = ports()
    effects.settleQueue.mockImplementation(() => { order.push('queue-settle') })
    expect(owner.snapshot('a').groupId).toBe('g1')
    await owner.finish(lease, conversation('a'), effects)
    expect(order).toEqual(['group-begin', 'group-end', 'queue-settle'])
  })
})
