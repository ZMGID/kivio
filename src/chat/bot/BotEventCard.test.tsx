// @vitest-environment jsdom
import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import { BotEventCard } from './BotEventCard'
import { subscribePeerExchange } from './exchangeRequest'
import type { ToolCallRecord } from '../types'
import type { Delivery, PendingAction } from './collaboration'

const live = vi.hoisted(() => ({ deliveries: [] as Delivery[], pending: [] as PendingAction[] }))
vi.mock('./collaborationStore', () => ({
  useBotCollaboration: () => ({ snapshot: { deliveries: live.deliveries, pending: live.pending } }),
}))
beforeEach(() => { live.deliveries = []; live.pending = [] })

const toolCall: ToolCallRecord = {
  id: 'tool',
  toolName: 'bot_peer_send',
  status: 'completed',
  result: JSON.stringify({
    id: 'dlv-1',
    exchangeId: 'ex-1',
    status: 'queued',
    toName: 'Bee',
    body: 'hello',
    receipts: [{ state: 'accepted' }],
  }),
}

it('hides exchange query activity while retaining the sent-message entry', () => {
  render(<>
    <BotEventCard toolCall={toolCall} />
    <BotEventCard toolCall={{
      id: 'query', toolName: 'bot_exchange_query', status: 'completed',
      result: JSON.stringify({ id: 'ex-1', entries: [{ body: 'internal transcript' }] }),
    }} />
  </>)
  expect(screen.queryByText('查看交流')).not.toBeInTheDocument()
  expect(screen.getAllByRole('button')).toHaveLength(1)
  expect(screen.getByRole('button', { name: /查看交流.*Bee/ })).toBeInTheDocument()
})

it('shows an accepted send as waiting, not as a JSON dump', () => {
  const seen: string[] = []
  const stop = subscribePeerExchange((id) => seen.push(id))
  render(<BotEventCard toolCall={toolCall} />)
  expect(screen.getByText('已受理，等待处理')).toBeInTheDocument()
  expect(screen.queryByText('hello')).not.toBeInTheDocument()
  expect(screen.queryByText(/exchangeId|receipts/)).not.toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: /查看交流.*Bee/ }))
  expect(seen).toEqual(['ex-1'])
  stop()
})

it('shows the authoritative completed delivery instead of its initial queued receipt', () => {
  const { rerender } = render(<BotEventCard toolCall={toolCall} />)
  expect(screen.getByText('已受理，等待处理')).toBeInTheDocument()
  live.deliveries = [{
    id: 'dlv-1', exchangeId: 'ex-1', entryId: 'entry-1', roomCycle: 0,
    from: { kind: 'bot', id: 'sender', name: 'Sender' },
    toBotId: 'bee', toName: 'Bee', toConversationId: 'conv_bot_bee',
    kind: 'peer', body: 'hello', depth: 0, status: 'succeeded',
    admitted: true, sensitive: false, receipts: [], createdAt: 1, updatedAt: 2,
    nativeOutput: 'Received', finishedAt: 2,
  }]
  rerender(<BotEventCard toolCall={toolCall} />)
  expect(screen.getByText('已完成')).toBeInTheDocument()
  expect(screen.queryByText('已受理，等待处理')).not.toBeInTheDocument()
})

 it('summarizes Bot updates without exposing profile or memory payloads', () => {
  render(<BotEventCard toolCall={{ id: 'update', toolName: 'bot_memory_update', status: 'completed', result: JSON.stringify({ content: 'private memory content' }) }} />)
  expect(screen.getByText('已完成')).toBeInTheDocument()
  expect(screen.queryByText(/private memory content/)).not.toBeInTheDocument()
})
it('hides successful internal reads but retains their failures', () => {
  const { rerender } = render(<BotEventCard toolCall={{ id: 'read', toolName: 'bot_get', status: 'completed', result: JSON.stringify({ name: 'Bee' }) }} />)
  expect(screen.queryByText('Bee')).not.toBeInTheDocument()
  rerender(<BotEventCard toolCall={{ id: 'read', toolName: 'bot_get', status: 'error', result: JSON.stringify({ error: 'Profile unavailable' }) }} />)
  expect(screen.getByRole('alert')).toHaveTextContent('Profile unavailable')
})

it('does not claim a broadcast is completed before its deliveries finish', () => {
  render(<BotEventCard toolCall={{ id: 'broadcast', toolName: 'bot_room_broadcast', status: 'completed', result: JSON.stringify({roomId: 'room', deliveryIds: ['one', 'two'], skippedBotIds: [], bound: 2}) }} />)
  expect(screen.getByText('已受理，等待处理')).toBeInTheDocument()
  expect(screen.queryByText('已完成')).not.toBeInTheDocument()
})
it('updates a membership request when the pending action is approved', () => {
  const call: ToolCallRecord = {id: 'members', toolName: 'bot_room_members', status: 'completed', result: JSON.stringify({pending: [{id: 'p'}]})}
  const {rerender} = render(<BotEventCard toolCall={call} />)
  live.pending = [{id: 'p', status: 'approved'} as PendingAction]
  rerender(<BotEventCard toolCall={call} />)
  expect(screen.getByText('已完成')).toBeInTheDocument()
})
