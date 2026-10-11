// @vitest-environment jsdom
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, expect, it, vi } from 'vitest'
import { api } from '../api/tauri'
import { BotRoster } from './BotRoster'
import { chatApi } from './api'
import { profileApi } from './bot/profile'
import type { ConversationListItem } from './types'

vi.mock('./bot/collaborationStore', () => ({
  refreshCollaboration: vi.fn(),
  useBotCollaboration: () => ({ error: '', snapshot: {
    rooms: [{ id: 'room-research', title: '研究讨论', status: 'active', cycle: 0, members: [], log: [], createdAt: 1, updatedAt: 20 }],
    pending: [], deliveries: [],
  } }),
}))
afterEach(() => vi.restoreAllMocks())

it('mixes group chats with Bots by recency and opens each through its own chat entry', async () => {
  const conversations: ConversationListItem[] = [
    { id: 'conv_bot_old', title: '旧 Bot', preview: '', created_at: 1, updated_at: 10, provider_id: '', model: '', message_count: 0 },
    { id: 'conv_bot_new', title: '新 Bot', preview: '', created_at: 1, updated_at: 30, provider_id: '', model: '', message_count: 0 },
  ]
  vi.spyOn(chatApi, 'getBotConversations').mockResolvedValue(conversations)
  vi.spyOn(chatApi, 'getAssistants').mockResolvedValue([])
  vi.spyOn(api, 'onChatAssistantsChanged').mockResolvedValue(() => {})
  vi.spyOn(profileApi, 'onChanged').mockResolvedValue(() => {})
  const opened: string[] = []
  const user = userEvent.setup()
  render(<BotRoster lang="zh" currentConversationId={undefined} refreshKey={0} searchOpen={false}
    onSelectConversation={id => opened.push(id)} onOpenRoom={id => opened.push(id)}
    onOpenSettings={() => {}} onOpenExtensionsItem={() => {}} onNewBot={() => {}} footer={null} />)
  await screen.findByRole('button', { name: /新 Bot/ })
  expect(screen.queryByText('群组')).not.toBeInTheDocument()
  const entries = screen.getAllByRole('button').filter(button => /新 Bot|研究讨论|旧 Bot/.test(button.textContent ?? ''))
  expect(entries.map(button => button.textContent)).toEqual([expect.stringContaining('新 Bot'), expect.stringContaining('研究讨论'), expect.stringContaining('旧 Bot')])
  await user.click(entries[1])
  await user.click(entries[0])
  await waitFor(() => expect(opened).toEqual(['room-research', 'conv_bot_new']))
})
