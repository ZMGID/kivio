// @vitest-environment jsdom
import { afterEach, expect, it } from 'vitest'
import { chatApi } from './api'

afterEach(() => localStorage.removeItem('kivio-chat-dev-conversations'))

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
