// @vitest-environment jsdom
import { act, render, screen } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { chatApi } from './api'
import { createBlankAssistant } from './assistantDraftStore'
import { BotConversationHeader } from './BotConversationHeader'
import { collaborationApi } from './bot/collaboration'
import { refreshCollaboration, resetCollaborationForTests } from './bot/collaborationStore'
import { patchSnapshot, reset } from './streamingStore'

afterEach(() => { resetCollaborationForTests(); reset(); localStorage.clear(); vi.restoreAllMocks() })

it('uses real execution evidence and lets user attention win over any active phase', async () => {
  const assistant = await chatApi.createAssistant(createBlankAssistant('研究员'))
  const conversation = await chatApi.openBot(assistant.id)
  const { rerender } = render(<BotConversationHeader conversation={conversation} active onEdit={() => {}} detailsOpen={false} />)
  expect(await screen.findByRole('button', { name: /工作中/ })).toBeVisible()
  act(() => { patchSnapshot({ streaming: true, reasoningStreaming: true }) })
  expect(screen.getByRole('button', { name: /思考中/ })).toBeVisible()
  rerender(<BotConversationHeader conversation={conversation} active attention error onEdit={() => {}} detailsOpen={false} />)
  expect(screen.getByRole('button', { name: /等你处理/ })).toBeVisible()
  rerender(<BotConversationHeader conversation={conversation} active error onEdit={() => {}} detailsOpen={false} />)
  expect(screen.getByRole('button', { name: /遇到问题/ })).toBeVisible()
})


it('celebrates new collaboration success despite previous private success, and does not celebrate a later cancelled delivery', async () => {
  resetCollaborationForTests()
  vi.spyOn(collaborationApi, 'onChanged').mockResolvedValue(() => {})
  const assistant = await chatApi.createAssistant(createBlankAssistant('研究员'))
  const conversation = await chatApi.openBot(assistant.id)
  conversation.messages.push({ id: 'old-private-message', role: 'assistant', content: '旧交付', timestamp: 1, stream_outcome: 'completed' })
  const delivery = { id: 'new-peer', exchangeId: 'ex', entryId: 'ent', roomCycle: 1,
    from: { kind: 'user' }, toBotId: assistant.id, toName: assistant.name, toConversationId: conversation.id,
    kind: 'peer', body: 'hello', depth: 0, status: 'running', admitted: false, sensitive: false,
    receipts: [], createdAt: 1, updatedAt: 1, attachmentIds: [] }
  const read = vi.spyOn(collaborationApi, 'snapshot').mockResolvedValue({ deliveries: [delivery], rooms: [], exchanges: [], pending: [] })
  render(<BotConversationHeader conversation={conversation} executionFeedback={{ status: 'completed', key: 'old-private' }} onEdit={() => {}} detailsOpen={false} />)
  expect(await screen.findByRole('button', { name: /工作中/ })).toBeVisible()
  const success = { ...delivery, status: 'succeeded', updatedAt: 2 }
  read.mockResolvedValue({ deliveries: [success], rooms: [], exchanges: [], pending: [] })
  await act(async () => { await refreshCollaboration() })
  expect(screen.getByRole('button', { name: /刚刚完成/ })).toBeVisible()
  const next = { ...delivery, id: 'next', status: 'running', updatedAt: 3 }
  read.mockResolvedValue({ deliveries: [success, next], rooms: [], exchanges: [], pending: [] })
  await act(async () => { await refreshCollaboration() })
  read.mockResolvedValue({ deliveries: [success, { ...next, status: 'cancelled', updatedAt: 4 }], rooms: [], exchanges: [], pending: [] })
  await act(async () => { await refreshCollaboration() })
  expect(screen.getByRole('button', { name: /安静/ })).toBeVisible()
})
