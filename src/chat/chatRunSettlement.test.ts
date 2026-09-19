import { describe, expect, it } from 'vitest'
import { createChatRunSettlement } from './chatRunSettlement'
import type { Conversation } from './types'

function conversation(id: string): Conversation {
  return {
    id, revision: 1, title: id, provider_id: 'test', model: 'test',
    messages: [], created_at: 1, updated_at: 1,
  }
}

function effects() {
  const events: string[] = []
  return {
    events,
    ports: {
      completeWithConversation: (id: string) => { events.push(`persisted:${id}`) },
      completeTerminal: async (terminal: { conversationId: string }) => { events.push(`terminal:${terminal.conversationId}`) },
      abandonPreview: (id: string) => { events.push(`abandon:${id}`) },
      settleQueue: (id: string, persisted?: Conversation | null) => { events.push(`queue:${id}:${persisted ? 'persisted' : 'failed'}`) },
    },
  }
}

describe('chat run settlement', () => {
  it('uses a persisted reply instead of rereading a terminal that arrived before the invoke result', async () => {
    const owner = createChatRunSettlement()
    const result = effects()
    const token = owner.beginInvoke('a')
    owner.acceptRunEvent('a', 'run-a', true)
    owner.deferTerminal({ conversationId: 'a', runId: 'run-a', reason: 'done' })
    await owner.settleInvoke('a', token, conversation('a'), result.ports)

    expect(result.events).toEqual(['persisted:a', 'queue:a:persisted'])
  })

  it('flushes a deferred terminal after hard failure, without draining a queued send', async () => {
    const owner = createChatRunSettlement()
    const result = effects()
    const token = owner.beginInvoke('a')
    owner.acceptRunEvent('a', 'run-a', true)
    owner.deferTerminal({ conversationId: 'a', runId: 'run-a', reason: 'error' })
    await owner.settleInvoke('a', token, null, result.ports)
    const retry = owner.beginInvoke('a')
    await owner.settleInvoke('a', retry, null, result.ports)

    expect(result.events).toEqual([
      'queue:a:failed', 'terminal:a',
      'queue:a:failed', 'abandon:a',
    ])
  })

  it('keeps independent conversations and a navigation change cannot consume their pending terminal', async () => {
    const owner = createChatRunSettlement()
    const result = effects()
    const a = owner.beginInvoke('a')
    const b = owner.beginInvoke('b')
    owner.acceptRunEvent('a', 'run-a', true)
    owner.acceptRunEvent('b', 'run-b', true)
    owner.deferTerminal({ conversationId: 'a', runId: 'run-a', reason: 'done' })
    owner.deferTerminal({ conversationId: 'b', runId: 'run-b', reason: 'done' })
    await owner.settleInvoke('b', b, conversation('b'), result.ports)
    expect(result.events).toEqual(['persisted:b', 'queue:b:persisted'])
    await owner.settleInvoke('a', a, null, result.ports)
    expect(result.events).toEqual([
      'persisted:b', 'queue:b:persisted',
      'queue:a:failed', 'terminal:a',
    ])
  })

  it('does not hold the completed run open while the next queued send is running', async () => {
    const owner = createChatRunSettlement()
    const result = effects()
    let finishNext!: () => void
    const nextRun = new Promise<void>((resolve) => { finishNext = resolve })
    const ports = {
      ...result.ports,
      settleQueue: () => nextRun,
    }
    const token = owner.beginInvoke('a')
    await owner.settleInvoke('a', token, conversation('a'), ports)
    expect(result.events).toEqual(['persisted:a'])
    finishNext()
  })

  it('rejects a terminal from a retired run after a new invocation has begun', async () => {
    const owner = createChatRunSettlement()
    const result = effects()
    const first = owner.beginInvoke('a')
    expect(owner.acceptRunEvent('a', 'run-old', true)).toBe(true)
    await owner.settleInvoke('a', first, conversation('a'), result.ports)

    const second = owner.beginInvoke('a')
    expect(owner.acceptRunEvent('a', 'run-old')).toBe(false)
    expect(owner.deferTerminal({ conversationId: 'a', runId: 'run-old', reason: 'done' })).toBe(false)
    await owner.settleInvoke('a', second, null, result.ports)

    expect(result.events).toEqual([
      'persisted:a', 'queue:a:persisted',
      'queue:a:failed', 'abandon:a',
    ])
  })
})
