import { Profiler } from 'react'
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { api } from '../api/tauri'
import type { ChatContextPayload, ChatStreamPayload } from '../api/tauri'
import { CHAT_PROTOCOL_VERSION } from '../generated/chatProtocol'
import { chatApi } from './api'
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

afterEach(() => { act(() => reset()); vi.restoreAllMocks() })

function mockConversationPair() {
  for (const key of Object.keys(api) as Array<keyof typeof api>) {
    if (key.startsWith('on') && typeof api[key] === 'function') vi.spyOn(api, key).mockResolvedValue(() => {})
  }
  vi.spyOn(api, 'chatSyncState').mockResolvedValue(undefined)
  vi.spyOn(api, 'scheduledTasksSnapshot').mockResolvedValue({ tasks: [], liveRuns: [] })
  const conversations: Conversation[] = ['switch-a', 'switch-b'].map(id => ({
    id, revision: 1, title: id, provider_id: '', model: '', created_at: 1, updated_at: 1,
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
  vi.spyOn(chatApi, 'getConversationRevision').mockResolvedValue(1)
  vi.spyOn(chatApi, 'getConversation').mockImplementation(read)
  vi.spyOn(chatApi, 'getContextStats').mockResolvedValue({ contextState: {} })
  return { conversations, windowReads }
}

it('shows cached history read-only until validation, and cancels it when the open chat is reselected', async () => {
  const { windowReads } = mockConversationPair()
  window.history.replaceState(null, '', '#chat/switch-a')
  const readOnlyCommits: Element[] = []
  const observeCommit = () => {
    const preview = document.querySelector('[inert] .chat-message-list')
    if (preview) readOnlyCommits.push(preview)
  }
  const view = render(<Profiler id="warm-return" onRender={observeCommit}><Chat onSettingsChange={() => {}} /></Profiler>)
  await act(async () => { await vi.dynamicImportSettled() })
  await screen.findByText('switch-a question')
  fireEvent.click(await screen.findByRole('button', { name: 'switch-b' }))
  await screen.findByText('switch-b question')
  // Hold the repository reply. Cached history can paint, but the old route
  // remains authoritative and message actions stay inert until validation.
  let validate!: (revision: number) => void
  vi.mocked(chatApi.getConversationRevision).mockImplementationOnce(() => new Promise(done => { validate = done }))
  fireEvent.click(await screen.findByRole('button', { name: 'switch-a' }))
  const preview = await screen.findByText('switch-a question')
  expect(preview.closest('[inert]')).not.toBeNull()
  expect(window.location.hash).toBe('#chat/switch-b')
  await act(async () => validate(1))
  expect(screen.getByText('switch-a question')).toBe(preview)
  expect(preview.closest('[inert]')).toBeNull()
  expect(window.location.hash).toBe('#chat/switch-a')

  // A late revision change from B cannot roll back a re-selection of A.
  vi.mocked(chatApi.getConversationRevision).mockImplementationOnce(() => new Promise(done => { validate = done }))
  fireEvent.click(await screen.findByRole('button', { name: 'switch-b' }))
  expect((await screen.findByText('switch-b question')).closest('[inert]')).not.toBeNull()
  fireEvent.click(await screen.findByRole('button', { name: 'switch-a' }))
  await screen.findByText('switch-a question')
  await act(async () => validate(2))
  expect(window.location.hash).toBe('#chat/switch-a')
  expect(screen.getByText('switch-a question').closest('[inert]')).toBeNull()
  expect(screen.queryByText('switch-b question')).not.toBeInTheDocument()

  // B was invalidated. A subsequent visit must show its new durable content.
  const fresh = {
    id: 'switch-b', title: 'switch-b', revision: 2, provider_id: '', model: '', created_at: 1, updated_at: 2,
    messages: [{ id: 'b-new', role: 'user' as const, content: 'new B question', timestamp: 2 }],
  }
  windowReads.mockResolvedValue(fresh)
  fireEvent.click(await screen.findByRole('button', { name: 'switch-b' }))
  await screen.findByText('new B question')
  expect(window.location.hash).toBe('#chat/switch-b')

  expect(readOnlyCommits.length).toBeGreaterThan(0)
  readOnlyCommits.length = 0
  fireEvent.click(await screen.findByRole('button', { name: 'switch-a' }))
  await screen.findByText('switch-a question')
  expect(window.location.hash).toBe('#chat/switch-a')
  expect(readOnlyCommits).toHaveLength(0)
  view.unmount()
})

it('withdraws deleted cached history while the definitive read is still pending', async () => {
  const { windowReads } = mockConversationPair()
  window.history.replaceState(null, '', '#chat/switch-a')
  const view = render(<Chat onSettingsChange={() => {}} />)
  await act(async () => { await vi.dynamicImportSettled() })
  await screen.findByText('switch-a question')
  fireEvent.click(await screen.findByRole('button', { name: 'switch-b' }))
  await screen.findByText('switch-b question')

  let missing!: (revision: null) => void
  vi.mocked(chatApi.getConversationRevision).mockImplementationOnce(() => new Promise(done => { missing = done }))
  let failRead!: (error: Error) => void
  windowReads.mockImplementationOnce(() => new Promise((_, reject) => { failRead = reject }))
  fireEvent.click(await screen.findByRole('button', { name: 'switch-a' }))
  expect((await screen.findByText('switch-a question')).closest('[inert]')).not.toBeNull()
  await act(async () => missing(null))
  expect(screen.queryByText('switch-a question')).not.toBeInTheDocument()
  await act(async () => failRead(new Error('conversation no longer exists')))
  await waitFor(() => expect(window.location.hash).toBe('#chat'))
  expect(screen.queryByText('switch-a question')).not.toBeInTheDocument()
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
    fireEvent.click(await screen.findByRole('button', { name: id }))
    await screen.findByText(`${id} question`)
    await waitFor(() => expect(getSnapshot().messageId).toBe(`${id}-answer`))
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
  vi.spyOn(api, 'chatSyncState').mockResolvedValue(undefined)
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
