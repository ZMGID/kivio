// @vitest-environment jsdom
import { afterEach, expect, it } from 'vitest'
import { chatApi } from './api'
import { isPartialConversation, mergeConversationHistoryPage } from './conversationHistoryWindow'

afterEach(() => localStorage.removeItem('kivio-chat-dev-conversations'))

it('deletes one grouped answer and drops the selection only when it pointed at that answer', async () => {
  localStorage.setItem('kivio-chat-dev-conversations', JSON.stringify([{
    id: 'grouped', revision: 1, title: 'multi', provider_id: 'dev-provider', model: 'model-a',
    created_at: 1, updated_at: 1,
    groupSelections: { 'smoke-group': 'b', 'other-group': 'later' },
    messages: [
      { id: 'user', role: 'user', content: 'question', timestamp: 1 },
      { id: 'a', role: 'assistant', content: 'answer a', model: 'model-a', group_id: 'smoke-group', timestamp: 2 },
      { id: 'b', role: 'assistant', content: 'answer b', model: 'model-b', group_id: 'smoke-group', timestamp: 3 },
      { id: 'c', role: 'assistant', content: 'answer c', model: 'model-c', group_id: 'smoke-group', timestamp: 4 },
      { id: 'user-2', role: 'user', content: 'next', timestamp: 5 },
      { id: 'later', role: 'assistant', content: 'later answer', timestamp: 6 },
    ],
  }]))

  const deletedSelected = await chatApi.deleteMessage('grouped', 'b')
  expect(deletedSelected.messages.map((message) => message.id)).toEqual(['user', 'a', 'c', 'user-2', 'later'])
  expect(deletedSelected.group_selections).toEqual({ 'other-group': 'later' })
  expect(deletedSelected.groupSelections).toEqual({ 'other-group': 'later' })

  const deletedOther = await chatApi.deleteMessage('grouped', 'c')
  expect(deletedOther.messages.map((message) => message.id)).toEqual(['user', 'a', 'user-2', 'later'])
  expect(deletedOther.group_selections).toEqual({ 'other-group': 'later' })
  await expect(chatApi.deleteMessage('grouped', 'user')).rejects.toThrow('仅支持删除助手回复')
  expect((await chatApi.getConversation('grouped')).messages.map((message) => message.id)).toEqual([
    'user', 'a', 'user-2', 'later',
  ])
})

it('supplies only referenced artifacts from outside both the first window and older pages', async () => {
  const artifact = { id: 'art_early', name: 'early.png', mime_type: 'image/png', data_url: 'data:image/png;base64,AAAA' }
  const messages = Array.from({ length: 130 }, (_, index) => ({
    id: `m${index}`, role: 'assistant', content: `reply ${index}`, timestamp: index,
    ...(index === 0 ? { artifacts: [artifact, { ...artifact, id: 'art_unused' }] } : {}),
  }))
  messages[129].content = '![earlier](artifact:art_early)'
  messages[69].content = '![earlier again](artifact:art_early)'
  localStorage.setItem('kivio-chat-dev-conversations', JSON.stringify([{
    id: 'window-reference', revision: 1, title: 'history', provider_id: 'p', model: 'm',
    created_at: 1, updated_at: 1, messages,
  }]))
  const window = await chatApi.getConversationWindow('window-reference')
  expect(window.messages).toHaveLength(60)
  expect(window.history_artifacts).toEqual([artifact])
  const page = await chatApi.getConversationPage('window-reference', window.history_start!)
  expect(page.messages).toHaveLength(60)
  expect(page.history_artifacts).toEqual([artifact])
  expect((await chatApi.getConversation('window-reference')).messages[129].artifacts).toBeUndefined()
})

it('includes an unloaded compaction marker when its display anchor was deleted', async () => {
  localStorage.setItem('kivio-chat-dev-conversations', JSON.stringify([{
    id: 'directory-fallback', revision: 1, title: 'history', provider_id: 'p', model: 'm',
    created_at: 1, updated_at: 1,
    messages: Array.from({ length: 100 }, (_, index) => ({
      id: `m${index}`, role: 'user', content: `question ${index}`, timestamp: index,
    })),
    context_state: { compaction_boundaries: [{
      id: 'boundary', source_until_message_id: 'm0', display_after_message_id: 'deleted',
      summary_content: 'old summary', trigger: 'manual', created_at: 2,
    }] },
  }]))
  const result = await chatApi.getConversationWindow('directory-fallback')
  expect(result.messages.some((message) => message.id === 'm0')).toBe(false)
  expect(result.history_directory).toContainEqual(expect.objectContaining({
    kind: 'compaction', message_id: 'm0', message_index: 0, answer_preview: 'old summary',
  }))
})

it('restores an early row and the latest turn, then reads through the gap from either side without skipping messages', async () => {
  const id = 'anchored-window'
  const messages = Array.from({ length: 380 }, (_, i) => ({
    id: `m${i}`, role: i % 2 === 0 ? 'user' : 'assistant', content: String(i), timestamp: i,
  }))
  localStorage.setItem('kivio-chat-dev-conversations', JSON.stringify([{
    id, revision: 8, title: 'history', provider_id: 'p', model: 'm', created_at: 1, updated_at: 1, messages,
  }]))

  let loaded = await chatApi.getConversationWindow(id, 'm5')
  expect(loaded.messages.some(message => message.id === 'm5')).toBe(true)
  expect(loaded.messages.at(-1)?.id).toBe('m379')
  expect(loaded.messages.some(message => message.id === 'm180')).toBe(false)
  expect(loaded.history_start).toBe(0)
  expect(isPartialConversation(loaded)).toBe(true)
  let forward = true
  while (loaded.history_gap) {
    const gap = loaded.history_gap
    const page = await chatApi.getConversationPage(id, gap.end, forward ? gap.start : undefined)
    const merged = mergeConversationHistoryPage(loaded, page)
    if (!merged) throw new Error('Page failed to extend the reading window')
    expect(merged.messages.length).toBeGreaterThan(loaded.messages.length)
    loaded = merged
    forward = !forward
  }
  expect(isPartialConversation(loaded)).toBe(false)
  expect(loaded.messages.map(message => message.id)).toEqual(messages.map(message => message.id))
})
