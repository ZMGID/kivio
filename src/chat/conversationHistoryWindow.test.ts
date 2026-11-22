import { expect, it } from 'vitest'
import { historyWindowStart, mergeConversationHistoryPage } from './conversationHistoryWindow'
import type { Conversation } from './types'

const message = (id: string) => ({ id, role: 'user' as const, content: id, timestamp: 1 })
const window: Conversation = {
  id: 'a', revision: 5, title: 'a', provider_id: 'p', model: 'm',
  messages: [message('new')], created_at: 1, updated_at: 1,
  history_start: 1, history_total: 2, history_source: 'current',
}

it('prepends one matching page without losing the loaded tail', () => {
  expect(mergeConversationHistoryPage(window, {
    revision: 5, history_source: 'current', start: 0, end: 1, total: 2, messages: [message('old')],
  })).toMatchObject({ history_source: 'current', history_start: 0, messages: [message('old'), message('new')] })
})

it('rejects pages from edited, truncated or unrelated windows', () => {
  const page = { revision: 5, start: 0, end: 1, total: 2, messages: [message('old')] }
  expect(mergeConversationHistoryPage(window, { ...page, history_source: 'restored' })).toBeNull()
  expect(mergeConversationHistoryPage(window, page)?.history_source).toBeUndefined()
  expect(mergeConversationHistoryPage(window, { ...page, revision: 6 })).toBeNull()
  expect(mergeConversationHistoryPage(window, { ...page, total: 1 })).toBeNull()
  expect(mergeConversationHistoryPage(window, { ...page, end: 2 })).toBeNull()
})

it('retains and deduplicates referenced artifacts while prepending a matching page', () => {
  const first = { id: 'art_first', name: 'first.png' }
  const second = { id: 'art_second', name: 'second.png' }
  const result = mergeConversationHistoryPage({ ...window, history_artifacts: [first] }, {
    revision: 5, start: 0, end: 1, total: 2, messages: [message('old')],
    history_artifacts: [first, second],
  })
  expect(result?.history_artifacts).toEqual([first, second])
})

it('keeps the user question and sibling models together when a byte cap splits their answers', () => {
  const messages = [message('first'), message('question'), ...['a', 'b', 'c'].map(id => ({
    ...message(id), role: 'assistant' as const, group_id: 'compare', content: 'x'.repeat(400_000),
  }))]
  expect(historyWindowStart(messages, messages.length)).toBe(1)
})

it('opens the most recent ten turns and loads twenty earlier turns per page', () => {
  const messages = Array.from({ length: 50 }, (_, i) => [
    message('u' + i), { ...message('a' + i), role: 'assistant' as const },
  ]).flat()
  const start = historyWindowStart(messages, messages.length)
  expect(messages[start].id).toBe('u40')
  expect(messages[historyWindowStart(messages, start, 20)].id).toBe('u20')
})

it('limits heavy history windows by payload size while always making progress', () => {
  const messages = Array.from({ length: 60 }, (_, index) => ({
    ...message(String(index)), content: 'x'.repeat(180_000),
  }))
  expect(historyWindowStart(messages, 60)).toBe(55)
  expect(historyWindowStart([{ ...message('huge'), content: 'x'.repeat(1_500_000) }], 1)).toBe(0)
})
