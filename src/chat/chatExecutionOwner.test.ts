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
  it('advances a per-conversation turn epoch only when a newer execution starts', async () => {
    const owner = createChatExecutionOwner()
    const initial = owner.turnEpoch('a')
    const first = owner.begin({ conversationId: 'a', kind: 'send', startedAt: 1 })!
    const duringFirst = owner.turnEpoch('a')
    expect(duringFirst).toBeGreaterThan(initial)
    owner.observe({ kind: 'runEvent', conversationId: 'a', runId: 'first', started: true })
    expect(owner.turnEpoch('a')).toBe(duringFirst)
    await owner.finish(first, null, ports())
    expect(owner.turnEpoch('a')).toBe(duringFirst)
    expect(owner.begin({ conversationId: 'a', kind: 'send', startedAt: 2 })).not.toBeNull()
    expect(owner.turnEpoch('a')).toBeGreaterThan(duringFirst)
    expect(owner.turnEpoch('b')).toBe(initial)
  })

  it('grants one cancellation request per run and suppresses only its late content', () => {
    const owner = createChatExecutionOwner()
    owner.begin({ conversationId: 'a', kind: 'send', startedAt: 1 })
    owner.begin({ conversationId: 'b', kind: 'send', startedAt: 2 })
    owner.observe({ kind: 'runEvent', conversationId: 'a', runId: 'run-a', started: true })
    owner.observe({ kind: 'runEvent', conversationId: 'b', runId: 'run-b', started: true })
    const permit = owner.requestCancellation('a', 'run-a')
    expect(permit).not.toBeNull()
    expect(owner.requestCancellation('a', 'run-a')).toBeNull()
    expect(owner.allowsStreamPayload({ conversationId: 'a', runId: 'run-a', type: 'delta' })).toBe(false)
    expect(owner.allowsStreamPayload({ conversationId: 'a', type: 'delta' })).toBe(false)
    expect(owner.allowsStreamPayload({ conversationId: 'a', runId: 'unrelated', type: 'delta' })).toBe(true)
    expect(owner.allowsStreamPayload({ conversationId: 'a', runId: 'run-a', type: 'run_cancelled' })).toBe(true)
    expect(owner.allowsStreamPayload({ conversationId: 'a', runId: 'run-a', type: 'run_completed' })).toBe(true)
    expect(owner.allowsStreamPayload({ conversationId: 'a', runId: 'run-a', type: 'run_failed' })).toBe(true)
    expect(owner.allowsStreamPayload({ conversationId: 'b', runId: 'run-b', type: 'delta' })).toBe(true)
    expect(owner.requestCancellation('b', 'run-b')).not.toBeNull()
  })

  it('allows a failed cancellation to be retried and restores content delivery', () => {
    const owner = createChatExecutionOwner()
    owner.begin({ conversationId: 'a', kind: 'send', startedAt: 1 })
    const first = owner.requestCancellation('a', null)!
    owner.completeCancellation(first, false)
    expect(owner.allowsStreamPayload({ conversationId: 'a', type: 'delta' })).toBe(true)
    expect(owner.requestCancellation('a', null)).not.toBeNull()
  })

  it('keeps an unidentified cancelled run fenced when its first run ID arrives late', () => {
    const owner = createChatExecutionOwner()
    owner.observe({ kind: 'externalStarted', conversationId: 'a' })
    const permit = owner.requestCancellation('a')!
    owner.observe({ kind: 'runEvent', conversationId: 'a', runId: 'first', groupId: 'group-1', started: true })
    expect(owner.allowsStreamPayload({ conversationId: 'a', runId: 'first', type: 'delta' })).toBe(false)
    expect(owner.allowsStreamPayload({ conversationId: 'a', runId: 'first', type: 'run_cancelled' })).toBe(true)
    owner.completeCancellation(permit, true)
    expect(owner.requestCancellation('a', 'first')).toBeNull()
    owner.observe({ kind: 'runEvent', conversationId: 'a', runId: 'next', groupId: 'group-2', started: true })
    expect(owner.allowsStreamPayload({ conversationId: 'a', runId: 'next', type: 'delta' })).toBe(true)
  })

  it('does not let an old cancellation completion clear a newer run fence', async () => {
    const owner = createChatExecutionOwner()
    const old = owner.begin({ conversationId: 'a', kind: 'send', startedAt: 1 })!
    const oldPermit = owner.requestCancellation('a', 'old')!
    await owner.finish(old, null, ports())
    owner.begin({ conversationId: 'a', kind: 'send', startedAt: 2 })
    const newPermit = owner.requestCancellation('a', 'new')!
    expect(owner.completeCancellation(oldPermit, false)).toBe(false)
    expect(owner.requestCancellation('a', 'new')).toBeNull()
    expect(owner.allowsStreamPayload({ conversationId: 'a', runId: 'new', type: 'delta' })).toBe(false)
    expect(owner.completeCancellation(newPermit, false)).toBe(true)
    expect(owner.allowsStreamPayload({ conversationId: 'a', runId: 'new', type: 'delta' })).toBe(true)
  })

  it('clears a previous cancellation when an external new run starts', () => {
    const owner = createChatExecutionOwner()
    owner.observe({ kind: 'externalStarted', conversationId: 'a' })
    owner.observe({ kind: 'runEvent', conversationId: 'a', runId: 'old', groupId: 'group-1', started: true })
    // The caller may not know the run ID even after the owner has observed it.
    const oldPermit = owner.requestCancellation('a', null)!
    owner.completeCancellation(oldPermit, true)
    expect(owner.allowsStreamPayload({ conversationId: 'a', runId: 'old', type: 'delta' })).toBe(false)
    owner.observe({ kind: 'runEvent', conversationId: 'a', runId: 'new', groupId: 'group-2', started: true })
    expect(owner.allowsStreamPayload({ conversationId: 'a', runId: 'new', type: 'delta' })).toBe(true)
    expect(owner.requestCancellation('a', 'new')).not.toBeNull()
    owner.completeCancellation(oldPermit, false)
    expect(owner.allowsStreamPayload({ conversationId: 'a', runId: 'new', type: 'delta' })).toBe(false)
  })

  it('does not mistake another arm of the same restored group for a new execution', () => {
    const owner = createChatExecutionOwner()
    owner.observe({ kind: 'runEvent', conversationId: 'a', runId: 'arm-a', groupId: 'group-1', started: true })
    owner.requestCancellation('a')
    owner.observe({ kind: 'runEvent', conversationId: 'a', runId: 'arm-b', groupId: 'group-1', started: true })
    expect(owner.allowsStreamPayload({ conversationId: 'a', runId: 'arm-a', type: 'delta' })).toBe(false)
    expect(owner.allowsStreamPayload({ conversationId: 'a', runId: 'arm-b', type: 'delta' })).toBe(false)
    expect(owner.requestCancellation('a', 'arm-b')).toBeNull()
    owner.observe({ kind: 'runEvent', conversationId: 'a', runId: 'arm-c', started: true })
    expect(owner.requestCancellation('a', 'arm-c')).toBeNull()
    owner.observe({ kind: 'runEvent', conversationId: 'a', runId: 'next', groupId: 'group-2', started: true })
    expect(owner.requestCancellation('a', 'next')).not.toBeNull()
  })

  it('fences every arm of a locally owned multi-answer group after a conversation-level cancel', () => {
    const owner = createChatExecutionOwner({ begin: vi.fn(), end: vi.fn() })
    owner.begin({
      conversationId: 'a', kind: 'send', startedAt: 1,
      group: { groupId: 'group-1', arms: [{ providerId: 'p', model: 'm' }] },
    })
    owner.observe({ kind: 'runEvent', conversationId: 'a', runId: 'arm-a', started: true })
    owner.observe({ kind: 'runEvent', conversationId: 'a', runId: 'arm-b', started: true })
    owner.requestCancellation('a', 'arm-a')
    expect(owner.allowsStreamPayload({ conversationId: 'a', runId: 'arm-a', type: 'delta' })).toBe(false)
    expect(owner.allowsStreamPayload({ conversationId: 'a', runId: 'arm-b', type: 'delta' })).toBe(false)
    expect(owner.allowsStreamPayload({ conversationId: 'a', runId: 'arm-b', type: 'run_completed' })).toBe(true)
  })

  it('uses external end as the boundary when a new run has no group identity', () => {
    const owner = createChatExecutionOwner()
    owner.observe({ kind: 'runEvent', conversationId: 'a', runId: 'old', started: true })
    owner.requestCancellation('a')
    owner.observe({ kind: 'runEvent', conversationId: 'a', runId: 'ambiguous', started: true })
    expect(owner.requestCancellation('a', 'ambiguous')).toBeNull()
    owner.observe({ kind: 'externalEnded', conversationId: 'a' })
    owner.observe({ kind: 'runEvent', conversationId: 'a', runId: 'new', started: true })
    expect(owner.requestCancellation('a', 'new')).not.toBeNull()
  })

  it('rejects late content after finish through run settlement even though the cancellation fence is gone', async () => {
    const owner = createChatExecutionOwner()
    const lease = owner.begin({ conversationId: 'a', kind: 'send', startedAt: 1 })!
    owner.observe({ kind: 'runEvent', conversationId: 'a', runId: 'old', started: true })
    owner.requestCancellation('a', 'old')
    await owner.finish(lease, conversation('a'), ports())
    expect(owner.allowsStreamPayload({ conversationId: 'a', runId: 'old', type: 'delta' })).toBe(true)
    expect(owner.observe({ kind: 'runEvent', conversationId: 'a', runId: 'old' })).toBe(false)
  })

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
    const sending = owner.submitPreparedRun({ lease, content: 'hello', attachments: [], attachmentSkillId: null }, effects)
    owner.observe({ kind: 'runEvent', conversationId: 'a', runId: 'run-a', started: true })
    owner.observe({ kind: 'deferTerminal', terminal: { conversationId: 'a', runId: 'run-a', reason: 'done' } })
    resolveSend(conversation('a'))
    const result = await sending
    expect(result.kind).toBe('persisted')
    expect(order).toEqual(['outcome', 'authoritative', 'queue'])
    expect(effects.completeTerminal).not.toHaveBeenCalled()
  })

  it('defers a local invoke terminal instead of treating it as an external completion', async () => {
    const owner = createChatExecutionOwner()
    const lease = owner.begin({ conversationId: 'a', kind: 'send', startedAt: 1 })!
    owner.observe({ kind: 'runEvent', conversationId: 'a', runId: 'local', started: true })
    expect(owner.observeTerminal({ conversationId: 'a', runId: 'local', reason: 'done' }, 'single'))
      .toMatchObject({ kind: 'deferred' })
    const effects = ports()
    await owner.finish(lease, conversation('a'), effects)
    expect(effects.completeWithConversation).toHaveBeenCalledTimes(1)
    expect(effects.completeTerminal).not.toHaveBeenCalled()
  })

  it('finishes a recovered group only after every arm has a terminal', () => {
    const groups = { begin: vi.fn(), end: vi.fn() }
    const owner = createChatExecutionOwner(groups)
    owner.observe({ kind: 'runEvent', conversationId: 'a', runId: 'arm-1', started: true, groupId: 'group-a', groupSize: 2 })
    owner.observe({ kind: 'runEvent', conversationId: 'a', runId: 'arm-2', started: true, groupId: 'group-a', groupSize: 2 })
    expect(owner.observeTerminal({ conversationId: 'a', runId: 'arm-1', reason: 'done' }, 'group'))
      .toMatchObject({ kind: 'pending' })
    expect(groups.end).not.toHaveBeenCalled()
    const ready = owner.observeTerminal({ conversationId: 'a', runId: 'arm-2', reason: 'done' }, 'group')
    expect(ready.kind).toBe('ready')
    expect(groups.end).toHaveBeenCalledOnce()
    expect(owner.snapshot('a').inFlight).toBe(true)
    if (ready.kind !== 'ready') throw new Error('Expected a terminal permit')
    expect(owner.completeExternalTerminal(ready.permit)).toBe(true)
    expect(owner.snapshot('a').inFlight).toBe(false)
  })

  it('does not let a late old terminal settle a newer recovered group', () => {
    const owner = createChatExecutionOwner({ begin: vi.fn(), end: vi.fn() })
    owner.observe({ kind: 'runEvent', conversationId: 'a', runId: 'old', started: true, groupId: 'old-group', groupSize: 1 })
    const old = owner.observeTerminal({ conversationId: 'a', runId: 'old', reason: 'done' }, 'group')
    if (old.kind !== 'ready') throw new Error('Expected an old terminal permit')
    owner.observe({ kind: 'runEvent', conversationId: 'a', runId: 'new', started: true, groupId: 'new-group', groupSize: 1 })
    expect(owner.completeExternalTerminal(old.permit)).toBe(false)
    expect(owner.observeTerminal({ conversationId: 'a', runId: 'old', reason: 'done' }, 'group'))
      .toMatchObject({ kind: 'ignored' })
    expect(owner.snapshot('a').inFlight).toBe(true)
    expect(owner.observeTerminal({ conversationId: 'a', runId: 'new', reason: 'done' }, 'group').kind).toBe('ready')
  })

  it('treats an explicit recovered group as a new run after an unidentified external run', () => {
    const owner = createChatExecutionOwner({ begin: vi.fn(), end: vi.fn() })
    owner.observe({ kind: 'runEvent', conversationId: 'a', runId: 'old', started: true })
    const cancelled = owner.requestCancellation('a', 'old')!
    owner.completeCancellation(cancelled, true)
    owner.observe({ kind: 'runEvent', conversationId: 'a', runId: 'arm-1', started: true, groupId: 'group-a', groupSize: 2 })
    expect(owner.observe({ kind: 'runEvent', conversationId: 'a', runId: 'old' })).toBe(false)
    expect(owner.allowsStreamPayload({ conversationId: 'a', runId: 'arm-1', type: 'text_delta' })).toBe(true)
    expect(owner.observeTerminal({ conversationId: 'a', runId: 'arm-1', reason: 'done' }, 'group'))
      .toMatchObject({ kind: 'pending' })
  })

  it('distinguishes a failed assistant run that kept the user message from an uncommitted send', async () => {
    const kept = conversation('a')
    kept.messages = [{ id: 'user-1', role: 'user', content: 'hello', timestamp: 1 }]
    const failedAfterPersist = Object.assign(new Error('model failed'), { conversation: kept })
    const first = createChatExecutionOwner(undefined, { sendMessage: vi.fn().mockRejectedValue(failedAfterPersist) })
    const effects = { ...ports(), onOutcome: vi.fn() }
    const lease = first.begin({ conversationId: 'a', kind: 'send', startedAt: 100 })!
    const result = await first.submitPreparedRun({ lease, content: 'hello', attachments: [], attachmentSkillId: null }, effects)
    expect(result).toMatchObject({ kind: 'persisted_error', conversation: kept, error: failedAfterPersist })
    expect(effects.settleQueue).toHaveBeenCalledWith('a')
    expect(first.snapshot('a').inFlight).toBe(false)

    const second = createChatExecutionOwner(undefined, { sendMessage: vi.fn().mockRejectedValue(new Error('write failed')) })
    const nextLease = second.begin({ conversationId: 'b', kind: 'send', startedAt: 101 })!
    const rejected = await second.submitPreparedRun({ lease: nextLease, content: 'hello', attachments: [], attachmentSkillId: null }, portsWithOutcome())
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
    const sending = owner.submitPreparedRun({ lease, content: 'x', attachments: [], attachmentSkillId: null }, portsWithOutcome())
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
    const outcome = await owner.submitPreparedRun({ lease, content: 'x', attachments: [], attachmentSkillId: null }, portsWithOutcome())
    expect(outcome).toMatchObject({ kind: 'persisted_error', conversation: kept, error: { message: '上游断开' } })
  })

  it('releases execution state even when a presentation observer throws', async () => {
    const report = vi.spyOn(console, 'error').mockImplementation(() => {})
    const owner = createChatExecutionOwner(undefined, { sendMessage: vi.fn().mockResolvedValue(conversation('a')) })
    const lease = owner.begin({ conversationId: 'a', kind: 'send', startedAt: 100 })!
    const effects = { ...ports(), onOutcome: vi.fn(() => { throw new Error('view failed') }) }
    const result = await owner.submitPreparedRun({ lease, content: 'x', attachments: [], attachmentSkillId: null }, effects)
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
