// @vitest-environment jsdom
import { act, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
  on: vi.fn(async () => () => {}),
}))
import { i18n } from '../../components/i18n'
import { createBlankAssistant } from '../assistantDraftStore'
import { chatApi } from '../api'
import { BotRoster } from '../BotRoster'
import { BotGroupPane } from './BotGroupPane'
import { collaborationApi, type Exchange, type Room, type RoomMember } from './collaboration'
import { resetCollaborationForTests } from './collaborationStore'
import { PeerExchangeView } from './PeerExchangeView'
import { profileApi, type BotListItem } from './profile'

const t = i18n.zh

function member(botId: string, name: string, conversationId = `conv_bot_${botId}`): RoomMember {
  return { botId, name, conversationId, state: 'active', joinedAt: 1 }
}

function room(id: string, title: string, members: RoomMember[]): Room {
  return { id, title, status: 'active', cycle: 1, members, log: [], createdAt: 1, updatedAt: 1 }
}

function exchange(id: string, entries: { id: string; body: string; at: number }[]): Exchange {
  return {
    id,
    entries: entries.map((entry) => ({
      id: entry.id,
      at: entry.at,
      actor: { kind: 'bot', id: 'ada', name: 'Ada' },
      kind: 'reply',
      body: entry.body,
      deliveryId: entry.id,
      status: 'succeeded',
    })),
  }
}

function collaboration(rooms: Room[], exchanges: Exchange[] = []) {
  return { deliveries: [], rooms, exchanges, pending: [] }
}

function listedBot(assistantId: string, name: string): BotListItem {
  return {
    assistantId,
    name,
    conversationId: `conv_bot_${assistantId}`,
    primary: false,
    persistedProfile: true,
    capabilitySource: 'granted_defaults',
    capabilityExplanation: '',
    modelSource: 'inherit_global',
    approvalSource: 'inherit_global',
  }
}

let publish: (payload: unknown) => void

beforeEach(() => {
  resetCollaborationForTests()
  publish = () => {}
  HTMLDialogElement.prototype.showModal = function showModal() { this.setAttribute('open', '') }
  vi.spyOn(profileApi, 'list').mockResolvedValue([])
  vi.spyOn(collaborationApi, 'onChanged').mockImplementation(async (listener) => {
    publish = listener
    return () => {}
  })
  vi.spyOn(collaborationApi, 'snapshot').mockResolvedValue(collaboration([]))
  vi.spyOn(collaborationApi, 'exchange').mockResolvedValue({ id: 'none', entries: [] })
})

afterEach(() => {
  delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__
  vi.restoreAllMocks()
  resetCollaborationForTests()
})

it('keeps each room draft and title, and a late send cannot clear the other room', async () => {
  let release: () => void = () => {}
  const gate = new Promise<void>((resolve) => { release = resolve })
  vi.spyOn(collaborationApi, 'snapshot').mockResolvedValue(collaboration([
    room('alpha', 'Alpha', [member('ada', 'Ada')]),
    room('beta', 'Beta', [member('bee', 'Bee')]),
  ]))
  vi.spyOn(collaborationApi, 'broadcast').mockImplementation(() => gate.then(() => ({})))
  const view = render(<BotGroupPane roomId="alpha" onClose={() => {}} />)
  const composer = await screen.findByPlaceholderText(t.chatBotGroupComposer)
  fireEvent.change(composer, { target: { value: 'only alpha' } })
  fireEvent.click(screen.getByRole('button', { name: t.chatBotRenameGroup }))
  fireEvent.change(screen.getByLabelText(t.chatBotGroupTitle), { target: { value: 'Alpha draft' } })

  view.rerender(<BotGroupPane roomId="beta" onClose={() => {}} />)
  expect(screen.getByRole('heading', { name: 'Beta' })).toBeInTheDocument()
  expect(screen.getByPlaceholderText(t.chatBotGroupComposer)).toHaveValue('')
  expect(screen.queryByDisplayValue('Alpha draft')).not.toBeInTheDocument()
  fireEvent.change(screen.getByPlaceholderText(t.chatBotGroupComposer), { target: { value: 'only beta' } })

  view.rerender(<BotGroupPane roomId="alpha" onClose={() => {}} />)
  expect(screen.getByLabelText(t.chatBotGroupTitle)).toHaveValue('Alpha draft')
  expect(screen.getByPlaceholderText(t.chatBotGroupComposer)).toHaveValue('only alpha')
  fireEvent.click(screen.getByRole('button', { name: t.chatBotBroadcast }))

  view.rerender(<BotGroupPane roomId="beta" onClose={() => {}} />)
  expect(screen.getByPlaceholderText(t.chatBotGroupComposer)).toHaveValue('only beta')
  await act(async () => { release() })
  expect(screen.getByPlaceholderText(t.chatBotGroupComposer)).toHaveValue('only beta')
  view.rerender(<BotGroupPane roomId="alpha" onClose={() => {}} />)
  expect(screen.getByPlaceholderText(t.chatBotGroupComposer)).toHaveValue('')
})

it('retries a partial @mention only for recipients that were not accepted', async () => {
  const calls: string[] = []
  vi.spyOn(collaborationApi, 'snapshot').mockResolvedValue(collaboration([
    room('alpha', 'Alpha', [member('ada', 'Ada'), member('bee', 'Bee')]),
  ]))
  vi.spyOn(collaborationApi, 'mention').mockImplementation(async (_roomId, botId) => {
    calls.push(botId)
    if (botId === 'bee') throw new Error('busy')
    return {}
  })
  render(<BotGroupPane roomId="alpha" onClose={() => {}} />)
  fireEvent.change(await screen.findByPlaceholderText(t.chatBotGroupComposer), { target: { value: '@Ada @Bee hello' } })
  fireEvent.click(screen.getByRole('button', { name: t.chatBotKindMention }))
  expect(await screen.findByRole('alert')).toHaveTextContent('busy')
  expect(calls).toEqual(['ada', 'bee'])
  fireEvent.click(screen.getByRole('button', { name: t.chatBotKindMention }))
  await act(async () => { await Promise.resolve() })
  expect(calls).toEqual(['ada', 'bee', 'bee'])
})

it('offers only bot profile contacts in the group member selector', async () => {
  vi.spyOn(collaborationApi, 'snapshot').mockResolvedValue(collaboration([
    room('alpha', 'Alpha', [member('ada', 'Ada')]),
  ]))
  vi.spyOn(profileApi, 'list').mockResolvedValue([listedBot('ada', 'Ada'), listedBot('cara', 'Cara')])
  vi.spyOn(chatApi, 'getAssistants').mockResolvedValue([
    { ...createBlankAssistant('Ordinary'), id: 'ord', name: 'Ordinary' },
  ])
  render(<BotGroupPane roomId="alpha" onClose={() => {}} />)
  await screen.findByRole('heading', { name: 'Alpha' })
  fireEvent.click(await screen.findByRole('button', { name: t.chatBotAddMember, expanded: false }))
  expect(await screen.findByRole('option', { name: 'Cara' })).toBeInTheDocument()
  expect(screen.queryByRole('option', { name: 'Ordinary' })).not.toBeInTheDocument()
  expect(screen.queryByRole('option', { name: 'Ada' })).not.toBeInTheDocument()
})

it('updates an open exchange from the collaboration store and ignores a late query', async () => {
  const hello = exchange('ex-1', [{ id: 'e1', body: 'first note', at: 1 }])
  const replied = exchange('ex-1', [
    { id: 'e1', body: 'first note', at: 1 },
    { id: 'e2', body: 'actual reply', at: 2 },
  ])
  const other = exchange('ex-2', [{ id: 'e3', body: 'other room', at: 3 }])
  let resolveFirst: (value: unknown) => void = () => {}
  const firstQuery = new Promise((resolve) => { resolveFirst = resolve })
  vi.spyOn(collaborationApi, 'snapshot').mockResolvedValue(collaboration([], [hello]))
  vi.spyOn(collaborationApi, 'exchange').mockImplementation((id: string) => (
    id === 'ex-1' ? firstQuery : Promise.resolve(other)
  ))
  const onClose = vi.fn()
  const view = render(<PeerExchangeView exchangeId="ex-1" onClose={onClose} />)
  expect(await screen.findByText('first note')).toBeInTheDocument()
  act(() => { publish(collaboration([], [replied])) })
  expect(await screen.findByText('actual reply')).toBeInTheDocument()
  await act(async () => { resolveFirst(exchange('ex-1', [{ id: 'stale', body: 'stale secret', at: 9 }])) })
  expect(screen.getByText('actual reply')).toBeInTheDocument()
  expect(screen.queryByText('stale secret')).not.toBeInTheDocument()

  act(() => { publish(collaboration([], [replied, other])) })
  view.rerender(<PeerExchangeView exchangeId="ex-2" onClose={onClose} />)
  expect(await screen.findByText('other room')).toBeInTheDocument()
  expect(screen.queryByText('actual reply')).not.toBeInTheDocument()
  expect(screen.queryByText('stale secret')).not.toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: t.chatBotExchangeClose }))
  fireEvent.keyDown(window, { key: 'Escape' })
  expect(onClose).toHaveBeenCalledTimes(2)
})

it('renders colleague messages as readable Markdown with each historical sender', async () => {
  const conversation = exchange('ex-markdown', [
    { id: 'request', body: '请整理新闻', at: 1 },
    { id: 'reply', body: '## 新闻摘要\n\n**重要动态**\n\n- 更新发布\n\n[官方来源](https://example.com/news)', at: 2 },
  ])
  conversation.entries[1].actor = { kind: 'bot', id: 'bee', name: 'Bee' }
  vi.spyOn(collaborationApi, 'snapshot').mockResolvedValue(collaboration([], [conversation]))
  render(<PeerExchangeView exchangeId="ex-markdown" onClose={() => {}} />)
  expect(await screen.findByRole('heading', { name: '新闻摘要' })).toBeInTheDocument()
  expect(screen.getByRole('listitem')).toHaveTextContent('更新发布')
  expect(screen.getByRole('link', { name: '官方来源' })).toHaveAttribute('href', 'https://example.com/news')
  expect(screen.getByText('请整理新闻').closest('article')).toHaveTextContent('Ada')
  expect(screen.getByRole('heading', { name: '新闻摘要' }).closest('article')).toHaveTextContent('Bee')
})

it('uses the current directory name for active members and keeps the history author', async () => {
  const group = room('alpha', 'Alpha', [member('ada', 'Ada')])
  group.log = [{ id: 'log-1', at: 1, actor: { kind: 'bot', id: 'ada', name: 'Ada' }, kind: 'message', body: 'historical line' }]
  vi.spyOn(collaborationApi, 'snapshot').mockResolvedValue(collaboration([group]))
  vi.spyOn(profileApi, 'list').mockResolvedValue([listedBot('ada', 'AdaPrime')])
  const mention = vi.spyOn(collaborationApi, 'mention').mockResolvedValue({})
  render(<BotGroupPane roomId="alpha" onClose={() => {}} />)
  expect(await screen.findByRole('checkbox', { name: 'AdaPrime' })).toBeInTheDocument()
  const history = screen.getByText('historical line').closest('article')
  expect(history?.textContent).toMatch(/Ada ·/)
  expect(history?.textContent).not.toContain('AdaPrime')
  fireEvent.change(screen.getByPlaceholderText(t.chatBotGroupComposer), { target: { value: '@AdaPrime hello' } })
  fireEvent.click(screen.getByRole('button', { name: t.chatBotKindMention }))
  expect(mention).toHaveBeenCalledWith('alpha', 'ada', '@AdaPrime hello')
})

it('shows a renamed bot on the group row from the directory', async () => {
  Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {}, configurable: true })
  vi.spyOn(chatApi, 'getAssistants').mockResolvedValue([])
  vi.spyOn(chatApi, 'getBotConversations').mockResolvedValue([])
  vi.spyOn(profileApi, 'list').mockResolvedValue([listedBot('ada', 'AdaPrime')])
  vi.spyOn(collaborationApi, 'snapshot').mockResolvedValue(collaboration([
    room('alpha', 'Alpha', [member('ada', 'Ada')]),
  ]))
  render(
    <BotRoster
      lang="zh"
      refreshKey={0}
      searchOpen={false}
      onSelectConversation={() => {}}
      onOpenSettings={() => {}}
      onOpenExtensionsItem={() => {}}
      footer={null}
      onNewBot={() => {}}
    />,
  )
  const row = await screen.findByRole('button', { name: /Alpha/ })
  expect(row).toHaveTextContent('AdaPrime')
  expect(row.textContent).not.toMatch(/\bAda\b/)
})

it('shows private approval attention ahead of working, including outside the open group', async () => {
  const conversations = [
    { id: 'conv_bot_ada', title: 'Ada', preview: 'quiet', provider_id: '', model: '', message_count: 1, created_at: 1, updated_at: 1 },
    { id: 'conv_bot_ord', title: 'Ordinary', preview: '', provider_id: '', model: '', message_count: 0, created_at: 1, updated_at: 1 },
  ]
  const assistantsReady = Promise.resolve([
    { ...createBlankAssistant('Ordinary'), id: 'ord', name: 'Ordinary' },
    { ...createBlankAssistant('Ada'), id: 'ada', name: 'Ada' },
  ])
  Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {}, configurable: true })
  vi.spyOn(chatApi, 'getAssistants').mockReturnValue(assistantsReady)
  vi.spyOn(chatApi, 'getBotConversations').mockResolvedValue(conversations)
  let resolveList: (items: BotListItem[]) => void = () => {}
  vi.spyOn(profileApi, 'list').mockReturnValue(new Promise((resolve) => { resolveList = resolve }))
  vi.spyOn(collaborationApi, 'snapshot').mockResolvedValue(collaboration([
    room('beta', 'Beta', [member('bee', 'Bee')]),
    room('gamma', 'Gamma', [member('cara', 'Cara')]),
  ]))
  render(
    <BotRoster
      lang="zh"
      refreshKey={0}
      searchOpen={false}
      generatingConversationIds={new Set(['conv_bot_ada', 'conv_bot_bee', 'conv_bot_cara'])}
      attentionConversationIds={new Set(['conv_bot_ada', 'conv_bot_bee'])}
      activeRoomId="gamma"
      onSelectConversation={() => {}}
      onOpenSettings={() => {}}
      onOpenExtensionsItem={() => {}}
      footer={null}
      onNewBot={() => {}}
    />,
  )
  await act(async () => { await assistantsReady })
  fireEvent.click(await screen.findByRole('button', { name: t.chatBotNewGroup }))
  expect(screen.queryByRole('checkbox', { name: 'Ordinary' })).not.toBeInTheDocument()
  await act(async () => { resolveList([listedBot('ada', 'Ada'), listedBot('bee', 'Bee'), listedBot('cara', 'Cara')]) })
  expect(await screen.findByRole('checkbox', { name: 'Ada' })).toBeInTheDocument()
  expect(screen.getByRole('checkbox', { name: 'Bee' })).toBeInTheDocument()
  expect(screen.queryByRole('checkbox', { name: 'Ordinary' })).not.toBeInTheDocument()
  expect(screen.getByRole('button', { name: /Beta/ })).toHaveTextContent(t.chatBotNeedsYou)
  expect(screen.getByRole('button', { name: /Beta/ })).not.toHaveTextContent(t.chatBotWorking)
  expect(screen.getByRole('button', { name: /Gamma/ })).toHaveTextContent(t.chatBotWorking)
  expect(screen.getByRole('button', { name: /Ada/ })).toHaveTextContent(t.chatBotNeedsYou)
  expect(screen.getByRole('button', { name: /Ada/ })).not.toHaveTextContent(t.chatBotWorking)
})
