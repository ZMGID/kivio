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

  it('publishes the persisted single-run result before settling an early terminal', async () => {
    let resolveSend!: (value: Conversation) => void
    const sendMessage = vi.fn(() => new Promise<Conversation>((resolve) => { resolveSend = resolve }))
    const owner = createChatExecutionOwner(undefined, { sendMessage })
    const lease = owner.begin({ conversationId: 'a', kind: 'send', startedAt: 100 })!
    const order: string[] = []
    const effects = {
      ...ports(),
      onOutcome: vi.fn(() => { order.push('outcome') }),
    }
    effects.completeWithConversation.mockImplementation(() => { order.push('authoritative') })
    effects.settleQueue.mockImplementation(() => { order.push('queue') })
    const sending = owner.submitPreparedSingleRun({ lease, content: 'hello', attachments: [], attachmentSkillId: null }, effects)
    owner.observe({ kind: 'runEvent', conversationId: 'a', runId: 'run-a', started: true })
    owner.observe({ kind: 'deferTerminal', terminal: { conversationId: 'a', runId: 'run-a', reason: 'done' } })
    resolveSend(conversation('a'))
    const result = await sending
    expect(result.kind).toBe('persisted')
    expect(order).toEqual(['outcome', 'authoritative', 'queue'])
    expect(effects.completeTerminal).not.toHaveBeenCalled()
  })

  it('distinguishes a failed assistant run that kept the user message from an uncommitted send', async () => {
    const kept = conversation('a')
    kept.messages = [{ id: 'user-1', role: 'user', content: 'hello', timestamp: 1 }]
    const failedAfterPersist = Object.assign(new Error('model failed'), { conversation: kept })
    const first = createChatExecutionOwner(undefined, { sendMessage: vi.fn().mockRejectedValue(failedAfterPersist) })
    const effects = { ...ports(), onOutcome: vi.fn() }
    const lease = first.begin({ conversationId: 'a', kind: 'send', startedAt: 100 })!
    const result = await first.submitPreparedSingleRun({ lease, content: 'hello', attachments: [], attachmentSkillId: null }, effects)
    expect(result).toMatchObject({ kind: 'persisted_error', conversation: kept, error: failedAfterPersist })
    expect(effects.settleQueue).toHaveBeenCalledWith('a')
    expect(first.snapshot('a').inFlight).toBe(false)

    const second = createChatExecutionOwner(undefined, { sendMessage: vi.fn().mockRejectedValue(new Error('write failed')) })
    const nextLease = second.begin({ conversationId: 'b', kind: 'send', startedAt: 101 })!
    const rejected = await second.submitPreparedSingleRun({ lease: nextLease, content: 'hello', attachments: [], attachmentSkillId: null }, portsWithOutcome())
    expect(rejected).toMatchObject({ kind: 'not_committed', error: new Error('write failed') })
    expect(second.snapshot('b').inFlight).toBe(false)
  })

  it('finishes a background single run after the page unsubscribes', async () => {
    let resolveSend!: (value: Conversation) => void
    const owner = createChatExecutionOwner(undefined, {
      sendMessage: () => new Promise((resolve) => { resolveSend = resolve }),
    })
    const lease = owner.begin({ conversationId: 'background', kind: 'send', startedAt: 100 })!
    const unsubscribe = owner.subscribe(vi.fn())
    const sending = owner.submitPreparedSingleRun({ lease, content: 'x', attachments: [], attachmentSkillId: null }, portsWithOutcome())
    unsubscribe()
    resolveSend(conversation('background'))
    expect((await sending).kind).toBe('persisted')
    expect(owner.snapshot('background').inFlight).toBe(false)
  })

  it('keeps a non-Error backend failure message and persisted conversation', async () => {
    const kept = conversation('a')
    const owner = createChatExecutionOwner(undefined, {
      sendMessage: vi.fn().mockRejectedValue({ message: '上游断开', conversation: kept }),
    })
    const lease = owner.begin({ conversationId: 'a', kind: 'send', startedAt: 100 })!
    const outcome = await owner.submitPreparedSingleRun({ lease, content: 'x', attachments: [], attachmentSkillId: null }, portsWithOutcome())
    expect(outcome).toMatchObject({ kind: 'persisted_error', conversation: kept, error: { message: '上游断开' } })
  })

  it('releases execution state even when a presentation observer throws', async () => {
    const report = vi.spyOn(console, 'error').mockImplementation(() => {})
    const owner = createChatExecutionOwner(undefined, { sendMessage: vi.fn().mockResolvedValue(conversation('a')) })
    const lease = owner.begin({ conversationId: 'a', kind: 'send', startedAt: 100 })!
    const effects = { ...ports(), onOutcome: vi.fn(() => { throw new Error('view failed') }) }
    const result = await owner.submitPreparedSingleRun({ lease, content: 'x', attachments: [], attachmentSkillId: null }, effects)
    expect(result.kind).toBe('persisted')
    expect(owner.snapshot('a').inFlight).toBe(false)
    expect(effects.settleQueue).toHaveBeenCalledWith('a', conversation('a'))
    expect(report).toHaveBeenCalled()
    report.mockRestore()
  })
})

function portsWithOutcome() {
  return { ...ports(), onOutcome: vi.fn() }
}
