import { describe, expect, it, vi } from 'vitest'
import { createConversationWarmCache } from './conversationWarmCache'
import type { Conversation } from './types'

const conversation = (id: string, revision = 1): Conversation => ({
  id, revision, title: id, provider_id: 'test', model: 'test',
  messages: [], created_at: 1, updated_at: 1,
})

describe('conversation warm cache', () => {
  it('evicts by aggregate bytes and keeps accounting correct after replacement and removal', () => {
    const cache = createConversationWarmCache()
    const large = (id: string) => ({ ...conversation(id), messages: [
      { id: 'm', role: 'user' as const, content: 'x'.repeat(7 * 1024 * 1024), timestamp: 1 },
    ] })
    const a = large('a')
    const b = large('b')
    cache.remember(a)
    expect(cache.stats().bytes).toBe(JSON.stringify(a).length * 2)
    cache.remember(b)
    expect(cache.stats()).toEqual({ entries: 1, bytes: JSON.stringify(b).length * 2 })
    const smaller = conversation('b', 2)
    cache.remember(smaller)
    expect(cache.stats()).toEqual({ entries: 1, bytes: JSON.stringify(smaller).length * 2 })
    cache.forget('b')
    expect(cache.stats()).toEqual({ entries: 0, bytes: 0 })
  })
  it('reuses a short-lived snapshot only when the repository revision still matches', async () => {
    let clock = 0
    const cache = createConversationWarmCache(() => clock)
    const a = conversation('a')
    cache.remember(a)
    const head = vi.fn(async () => 1)
    expect(await cache.get('a', head)).toBe(a)
    expect(head).toHaveBeenCalledWith('a')
    expect(await cache.get('a', async () => 2)).toBeNull()
    cache.remember(a)
    clock = 30_000
    expect(await cache.get('a', head)).toBeNull()
  })

  it('rejects a deleted conversation and a stale validation result', async () => {
    const cache = createConversationWarmCache()
    cache.remember(conversation('a'))
    expect(await cache.get('a', async () => null)).toBeNull()
    cache.remember(conversation('a'))
    let complete!: (value: number) => void
    const pending = cache.get('a', () => new Promise((resolve) => { complete = resolve }))
    cache.forget('a')
    complete(1)
    expect(await pending).toBeNull()
  })

  it('bounds entry count and completed snapshot bytes', () => {
    const cache = createConversationWarmCache()
    for (let index = 0; index < 5; index += 1) cache.remember(conversation(String(index)))
    expect(cache.stats().entries).toBe(4)
    expect(cache.stats().bytes).toBeLessThanOrEqual(24 * 1024 * 1024)
    const huge = conversation('huge')
    huge.messages = [{ id: 'x', role: 'user', content: 'x'.repeat(13 * 1024 * 1024), timestamp: 1 }]
    cache.remember(huge)
    expect(cache.stats().entries).toBe(4)
  })

  it('defers serialization past navigation and cancels a deleted snapshot', () => {
    vi.useFakeTimers()
    try {
      const cache = createConversationWarmCache()
      cache.rememberSoon(conversation('a'))
      expect(cache.stats().entries).toBe(0)
      cache.forget('a')
      vi.runAllTimers()
      expect(cache.stats().entries).toBe(0)
      cache.rememberSoon(conversation('a'))
      vi.runAllTimers()
      expect(cache.stats().entries).toBe(1)
    } finally {
      vi.useRealTimers()
    }
  })
})
