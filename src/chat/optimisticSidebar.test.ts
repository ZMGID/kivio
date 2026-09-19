import { describe, expect, it } from 'vitest'
import type { Conversation, ConversationListItem } from './types'
import {
  conversationLastMessageContent,
  optimisticConversationListItem,
  pruneSettledOptimisticItems,
  settleOptimisticConversationListItems,
} from './optimisticSidebar'

const conversation = (overrides: Partial<Conversation> = {}): Conversation => ({
  id: 'c1', revision: 1, title: '新对话', provider_id: 'p', model: 'm',
  messages: [], created_at: 1, updated_at: 1,
  ...overrides,
} as Conversation)

describe('optimisticConversationListItem', () => {
  it('derives a title from the first prompt when the stored title is a placeholder', () => {
    const item = optimisticConversationListItem(conversation(), '  帮我  看看这个   文件 ')
    expect(item.id).toBe('c1')
    expect(item.title).not.toBe('新对话')
    expect(item.preview).toBe('帮我 看看这个 文件')
    expect(item.message_count).toBe(1)
  })

  it('keeps a real title and truncates long previews', () => {
    const item = optimisticConversationListItem(conversation({ title: '模型标题' }), 'x'.repeat(150))
    expect(item.title).toBe('模型标题')
    expect(item.preview).toBe(`${'x'.repeat(100)}...`)
  })

  it('mirrors project / set / assistant identity in both field spellings', () => {
    const item = optimisticConversationListItem(conversation({
      projectId: 'proj', set_id: 'set', assistant_snapshot: { id: 'a', name: '专家' },
    } as Partial<Conversation>), 'hi')
    expect(item.project_id).toBe('proj')
    expect(item.projectId).toBe('proj')
    expect(item.set_id).toBe('set')
    expect(item.setId).toBe('set')
    expect(item.assistant_name).toBe('专家')
    expect(item.assistantName).toBe('专家')
  })
})

describe('conversationLastMessageContent', () => {
  it('skips non-chat roles and trims', () => {
    const conv = conversation({
      messages: [
        { id: '1', role: 'user', content: 'q' },
        { id: '2', role: 'assistant', content: '  answer  ' },
        { id: '3', role: 'system', content: 'ignored' },
      ],
    } as Partial<Conversation>)
    expect(conversationLastMessageContent(conv)).toBe('answer')
    expect(conversationLastMessageContent(conversation())).toBe('')
  })
})

describe('settleOptimisticConversationListItems', () => {
  const items: ConversationListItem[] = [
    { id: 'c1', title: '草稿标题' } as ConversationListItem,
    { id: 'other', title: 'other' } as ConversationListItem,
  ]

  it('replaces the entry in place with the persisted conversation', () => {
    const kept = conversation({
      title: '模型标题',
      messages: [{ id: 'u', role: 'user', content: '问题', attachments: [{ name: 'a.pdf' }] }],
    } as Partial<Conversation>)
    const next = settleOptimisticConversationListItems(items, 'c1', kept)
    expect(next.map((item) => item.id)).toEqual(['c1', 'other'])
    expect(next[0].title).toBe('模型标题')
    expect(next[0].preview).toBe('问题')
    expect(next[1]).toBe(items[1])
  })

  it('removes the entry when the send failed outright', () => {
    expect(settleOptimisticConversationListItems(items, 'c1', null).map((item) => item.id)).toEqual(['other'])
  })
})

describe('pruneSettledOptimisticItems', () => {
  const items: ConversationListItem[] = [
    { id: 'running' } as ConversationListItem,
    { id: 'done' } as ConversationListItem,
  ]

  it('keeps only still-generating conversations', () => {
    expect(pruneSettledOptimisticItems(items, new Set(['running'])).map((item) => item.id)).toEqual(['running'])
  })

  it('returns the same array reference when nothing is pruned (no spurious re-render)', () => {
    expect(pruneSettledOptimisticItems(items, new Set(['running', 'done']))).toBe(items)
  })
})
