// @vitest-environment jsdom
import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, expect, it, vi } from 'vitest'
import { BotEventCard } from './BotEventCard'
import { subscribePeerExchange } from './exchangeRequest'
import type { ToolCallRecord } from '../types'
import type { Delivery } from './collaboration'

const live = vi.hoisted(() => ({ deliveries: [] as Delivery[] }))
vi.mock('./collaborationStore', () => ({
  useBotCollaboration: () => ({ snapshot: { deliveries: live.deliveries } }),
}))
beforeEach(() => { live.deliveries = [] })

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
