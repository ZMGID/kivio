import { expect, it } from 'vitest'
import { historyWindowStart, prependConversationHistoryPage } from './conversationHistoryWindow'
import type { Conversation } from './types'

const message = (id: string) => ({ id, role: 'user' as const, content: id, timestamp: 1 })
const window: Conversation = {
  id: 'a', revision: 5, title: 'a', provider_id: 'p', model: 'm',
  messages: [message('new')], created_at: 1, updated_at: 1,
  history_start: 1, history_total: 2,
}

it('prepends one matching page without losing the loaded tail', () => {
  expect(prependConversationHistoryPage(window, {
    revision: 5, start: 0, end: 1, total: 2, messages: [message('old')],
  })).toMatchObject({ history_start: 0, messages: [message('old'), message('new')] })
})

it('rejects pages from edited, truncated or unrelated windows', () => {
  const page = { revision: 5, start: 0, end: 1, total: 2, messages: [message('old')] }
  expect(prependConversationHistoryPage(window, { ...page, revision: 6 })).toBeNull()
  expect(prependConversationHistoryPage(window, { ...page, total: 1 })).toBeNull()
  expect(prependConversationHistoryPage(window, { ...page, end: 2 })).toBeNull()
})

it('retains and deduplicates referenced artifacts while prepending a matching page', () => {
  const first = { id: 'art_first', name: 'first.png' }
  const second = { id: 'art_second', name: 'second.png' }
  const result = prependConversationHistoryPage({ ...window, history_artifacts: [first] }, {
    revision: 5, start: 0, end: 1, total: 2, messages: [message('old')],
    history_artifacts: [first, second],
  })
  expect(result?.history_artifacts).toEqual([first, second])
})

it('starts the visible window before a split multi-answer group', () => {
  const messages = Array.from({ length: 64 }, (_, index) => ({
    ...message(String(index)),
    role: index >= 4 && index <= 6 ? 'assistant' as const : 'user' as const,
    group_id: index >= 3 && index <= 6 ? 'group' : undefined,
  }))
  expect(historyWindowStart(messages, 64)).toBe(3)
})

it('limits heavy history windows by payload size while always making progress', () => {
  const messages = Array.from({ length: 60 }, (_, index) => ({
    ...message(String(index)), content: 'x'.repeat(180_000),
  }))
  expect(historyWindowStart(messages, 60)).toBeGreaterThan(55)
  expect(historyWindowStart([{ ...message('huge'), content: 'x'.repeat(900_000) }], 1)).toBe(0)
})
