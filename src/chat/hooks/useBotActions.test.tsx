import { act, renderHook } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { chatApi } from '../api'
import { useBotActions } from './useBotActions'
import type { Conversation } from '../types'

afterEach(() => vi.restoreAllMocks())

it('refreshes Bot configuration without losing loaded history or messages arriving during the read', async () => {
  const messages = Array.from({ length: 80 }, (_, i) => ({ id: `m${i}`, role: 'user' as const, content: `问题 ${i}`, timestamp: i }))
  const ref: { current: Conversation | null } = { current: {
    id: 'conv_bot_research', revision: 3, title: '研究员', provider_id: 'p', model: 'm',
    created_at: 1, updated_at: 1, messages, history_start: 0, history_total: 80,
  } }
  let complete!: (conversation: Conversation) => void
  const read = vi.spyOn(chatApi, 'getConversationWindow').mockImplementation(() => new Promise(resolve => { complete = resolve }))
  const { result } = renderHook(() => useBotActions({
    lang: 'zh', currentConversationRef: ref,
    navigation: { beginConversationCreation: vi.fn(), isConversationCreationCurrent: vi.fn(), commitCreatedConversation: vi.fn() },
    refreshSidebar: vi.fn(), refreshContextStats: vi.fn(async () => {}),
    applyConversationIfCurrent: (id, conversation) => { if (ref.current?.id !== id) return false; ref.current = conversation; return true },
    setStreamErrorForConversation: vi.fn(), setAssistantStreamStatsByMessageId: vi.fn(),
    runAfterLeavingEditors: action => action(), startAssistantChat: vi.fn(async () => {}),
  }))
  let saved!: Promise<void>
  act(() => { saved = result.current.finishEdit({ id: 'research', name: '新研究员', created_at: 1, updated_at: 1 }, false) })
  const window = { ...ref.current!, revision: 4, title: '新研究员', history_start: 20, messages: messages.slice(20),
    assistant_snapshot: { id: 'research', name: '新研究员', system_prompt: '先核验' } }
  read.mockResolvedValueOnce({ ...window, revision: 5 })
  ref.current = { ...ref.current!, revision: 5, messages: [...messages, { id: 'stream', role: 'assistant', content: '新回复', timestamp: 81 }] }
  await act(async () => { complete(window); await saved })
  expect(ref.current!.title).toBe('新研究员')
  expect(ref.current!.assistant_snapshot?.system_prompt).toBe('先核验')
  expect(ref.current!.revision).toBe(5)
  expect(ref.current!.history_start).toBe(0)
  expect(ref.current!.messages).toHaveLength(81)
  expect(ref.current!.messages[0].content).toBe('问题 0')
  expect(ref.current!.messages[80].content).toBe('新回复')

  act(() => { saved = result.current.finishEdit({ id: 'research', name: '新研究员', created_at: 1, updated_at: 1 }, false) })
  ref.current = { ...ref.current!, id: 'another', title: '另一个聊天' }
  await act(async () => { complete(window); await saved })
  expect(ref.current!.id).toBe('another')
  expect(ref.current!.title).toBe('另一个聊天')
})

it.each([false, true])('does not roll back a newer model selection when Bot refresh is stale (second read stale: %s)', async (secondReadStale) => {
  const original: Conversation = {
    id: 'conv_bot_research', revision: 5, title: '研究员', provider_id: 'old-provider', model: 'old-model',
    created_at: 1, updated_at: 1, messages: [],
  }
  const ref = { current: original }
  let complete!: (conversation: Conversation) => void
  const read = vi.spyOn(chatApi, 'getConversationWindow')
    .mockImplementationOnce(() => new Promise(resolve => { complete = resolve }))
    .mockImplementation(async () => secondReadStale ? original : ref.current)
  const { result } = renderHook(() => useBotActions({
    lang: 'zh', currentConversationRef: ref,
    navigation: { beginConversationCreation: vi.fn(), isConversationCreationCurrent: vi.fn(), commitCreatedConversation: vi.fn() },
    refreshSidebar: vi.fn(), refreshContextStats: vi.fn(async () => {}),
    applyConversationIfCurrent: (id, conversation) => { if (ref.current.id !== id) return false; ref.current = conversation; return true },
    setStreamErrorForConversation: vi.fn(), setAssistantStreamStatsByMessageId: vi.fn(),
    runAfterLeavingEditors: action => action(), startAssistantChat: vi.fn(async () => {}),
  }))
  let saved!: Promise<void>
  act(() => { saved = result.current.finishEdit({ id: 'research', name: '研究员', created_at: 1, updated_at: 1 }, false) })
  ref.current = { ...original, revision: 6, provider_id: 'new-provider', model: 'new-model' }
  await act(async () => { complete(original); await saved })
  expect(ref.current).toMatchObject({ revision: 6, provider_id: 'new-provider', model: 'new-model' })
  expect(read.mock.calls.length).toBeLessThanOrEqual(2)
})
