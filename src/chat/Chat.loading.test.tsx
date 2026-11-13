import { Profiler } from 'react'
import { act, render, screen, waitFor } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { api } from '../api/tauri'
import type { ChatContextPayload, ChatStreamPayload } from '../api/tauri'
import { CHAT_PROTOCOL_VERSION } from '../generated/chatProtocol'
import { chatApi } from './api'
import Chat from './Chat'
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

it('commits conversation history and its live preview together when switching', async () => {
  for (const key of Object.keys(api) as Array<keyof typeof api>) {
    if (key.startsWith('on') && typeof api[key] === 'function') vi.spyOn(api, key).mockResolvedValue(() => {})
  }
  vi.spyOn(api, 'chatSyncState').mockResolvedValue(undefined)
  vi.spyOn(api, 'scheduledTasksSnapshot').mockResolvedValue({ tasks: [], liveRuns: [] })
  let streamEvent!: (payload: ChatStreamPayload) => void
  let openEvent!: Parameters<typeof api.onChatOpenConversation>[0]
  vi.spyOn(api, 'onChatStream').mockImplementation(async handler => { streamEvent = handler; return () => {} })
  vi.spyOn(api, 'onChatOpenConversation').mockImplementation(async handler => { openEvent = handler; return () => {} })
  const conversations: Conversation[] = ['switch-a', 'switch-b'].map(id => ({
    id, revision: 1, title: id, provider_id: '', model: '', created_at: 1, updated_at: 1,
    messages: [{ id: `${id}-question`, role: 'user', content: `${id} question`, timestamp: 1 }],
  }))
  const read = async (id: string) => conversations.find(conversation => conversation.id === id)!
  vi.spyOn(chatApi, 'getConversations').mockResolvedValue([])
  vi.spyOn(chatApi, 'getProjects').mockResolvedValue([])
  vi.spyOn(chatApi, 'getSets').mockResolvedValue([])
  vi.spyOn(chatApi, 'getAssistants').mockResolvedValue([])
  vi.spyOn(chatApi, 'getConversationPins').mockResolvedValue({})
  vi.spyOn(chatApi, 'getConversationWindow').mockImplementation(read)
  vi.spyOn(chatApi, 'getConversation').mockImplementation(read)
  vi.spyOn(chatApi, 'getContextStats').mockImplementation(async id => ({ contextState: {}, conversation: await read(id) }))
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
  vi.spyOn(chatApi, 'getContextStats').mockResolvedValue({ contextState: {}, conversation })
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
