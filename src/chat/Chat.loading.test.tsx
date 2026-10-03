import { act, render, screen } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { api } from '../api/tauri'
import type { ChatContextPayload } from '../api/tauri'
import { chatApi } from './api'
import Chat from './Chat'

vi.mock('@xterm/xterm', () => ({ Terminal: class {} }))
vi.mock('@xterm/addon-webgl', () => ({ WebglAddon: class {} }))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ onFocusChanged: async () => () => {} }) }))
vi.mock('../api/settingsCache', async (original) => ({
  ...await original<typeof import('../api/settingsCache')>(),
  getSettingsCached: () => Promise.resolve({ chat: {}, providers: [] }),
}))

afterEach(() => vi.restoreAllMocks())

it('keeps sidebar reads idle while the open conversation receives context updates', async () => {
  // Native event transport is the seam; Chat, routing, context and Sidebar are real.
  for (const key of Object.keys(api) as Array<keyof typeof api>) {
    if (key.startsWith('on') && typeof api[key] === 'function') {
      vi.spyOn(api, key).mockResolvedValue(() => {})
    }
  }
  vi.spyOn(api, 'chatSyncState').mockResolvedValue(undefined)
  vi.spyOn(api, 'scheduledTasksList').mockResolvedValue([])
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
