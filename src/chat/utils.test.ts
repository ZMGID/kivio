import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  CHAT_EMPTY_GREETINGS,
  groupConversationsByTime,
  pickRandomChatEmptyGreeting,
} from './utils'
import type { ConversationListItem } from './types'

function conversation(id: string, updatedAt: number): ConversationListItem {
  return {
    id,
    title: id,
    preview: '',
    provider_id: 'openai',
    model: 'gpt-4o',
    message_count: 1,
    created_at: updatedAt,
    updated_at: updatedAt,
  }
}

describe('groupConversationsByTime', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date('2026-06-11T12:00:00Z'))
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('groups conversations into time buckets', () => {
    const now = Date.now() / 1000
    const groups = groupConversationsByTime([
      conversation('today', now - 3600),
      conversation('week', now - 86400 * 3),
      conversation('older', now - 86400 * 40),
    ])
    expect(groups.map((group) => group.title)).toEqual(['今天', '最近 7 天', '更早'])
    expect(groups[0]?.conversations.map((item) => item.id)).toEqual(['today'])
  })
})

describe('pickRandomChatEmptyGreeting', () => {
  it('returns one of the configured greetings', () => {
    vi.spyOn(Math, 'random').mockReturnValue(0)
    expect(pickRandomChatEmptyGreeting()).toBe(CHAT_EMPTY_GREETINGS[0])
    vi.restoreAllMocks()
  })
})
