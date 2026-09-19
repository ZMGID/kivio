import { act, renderHook } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { chatApi } from '../api'
import { getCoarse, setCoarse } from '../streamingStore'
import type { ChatAssistant, Conversation } from '../types'
import { useAssistantActions } from './useAssistantActions'

vi.mock('../api', () => ({
  chatApi: {
    createConversation: vi.fn(),
    createBuilderConversation: vi.fn(),
    updateConversation: vi.fn(),
  },
}))

const conversation = (overrides: Partial<Conversation> = {}): Conversation => ({
  id: 'c1', revision: 1, title: 't', provider_id: 'p', model: 'm', messages: [], created_at: 1, updated_at: 1,
  ...overrides,
} as Conversation)

const assistant = (overrides: Partial<ChatAssistant> = {}): ChatAssistant => ({
  id: 'a1',
  name: 'Expert',
  ...overrides,
} as ChatAssistant)

function setup(current: Conversation | null = conversation()) {
  const currentConversationRef = { current }
  const navigation = {
    beginConversationCreation: vi.fn(() => ({ token: 1 }) as never),
    isConversationCreationCurrent: vi.fn(() => true),
    commitCreatedConversation: vi.fn(() => true),
  }
  const applyConversationIfCurrent = vi.fn(() => true)
  const setStreamErrorForConversation = vi.fn()
  const setAssistantStreamStatsByMessageId = vi.fn()
  const refreshSidebar = vi.fn()
  const refreshContextStats = vi.fn(async () => {})
  const rendered = renderHook(() => useAssistantActions({
    currentConversationRef,
    navigation,
    identity: {
      activeProviderId: 'p-active',
      activeModel: 'm-active',
      projectId: 'proj',
      projectName: 'Proj',
      setId: 'set-1',
    },
    refreshSidebar,
    refreshContextStats,
    applyConversationIfCurrent,
    setStreamErrorForConversation,
    setAssistantStreamStatsByMessageId,
  }))
  return {
    ...rendered, currentConversationRef, navigation, applyConversationIfCurrent,
    setStreamErrorForConversation, refreshSidebar, refreshContextStats,
  }
}

beforeEach(() => {
  vi.clearAllMocks()
  vi.spyOn(console, 'error').mockImplementation(() => {})
  setCoarse({ streamError: '' })
})

describe('useAssistantActions', () => {
  it('selects an assistant on the current conversation and refreshes context', async () => {
    const updated = conversation({ revision: 2 })
    vi.mocked(chatApi.updateConversation).mockResolvedValue(updated)
    const { result, applyConversationIfCurrent, refreshContextStats } = setup()
    await act(async () => { await result.current.selectAssistant(assistant()) })
    expect(chatApi.updateConversation).toHaveBeenCalledWith('c1', { assistantId: 'a1' })
    expect(applyConversationIfCurrent).toHaveBeenCalledWith('c1', updated)
    expect(refreshContextStats).toHaveBeenCalledWith('c1')
  })

  it('clears the assistant with an empty id and does not refresh context', async () => {
    vi.mocked(chatApi.updateConversation).mockResolvedValue(conversation())
    const { result, refreshContextStats } = setup()
    await act(async () => { await result.current.selectAssistant(null) })
    expect(chatApi.updateConversation).toHaveBeenCalledWith('c1', { assistantId: '' })
    expect(refreshContextStats).not.toHaveBeenCalled()
  })

  it('opens a new conversation from the welcome page, preferring the assistant model', async () => {
    const created = conversation({ id: 'new' })
    vi.mocked(chatApi.createConversation).mockResolvedValue(created)
    const { result, navigation, refreshSidebar } = setup(null)
    await act(async () => {
      await result.current.selectAssistant(assistant({ provider_id: 'p-asst', model: 'm-asst' }))
    })
    expect(chatApi.createConversation).toHaveBeenCalledWith(
      'p-asst', 'm-asst', 'Proj', 'proj', 'a1', 'set-1',
    )
    expect(refreshSidebar).toHaveBeenCalled()
    expect(navigation.commitCreatedConversation).toHaveBeenCalledWith({ token: 1 }, created)
    expect(getCoarse().streamError).toBe('')
  })

  it('falls back to the active model when the assistant does not specify one', async () => {
    vi.mocked(chatApi.createConversation).mockResolvedValue(conversation({ id: 'new' }))
    const { result } = setup(null)
    await act(async () => { await result.current.startAssistantChat(assistant()) })
    expect(chatApi.createConversation).toHaveBeenCalledWith(
      'p-active', 'm-active', 'Proj', 'proj', 'a1', 'set-1',
    )
  })

  it('reports create failures only while the creation lease is current', async () => {
    vi.mocked(chatApi.createConversation).mockRejectedValue(new Error('nope'))
    const { result, navigation } = setup(null)
    await act(async () => { await result.current.startAssistantChat(assistant()) })
    expect(getCoarse().streamError).toBe('nope')

    setCoarse({ streamError: '' })
    navigation.isConversationCreationCurrent.mockReturnValue(false)
    await act(async () => { await result.current.startAssistantChat(assistant()) })
    expect(getCoarse().streamError).toBe('')
  })

  it('starts a builder conversation on the active identity', async () => {
    const created = conversation({ id: 'builder' })
    vi.mocked(chatApi.createBuilderConversation).mockResolvedValue(created)
    const { result, navigation } = setup()
    await act(async () => { await result.current.startBuilderChat() })
    expect(chatApi.createBuilderConversation).toHaveBeenCalledWith('p-active', 'm-active', 'proj')
    expect(navigation.commitCreatedConversation).toHaveBeenCalledWith({ token: 1 }, created)
  })
})
