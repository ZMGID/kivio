import { act, renderHook } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { api } from '../../api/tauri'
import { chatApi } from '../api'
import { insertTextIntoComposer } from '../composerInsert'
import { getCoarse, setCoarse } from '../streamingStore'
import type { Conversation } from '../types'
import { noteTitleFromContent, useMessageActions } from './useMessageActions'

vi.mock('../../api/tauri', () => ({ api: { notesCreate: vi.fn() } }))
vi.mock('../api', () => ({
  chatApi: {
    updateMessage: vi.fn(),
    deleteMessage: vi.fn(),
    rewindToMessage: vi.fn(),
    forkConversation: vi.fn(),
    setGroupSelection: vi.fn(),
  },
}))
vi.mock('../composerInsert', () => ({ insertTextIntoComposer: vi.fn() }))

const conversation = (overrides: Partial<Conversation> = {}): Conversation => ({
  id: 'c1', revision: 1, title: 't', provider_id: 'p', model: 'm', created_at: 1, updated_at: 1,
  messages: [
    { id: 'm1', role: 'user', content: 'hello', created_at: 1 },
    { id: 'm2', role: 'assistant', content: '## Answer\n\nbody', created_at: 2 },
    { id: 'm3', role: 'assistant', content: '   ', created_at: 3 },
  ],
  ...overrides,
} as Conversation)

function setup(current: Conversation | null = conversation()) {
  const currentConversationRef = { current }
  const navigation = {
    beginConversationCreation: vi.fn(() => ({ token: 1 }) as never),
    isConversationCreationCurrent: vi.fn(() => true),
    commitCreatedConversation: vi.fn(() => true),
  }
  const applyConversationIfCurrent = vi.fn((expectedId: string) => currentConversationRef.current?.id === expectedId)
  const applyConversationMeta = vi.fn()
  const setStreamErrorForConversation = vi.fn()
  const setAssistantStreamStatsByMessageId = vi.fn()
  const refreshSidebar = vi.fn()
  const refreshContextStats = vi.fn(async () => {})
  const rendered = renderHook(() => useMessageActions({
    currentConversationRef,
    navigation,
    applyConversationIfCurrent,
    applyConversationMeta,
    setStreamErrorForConversation,
    setAssistantStreamStatsByMessageId,
    refreshSidebar,
    refreshContextStats,
  }))
  return {
    ...rendered, currentConversationRef, navigation, applyConversationIfCurrent, applyConversationMeta,
    setStreamErrorForConversation, setAssistantStreamStatsByMessageId, refreshSidebar, refreshContextStats,
  }
}

beforeEach(() => {
  vi.clearAllMocks()
  vi.spyOn(console, 'error').mockImplementation(() => {})
  vi.spyOn(window, 'confirm').mockReturnValue(true)
  setCoarse({ streamError: '' })
})

describe('noteTitleFromContent', () => {
  it('strips heading markers and emphasis, capping at 40 chars', () => {
    expect(noteTitleFromContent('## **Bold** _title_\nrest')).toBe('Bold title')
    expect(noteTitleFromContent('\n\n  x'.padEnd(60, 'y'))).toHaveLength(40)
  })
  it('falls back when nothing survives', () => {
    expect(noteTitleFromContent('')).toBe('对话笔记')
    expect(noteTitleFromContent('###', 'fb')).toBe('fb')
  })
})

describe('useMessageActions', () => {
  it('edits a message and refreshes the sidebar', async () => {
    const updated = conversation({ revision: 2 })
    vi.mocked(chatApi.updateMessage).mockResolvedValue(updated)
    const { result, applyConversationIfCurrent, refreshSidebar } = setup()
    await act(async () => { await result.current.updateMessage('m1', 'edited') })
    expect(chatApi.updateMessage).toHaveBeenCalledWith('c1', 'm1', 'edited')
    expect(applyConversationIfCurrent).toHaveBeenCalledWith('c1', updated)
    expect(refreshSidebar).toHaveBeenCalled()
  })

  it('does nothing when the user cancels the delete confirm', async () => {
    vi.mocked(window.confirm).mockReturnValue(false)
    const { result } = setup()
    await act(async () => { await result.current.deleteMessage('m2') })
    expect(chatApi.deleteMessage).not.toHaveBeenCalled()
  })

  it('drops the stream stats of a deleted message only if the conversation is still current', async () => {
    vi.mocked(chatApi.deleteMessage).mockResolvedValue(conversation())
    const { result, setAssistantStreamStatsByMessageId } = setup()
    await act(async () => { await result.current.deleteMessage('m2') })
    expect(setAssistantStreamStatsByMessageId).toHaveBeenCalledTimes(1)
    const updater = setAssistantStreamStatsByMessageId.mock.calls[0][0] as (prev: Record<string, unknown>) => Record<string, unknown>
    expect(updater({ m1: 1, m2: 2 })).toEqual({ m1: 1 })

    const other = setup()
    other.currentConversationRef.current = conversation({ id: 'c2' })
    vi.mocked(chatApi.deleteMessage).mockResolvedValue(conversation({ id: 'c2' }))
    other.applyConversationIfCurrent.mockReturnValue(false)
    await act(async () => { await other.result.current.deleteMessage('m2') })
    expect(other.setAssistantStreamStatsByMessageId).not.toHaveBeenCalled()
    expect(other.refreshSidebar).toHaveBeenCalled()
  })

  it('rewinds: puts the text back in the composer, clears the error and recomputes context', async () => {
    vi.mocked(chatApi.rewindToMessage).mockResolvedValue({ conversation: conversation({ revision: 3 }), content: 'hello' })
    setCoarse({ streamError: 'old' })
    const { result, setAssistantStreamStatsByMessageId, refreshContextStats } = setup()
    await act(async () => { await result.current.rewindToMessage('m1') })
    expect(insertTextIntoComposer).toHaveBeenCalledWith('hello')
    expect(setAssistantStreamStatsByMessageId).toHaveBeenCalledWith({})
    expect(getCoarse().streamError).toBe('')
    expect(refreshContextStats).toHaveBeenCalledWith('c1')
  })

  it('reports rewind failures against the originating conversation', async () => {
    vi.mocked(chatApi.rewindToMessage).mockRejectedValue(new Error('nope'))
    const { result, setStreamErrorForConversation } = setup()
    await act(async () => { await result.current.rewindToMessage('m1') })
    expect(setStreamErrorForConversation).toHaveBeenCalledWith('c1', 'nope')
    expect(insertTextIntoComposer).not.toHaveBeenCalled()
  })

  it('forks and commits the new conversation only while the creation lease is current', async () => {
    const forked = conversation({ id: 'fork' })
    vi.mocked(chatApi.forkConversation).mockResolvedValue(forked)
    const { result, navigation, refreshSidebar } = setup()
    await act(async () => { await result.current.forkAtMessage('m2') })
    expect(refreshSidebar).toHaveBeenCalled()
    expect(navigation.commitCreatedConversation).toHaveBeenCalledWith({ token: 1 }, forked)

    const stale = setup()
    stale.navigation.isConversationCreationCurrent.mockReturnValue(false)
    await act(async () => { await stale.result.current.forkAtMessage('m2') })
    expect(stale.navigation.commitCreatedConversation).not.toHaveBeenCalled()
  })

  it('saves a note titled from the message body and refuses blank messages', async () => {
    vi.mocked(api.notesCreate).mockResolvedValue({} as never)
    const { result } = setup()
    let ok = false
    await act(async () => { ok = await result.current.saveMessageToNote('m2') })
    expect(ok).toBe(true)
    expect(api.notesCreate).toHaveBeenCalledWith('Answer', '## Answer\n\nbody', '', 'chat')

    await act(async () => { ok = await result.current.saveMessageToNote('m3') })
    expect(ok).toBe(false)
    await act(async () => { ok = await result.current.saveMessageToNote('missing') })
    expect(ok).toBe(false)
    expect(api.notesCreate).toHaveBeenCalledTimes(1)
  })

  it('surfaces note failures on the current view only', async () => {
    vi.mocked(api.notesCreate).mockRejectedValue(new Error('disk full'))
    const { result, setStreamErrorForConversation } = setup()
    await act(async () => { await result.current.saveMessageToNote('m1') })
    expect(getCoarse().streamError).toBe('disk full')
    expect(setStreamErrorForConversation).not.toHaveBeenCalled()
  })

  it('applies group selection as a metadata-only update', async () => {
    const updated = conversation({ revision: 4 })
    vi.mocked(chatApi.setGroupSelection).mockResolvedValue(updated)
    const { result, applyConversationMeta } = setup()
    await act(async () => { await result.current.setGroupSelection('g1', 'm2') })
    expect(chatApi.setGroupSelection).toHaveBeenCalledWith('c1', 'g1', 'm2')
    expect(applyConversationMeta).toHaveBeenCalledWith(updated)
  })

  it('is a no-op without a conversation', async () => {
    const { result } = setup(null)
    await act(async () => {
      await result.current.updateMessage('m1', 'x')
      await result.current.deleteMessage('m1')
      await result.current.rewindToMessage('m1')
      await result.current.forkAtMessage('m1')
      await result.current.setGroupSelection('g', 'm1')
      expect(await result.current.saveMessageToNote('m1')).toBe(false)
    })
    expect(chatApi.updateMessage).not.toHaveBeenCalled()
    expect(chatApi.deleteMessage).not.toHaveBeenCalled()
    expect(chatApi.rewindToMessage).not.toHaveBeenCalled()
    expect(chatApi.forkConversation).not.toHaveBeenCalled()
    expect(chatApi.setGroupSelection).not.toHaveBeenCalled()
  })
})
