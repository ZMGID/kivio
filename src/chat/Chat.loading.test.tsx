import { Profiler } from 'react'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeAll, expect, it, vi } from 'vitest'
import { api } from '../api/tauri'
import type { ChatContextPayload, ChatStreamPayload } from '../api/tauri'
import { CHAT_PROTOCOL_VERSION, type ChatHistoryCursor } from '../generated/chatProtocol'
import { chatApi } from './api'
import { recordChatConversationRevision } from '../api/chatProtocol'
import Chat from './Chat'
import { draftKey, updateComposerDraft } from './composerDraft'
import { getSnapshot, reset } from './streamingStore'
import type { Conversation } from './types'

vi.mock('@xterm/xterm', () => ({ Terminal: class {} }))
vi.mock('@xterm/addon-webgl', () => ({ WebglAddon: class {} }))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ onFocusChanged: async () => () => {} }) }))
vi.mock('../api/settingsCache', async (original) => ({
  ...await original<typeof import('../api/settingsCache')>(),
  getSettingsCached: () => Promise.resolve({ chat: {}, providers: [] }),
}))

beforeAll(() => {
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList
  Range.prototype.getBoundingClientRect = () => new DOMRect()
})

afterEach(() => { act(() => reset()); vi.restoreAllMocks() })

function checkpoint(after: ChatHistoryCursor | undefined, revision: number | null) {
  return { conversationRevision: revision,
    historyReplay: after && after.revision === revision ? { after, through: after, patches: [] } : undefined }
}

function mockConversationPair() {
  for (const key of Object.keys(api) as Array<keyof typeof api>) {
    if (key.startsWith('on') && typeof api[key] === 'function') vi.spyOn(api, key).mockResolvedValue(() => {})
  }
  vi.spyOn(api, 'scheduledTasksSnapshot').mockResolvedValue({ tasks: [], liveRuns: [] })
  const conversations: Conversation[] = ['switch-a', 'switch-b'].map(id => ({
    id, revision: 1, title: id, provider_id: '', model: '', created_at: 1, updated_at: 1, history_start: 0, history_source: '1', history_total: 1,
    messages: [{ id: `${id}-question`, role: 'user', content: `${id} question`, timestamp: 1 }],
  }))
  const read = async (id: string) => conversations.find(conversation => conversation.id === id)!
  vi.spyOn(chatApi, 'getConversations').mockResolvedValue(conversations.map(({ id }) => ({
    id, title: id, preview: '', provider_id: '', model: '', message_count: 1, created_at: 1, updated_at: 1,
  })))
  vi.spyOn(chatApi, 'getProjects').mockResolvedValue([])
  vi.spyOn(chatApi, 'getSets').mockResolvedValue([])
  vi.spyOn(chatApi, 'getAssistants').mockResolvedValue([])
  vi.spyOn(chatApi, 'getConversationPins').mockResolvedValue({})
  const windowReads = vi.spyOn(chatApi, 'getConversationWindow').mockImplementation(read)
  vi.spyOn(chatApi, 'syncConversation').mockImplementation(async (_id, after) => checkpoint(after, 1))
  vi.spyOn(chatApi, 'getConversation').mockImplementation(read)
  vi.spyOn(chatApi, 'getContextStats').mockResolvedValue({ contextState: {} })
  return { conversations, windowReads }
}

it('opens the retained draft immediately and enables sends after replaying a missed title', async () => {
  const { conversations, windowReads } = mockConversationPair()
  window.history.replaceState(null, '', '#chat/switch-a')
  const view = render(<Chat onSettingsChange={() => {}} />)
  await act(async () => { await vi.dynamicImportSettled() })
  await screen.findByText('switch-a question')
  fireEvent.click(await screen.findByRole('button', { name: 'switch-b' }))
  await screen.findByText('switch-b question')
  const send = vi.spyOn(chatApi, 'sendMessage')
  const replay = vi.mocked(chatApi.syncConversation)
  replay.mockClear()
  windowReads.mockClear()
  let finish!: () => void
  replay.mockImplementationOnce((_id, after) => new Promise(resolve => {
    finish = () => resolve({ conversationRevision: 2, historyReplay: {
      after: after!, through: { revision: 2, source: 'second' },
      patches: [{ revision: 2, updatedAt: 2, events: [{ type: 'title_updated', title: '新标题' }] }],
    } })
  }))
  fireEvent.click(await screen.findByRole('button', { name: 'switch-a' }))
  const retained = await screen.findByText('switch-a question')
  expect(window.location.hash).toBe('#chat/switch-a')
  expect(retained.closest('[inert]')).toBeNull()
  const editor = screen.getByRole('textbox')
  expect(editor).toHaveAttribute('contenteditable', 'true')
  fireEvent.paste(editor, { clipboardData: { files: [], getData: (type: string) => type === 'text/plain' ? 'Draft while catching up' : '' } })
  fireEvent.keyDown(editor, { key: 'Enter' })
  expect(send).not.toHaveBeenCalled()
  expect(editor.textContent).toBe('Draft while catching up')
  const summaries = conversations.map(({ id, title }) => ({
    id, title: id === 'switch-a' ? '新标题' : title, preview: '', provider_id: '', model: '',
    message_count: 1, created_at: 1, updated_at: 2,
  }))
  vi.mocked(chatApi.getConversations).mockResolvedValue(summaries)
  await act(async () => finish())
  expect(await screen.findByRole('button', { name: '新标题' })).toBeVisible()
  expect(screen.getByRole('button', { name: /^(Send|发送)$/ })).toBeEnabled()
  expect(screen.getByText('switch-a question')).toBe(retained)
  expect(screen.getByRole('textbox').textContent).toBe('Draft while catching up')
  expect(replay).toHaveBeenCalledTimes(1)
  expect(windowReads).not.toHaveBeenCalled()
  view.unmount()
})

it('releases an unverified preview after stale reads and restores its draft on a fresh selection', async () => {
  const { conversations, windowReads } = mockConversationPair()
  const id = 'warm-stale-target'
  conversations[0] = { ...conversations[0], id, title: id }
  vi.mocked(chatApi.getConversations).mockResolvedValue(conversations.map(({ id }) => ({
    id, title: id, preview: '', provider_id: '', model: '', message_count: 1, created_at: 1, updated_at: 1,
  })))
  window.history.replaceState(null, '', `#chat/${id}`)
  const view = render(<Chat onSettingsChange={() => {}} />)
  await act(async () => { await vi.dynamicImportSettled() })
  await screen.findByText('switch-a question')
  fireEvent.click(await screen.findByRole('button', { name: 'switch-b' }))
  await screen.findByText('switch-b question')
  let resume!: () => void
  vi.mocked(chatApi.syncConversation).mockImplementationOnce(() => new Promise(resolve => {
    resume = () => resolve({ conversationRevision: 2 })
  }))
  fireEvent.click(await screen.findByRole('button', { name: id }))
  await screen.findByText('switch-a question')
  fireEvent.paste(screen.getByRole('textbox'), { clipboardData: {
    files: [], getData: (type: string) => type === 'text/plain' ? 'Only for this conversation' : '',
  } })
  recordChatConversationRevision(id, 2)
  await act(async () => resume())
  expect(window.location.hash).toBe('#chat')
  expect(screen.queryByText('switch-a question')).toBeNull()
  expect(screen.getByRole('textbox')).not.toHaveTextContent('Only for this conversation')
  windowReads.mockResolvedValueOnce({ ...conversations[0], revision: 2 })
  fireEvent.click(await screen.findByRole('button', { name: id }))
  await screen.findByText('switch-a question')
  expect(screen.getByRole('textbox')).toHaveTextContent('Only for this conversation')
  expect(screen.getByRole('button', { name: /^(Send|发送)$/ })).toBeEnabled()
  view.unmount()
})

it('applies a missed edit from a fresh window and rejects a late deletion after switching away', async () => {
  const { conversations, windowReads } = mockConversationPair()
  window.history.replaceState(null, '', '#chat/switch-a')
  const view = render(<Chat onSettingsChange={() => {}} />)
  await act(async () => { await vi.dynamicImportSettled() })
  await screen.findByText('switch-a question')
  fireEvent.click(await screen.findByRole('button', { name: 'switch-b' }))
  await screen.findByText('switch-b question')

  let finish!: (revision: number | null) => void
  const delayReplay = () => vi.mocked(chatApi.syncConversation).mockImplementationOnce(
    (_id, after) => new Promise(resolve => { finish = revision => resolve(checkpoint(after, revision)) }))
  delayReplay()
  fireEvent.click(await screen.findByRole('button', { name: 'switch-a' }))
  await screen.findByText('switch-a question')
  const updated = { ...conversations[0], revision: 2, messages: [
    { id: 'edited', role: 'user' as const, content: 'edited in another window', timestamp: 2 },
  ] }
  windowReads.mockResolvedValueOnce(updated)
  await act(async () => finish(2))
  await screen.findByText('edited in another window')
  expect(screen.queryByText('switch-a question')).not.toBeInTheDocument()

  delayReplay()
  fireEvent.click(await screen.findByRole('button', { name: 'switch-b' }))
  await screen.findByText('switch-b question')
  vi.mocked(chatApi.syncConversation).mockImplementationOnce(async (_id, after) => checkpoint(after, 2))
  fireEvent.click(await screen.findByRole('button', { name: 'switch-a' }))
  await screen.findByText('edited in another window')
  await act(async () => finish(null))
  expect(window.location.hash).toBe('#chat/switch-a')

  windowReads.mockRejectedValueOnce(new Error('Conversation no longer exists'))
  fireEvent.click(await screen.findByRole('button', { name: 'switch-b' }))
  await waitFor(() => expect(window.location.hash).toBe('#chat'))
  expect(screen.queryByText('switch-b question')).not.toBeInTheDocument()
  view.unmount()
})

it('keeps previews, queued sends and pending approvals scoped to their running conversation', async () => {
  const { conversations, windowReads } = mockConversationPair()
  let streamEvent!: (payload: ChatStreamPayload) => void
  let toolApproval!: Parameters<typeof api.onChatToolConfirm>[0]
  let openEvent!: Parameters<typeof api.onChatOpenConversation>[0]
  vi.spyOn(api, 'onChatStream').mockImplementation(async handler => { streamEvent = handler; return () => {} })
  vi.spyOn(api, 'onChatToolConfirm').mockImplementation(async handler => { toolApproval = handler; return () => {} })
  vi.spyOn(api, 'onChatOpenConversation').mockImplementation(async handler => { openEvent = handler; return () => {} })
  const mixedCommits: string[] = []
  const observeCommit = () => {
    const liveMessageId = getSnapshot().messageId
    for (const conversation of conversations) {
      if (liveMessageId && liveMessageId !== `${conversation.id}-answer`
        && document.querySelector(`[data-message-id="${conversation.id}-question"]`)) {
        mixedCommits.push(`${conversation.id}: ${liveMessageId}`)
      }
    }
  }
  window.history.replaceState(null, '', '#chat/switch-a')
  const view = render(<Profiler id="switch" onRender={observeCommit}><Chat onSettingsChange={() => {}} /></Profiler>)
  // The first message pane is lazy-loaded. Wait for that import before inspecting its committed history.
  await act(async () => { await vi.dynamicImportSettled() })
  await screen.findByText('switch-a question')
  await act(async () => {
    for (const conversation of conversations) {
      const envelope = {
        protocolVersion: CHAT_PROTOCOL_VERSION, scope: 'run' as const, conversationId: conversation.id,
        runId: `${conversation.id}-run`, messageId: `${conversation.id}-answer`, seq: 1, baseRevision: 1,
      }
      streamEvent({ ...envelope, type: 'run_started', recovery: null })
      streamEvent({ ...envelope, seq: 2, type: 'text_delta', delta: `${conversation.id} answer`, segment: null })
    }
  })
  await waitFor(() => expect(getSnapshot().messageId).toBe('switch-a-answer'))
  for (const id of ['switch-b', 'switch-a']) {
    await act(async () => openEvent({ conversationId: id }))
    await screen.findByText(`${id} question`)
    await waitFor(() => expect(getSnapshot().messageId).toBe(`${id}-answer`))
  }
  expect(mixedCommits).toEqual([])
  // Both conversations are still running. Leaving one from the sidebar keeps
  // its loaded window, so coming back while it is unchanged reads nothing.
  windowReads.mockClear()
  for (const id of ['switch-b', 'switch-a', 'switch-b']) {
    let finish!: (value: number) => void
    vi.mocked(chatApi.syncConversation).mockImplementationOnce((_id, after) => new Promise(resolve => { finish = revision => resolve(checkpoint(after, revision)) }))
    fireEvent.click(await screen.findByRole('button', { name: id }))
    await screen.findByText(`${id} question`)
    await waitFor(() => expect(getSnapshot().messageId).toBe(`${id}-answer`))
    expect(window.location.hash).toBe(`#chat/${id}`)
    await act(async () => finish(1))
  }
  expect(mixedCommits).toEqual([])
  expect(windowReads).not.toHaveBeenCalled()
  expect(chatApi.getConversation).not.toHaveBeenCalled()

  const input = screen.getByRole('textbox')
  act(() => updateComposerDraft(draftKey('switch-b'), (draft) => ({ ...draft, input: 'Queued for B' })))
  fireEvent.keyDown(input, { key: 'Enter' })
  expect((await screen.findByText('Queued for B')).closest('[data-chat-message-list-item="injected-user"]')).toBeInTheDocument()
  fireEvent.click(await screen.findByRole('button', { name: 'switch-a' }))
  await screen.findByText('switch-a question')

  const finished = { ...conversations[1], revision: 2, messages: [
    ...conversations[1].messages,
    { id: 'switch-b-answer', role: 'assistant' as const, content: 'Finished B', timestamp: 2 },
  ] }
  vi.mocked(chatApi.getConversation).mockResolvedValueOnce(finished)
  const send = vi.spyOn(chatApi, 'sendMessage').mockResolvedValue(finished)
  await act(async () => streamEvent({
    protocolVersion: CHAT_PROTOCOL_VERSION, scope: 'run', conversationId: 'switch-b',
    runId: 'switch-b-run', messageId: 'switch-b-answer', seq: 3, baseRevision: 1,
    type: 'run_completed', full: 'Finished B', conversationRevision: 2,
  }))
  expect(send.mock.calls[0]?.slice(0, 2)).toEqual(['switch-b', 'Queued for B'])
  expect(screen.getByText('switch-a question')).toBeVisible()
  expect(getSnapshot().messageId).toBe('switch-a-answer')

  let failStop!: (error: Error) => void
  const cancel = vi.spyOn(chatApi, 'cancelStream').mockImplementation(() => new Promise((_, reject) => { failStop = reject }))
  await act(async () => toolApproval({
    conversationId: 'switch-a', runId: 'switch-a-run', interactionId: 'approval-a', toolCallId: 'tool-a',
    name: 'fixture-tool', source: 'native', argumentsPreview: 'An approval for A',
  }))
  expect(await screen.findByText('An approval for A')).toBeVisible()
  fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Escape' })
  await waitFor(() => expect(cancel).toHaveBeenCalledWith('switch-a'))
  expect(screen.queryByText('An approval for A')).not.toBeInTheDocument()
  await act(async () => failStop(new Error('Stop request failed')))
  expect(await screen.findByText('An approval for A')).toBeVisible()
  expect(getSnapshot().streaming).toBe(true)
  view.unmount()
})

it('keeps sidebar reads idle while the open conversation receives context updates', async () => {
  // Native event transport is the seam; Chat, routing, context and Sidebar are real.
  for (const key of Object.keys(api) as Array<keyof typeof api>) {
    if (key.startsWith('on') && typeof api[key] === 'function') {
      vi.spyOn(api, key).mockResolvedValue(() => {})
    }
  }
  vi.spyOn(api, 'scheduledTasksSnapshot').mockResolvedValue({ tasks: [], liveRuns: [] })
  let contextEvent!: (payload: ChatContextPayload) => void
  vi.spyOn(api, 'onChatContext').mockImplementation(async handler => {
    contextEvent = handler
    return () => {}
  })
  const reads = vi.spyOn(chatApi, 'getConversations').mockResolvedValue([])
  vi.spyOn(chatApi, 'getProjects').mockResolvedValue([])
  vi.spyOn(chatApi, 'getSets').mockResolvedValue([])
  vi.spyOn(chatApi, 'getAssistants').mockResolvedValue([])
  vi.spyOn(chatApi, 'getConversationPins').mockResolvedValue({})
  const conversation = {
    id: 'conv_loading_test', revision: 1, title: 'Loading regression', provider_id: '', model: '',
    messages: [], created_at: 1, updated_at: 1,
  }
  vi.spyOn(chatApi, 'getConversationWindow').mockResolvedValue(conversation)
  vi.spyOn(chatApi, 'getContextStats').mockResolvedValue({ contextState: {} })
  window.history.replaceState(null, '', '#chat/conv_loading_test')
  const view = render(<Chat onSettingsChange={() => {}} />)
  await screen.findByText('Loading regression')
  await act(async () => {})
  const loaded = reads.mock.calls.length
  for (let usedTokens = 1; usedTokens <= 10; usedTokens++) {
    await act(async () => contextEvent({ conversationId: conversation.id, live: { usedTokens, contextWindowTokens: 1000 } }))
  }
  expect(reads.mock.calls.length).toBe(loaded)
  expect(window.location.hash).toBe('#chat/conv_loading_test')
  view.unmount()
})
