import { describe, expect, it } from 'vitest'
import { artifactsForConversation } from './artifactScope'
import { decodeSnapshot, type CollaborationSnapshot } from './collaboration'
import { fetchStillCurrent, reduceCollaboration, type CollaborationView } from './collaborationStore'
import { collaborationCard, deliveryCardState } from './eventCard'
import { mentionPlan, roomPost, sendMentions } from './mentionPlan'
import { contactAvatarMood, contactSignal } from './presence'
import type { ToolCallRecord } from '../types'

const snapshot: CollaborationSnapshot = {
  deliveries: [],
  rooms: [],
  exchanges: [],
  pending: [],
}

function view(revision: number): CollaborationView {
  return { snapshot, error: '', revision }
}

describe('collaboration snapshot', () => {
  it('keeps the previous snapshot when a late or unreadable payload arrives', () => {
    const current = view(3)
    expect(reduceCollaboration(current, 2, { ok: true, snapshot: { ...snapshot, error: 'late' } })).toBe(current)
    const failed = reduceCollaboration(current, 4, decodeSnapshot({ deliveries: [{}] }))
    expect(failed.snapshot).toBe(current.snapshot)
    expect(failed.error).toBe('协作状态无法读取')
    expect(failed.revision).toBe(4)
  })

  it('drops an in-flight read once a newer event has been applied', () => {
    expect(fetchStillCurrent(1, 2)).toBe(false)
    expect(fetchStillCurrent(2, 2)).toBe(true)
  })

  it('reads a delivery without treating acceptance as completion', () => {
    const decoded = decodeSnapshot({
      deliveries: [{
        id: 'dlv', exchangeId: 'ex', entryId: 'ent', roomCycle: 1,
        from: { kind: 'user' }, toBotId: 'b', toName: 'Bee', toConversationId: 'conv_bot_b',
        kind: 'peer', body: 'hello', depth: 0, status: 'queued', admitted: false, sensitive: false,
        receipts: [{ at: 1, state: 'accepted' }], createdAt: 1, updatedAt: 1,
        attachmentIds: ['file-1'],
      }],
      rooms: [], exchanges: [], pending: [],
    })
    expect(decoded.ok && decoded.snapshot.deliveries[0].status).toBe('queued')
    expect(decoded.ok && decoded.snapshot.deliveries[0].attachmentIds).toEqual(['file-1'])
  })
})

describe('bot presence', () => {
  it('keeps approval waiting ahead of work and errors and never calls queued work success', () => {
    const running = { id: 'run', status: 'running' as const, updatedAt: 3 }
    expect(contactAvatarMood({ generating: true, pending: true, error: true, deliveries: [running] })).toBe('wait')
    expect(contactAvatarMood({ generating: true, pending: false, attention: true, deliveries: [running] })).toBe('wait')
    expect(contactAvatarMood({ generating: false, pending: false, deliveries: [{ ...running, status: 'queued' }] })).toBe('work')
    expect(contactAvatarMood({ generating: false, pending: false, deliveries: [{ ...running, status: 'failed' }] })).toBe('error')
    expect(contactAvatarMood({ generating: false, pending: false, deliveries: [{ ...running, status: 'succeeded' }] })).toBe('idle')
  })

  it('shows working for an in-flight delivery and attention only for the latest problem', () => {
    expect(contactSignal({
      generating: false,
      pending: false,
      deliveries: [
        { id: 'old', status: 'failed', updatedAt: 1 },
        { id: 'new', status: 'succeeded', updatedAt: 2 },
      ],
    })).toBe('quiet')
    expect(contactSignal({
      generating: false,
      pending: false,
      deliveries: [{ id: 'run', status: 'queued', updatedAt: 1 }],
    })).toBe('working')
    expect(contactSignal({
      generating: false,
      pending: true,
      deliveries: [{ id: 'done', status: 'succeeded', updatedAt: 2 }],
    })).toBe('attention')
    expect(contactSignal({
      generating: true,
      pending: false,
      attention: true,
      deliveries: [{ id: 'run', status: 'running', updatedAt: 3 }],
    })).toBe('attention')
  })
})

describe('group mentions', () => {
  const members = [{ botId: 'a', name: 'Ada' }, { botId: 'b', name: 'Bee' }]

  it('does not fan out when nobody is addressed', () => {
    expect(mentionPlan(members, [], 'hello everyone')).toEqual([])
    expect(roomPost(members, [], 'hello everyone')).toEqual({ mode: 'broadcast' })
    expect(mentionPlan(members, ['a'], 'hello @Bee @missing')).toEqual(['a', 'b'])
    expect(roomPost(members, ['a'], 'hello')).toEqual({ mode: 'mention', ids: ['a'] })
  })

  it('stops after the first rejected recipient', async () => {
    const calls: string[] = []
    const result = await sendMentions(['a', 'b', 'c'], async (id) => {
      calls.push(id)
      if (id === 'b') throw new Error('busy')
    })
    expect(calls).toEqual(['a', 'b'])
    expect(result.sent).toEqual(['a'])
    expect(result.error).toBeInstanceOf(Error)
  })
})

describe('artifact scope', () => {
  it('keeps every artifact until a Bot conversation is selected', () => {
    const items = [{ conversationId: 'conv_bot_a' }, { conversationId: 'other' }]
    expect(artifactsForConversation(items, null)).toEqual(items)
    expect(artifactsForConversation(items, 'conv_bot_a')).toEqual([items[0]])
  })
})

describe('collaboration cards', () => {
  it('uses the delivery status instead of dumping the payload', () => {
    const toolCall: ToolCallRecord = {
      id: 'tool',
      toolName: 'bot_peer_send',
      status: 'completed',
      result: JSON.stringify({
        exchangeId: 'ex-1', status: 'queued', toName: 'Bee', body: 'hello', error: 'not yet',
      }),
    }
    const card = collaborationCard(toolCall)
    expect(card).toMatchObject({ status: 'queued', exchangeId: 'ex-1', body: 'hello', name: 'Bee' })
    expect(card?.body).not.toContain('exchangeId')
  })

  it('replaces the frozen send status only after that delivery is in the snapshot', () => {
    const toolCall: ToolCallRecord = {
      id: 'tool',
      toolName: 'bot_peer_send',
      status: 'completed',
      result: JSON.stringify({
        id: 'dlv-1', exchangeId: 'ex-1', status: 'queued', toName: 'Bee', body: 'hello', error: 'not yet',
      }),
    }
    const card = collaborationCard(toolCall)
    const finished = { id: 'dlv-1', status: 'succeeded' as const, error: null }
    const other = { id: 'dlv-2', status: 'failed' as const, error: 'other' }
    expect(card?.deliveryId).toBe('dlv-1')
    expect(deliveryCardState(card!, [])).toMatchObject({ status: 'queued', error: 'not yet', pending: false })
    expect(deliveryCardState(card!, [other])).toMatchObject({ status: 'queued', error: 'not yet' })
    expect(deliveryCardState(card!, [other, finished])).toEqual({ status: 'succeeded', error: undefined, pending: false })
    expect(deliveryCardState({ ...card!, pending: true, deliveryId: undefined }, [finished])).toMatchObject({ status: 'queued', error: 'not yet', pending: true })
    expect(deliveryCardState({ ...card!, pending: true }, [finished])).toEqual({ status: 'succeeded', error: undefined, pending: false })
    const rejected = collaborationCard({
      id: 'tool', toolName: 'bot_peer_send', status: 'error', result: '目标不存在',
    })
    expect(deliveryCardState(rejected!, [finished])).toMatchObject({ error: '目标不存在' })
  })
})
