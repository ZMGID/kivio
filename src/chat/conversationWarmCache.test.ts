import { afterEach, expect, it, vi } from 'vitest'
import { createConversationWarmCache } from './conversationWarmCache'
import type { ChatHistoryCursor } from '../generated/chatProtocol'
import type { Conversation } from './types'

const windowFor = (id: string): Conversation => ({
  id, revision: 1, title: id, provider_id: '', model: '', history_start: 0, history_source: 'first',
  messages: [], created_at: 1, updated_at: 1,
})

const replayCurrent = async (_id: string, after?: ChatHistoryCursor) => ({
  conversationRevision: after?.revision ?? null, historyReplay: after ? { after, through: after, patches: [] } : undefined,
})

afterEach(() => vi.useRealTimers())

it('evicts older windows above four entries and expires the remainder after five idle minutes', async () => {
  vi.useFakeTimers()
  const cache = createConversationWarmCache(async id => windowFor(id))
  const ids = Array.from({ length: 40 }, (_, i) => String(i))
  for (const id of ids) cache.remember(windowFor(id))
  vi.advanceTimersByTime(60_000)
  const resume = vi.fn(replayCurrent)
  const display = vi.fn()
  const results = await Promise.all(ids.map(id => cache.get(id, resume, display)))
  expect(results.flatMap(window => window ? [window.id] : [])).toEqual(['36', '37', '38', '39'])
  vi.advanceTimersByTime(240_000)
  expect(await cache.get('39', resume, display)).toBeNull()
  expect(display).toHaveBeenCalledTimes(4)
  expect(resume).toHaveBeenCalledTimes(4)
  expect(vi.getTimerCount()).toBe(0)
})

it('shows a completed send immediately and retains only its bounded window after projection', async () => {
  const full = { ...windowFor('a'), history_start: undefined, revision: 2 }
  const bounded = { ...windowFor('a'), history_start: 55, revision: 2 }
  let supply!: (window: Conversation) => void
  const read = vi.fn(() => new Promise<Conversation>(resolve => { supply = resolve }))
  const cache = createConversationWarmCache(read)
  cache.remember(full, 'anchored-message')
  const display = vi.fn()
  const restoring = cache.get('a', replayCurrent, display)
  expect(display).toHaveBeenCalledWith(full)
  expect(read).toHaveBeenCalledWith('a', 'anchored-message')
  supply(bounded)
  expect(await restoring).toBe(bounded)
  cache.clear()
})

it.each([
  { limit: 'entry count', length: 0, kept: 'E' },
  { limit: 'byte budget', length: 7 * 1024 * 1024, kept: 'B' },
])('protects the selected window when parking another reaches the $limit', async ({ length, kept }) => {
  const cache = createConversationWarmCache(async id => windowFor(id))
  const window = (id: string): Conversation => ({ ...windowFor(id), messages: [
    { id: 'reply', role: 'assistant', content: 'x'.repeat(length), timestamp: 1 },
  ] })
  cache.remember(window('A'))
  for (const id of ['B', 'C', 'D']) cache.remember(windowFor(id))
  const outgoing = window('E')
  const restored = await cache.get('A', replayCurrent, () => cache.remember(outgoing))
  expect(restored?.id).toBe('A')
  const parked = await cache.get('E', replayCurrent, () => {})
  const oldestIdle = await cache.get('B', replayCurrent, () => {})
  expect([parked?.id, oldestIdle?.id].filter(Boolean)).toEqual([kept])
  cache.clear()
})

it('projects an oversized full history and restores only the bounded window', async () => {
  const bounded = windowFor('large')
  const full: Conversation = { ...bounded, history_start: undefined, messages: [
    { id: 'old', role: 'assistant', content: 'x'.repeat(13 * 1024 * 1024), timestamp: 1 },
  ] }
  let supply!: (window: Conversation) => void
  const read = vi.fn(() => new Promise<Conversation>(resolve => { supply = resolve }))
  const cache = createConversationWarmCache(read)
  cache.remember(full, 'reading-position')
  const display = vi.fn()
  const restoring = cache.get(full.id, replayCurrent, display)
  expect(read).toHaveBeenCalledWith(full.id, 'reading-position')
  expect(display).not.toHaveBeenCalled()
  supply(bounded)
  expect(await restoring).toBe(bounded)
  expect(display.mock.calls).toEqual([[bounded]])
  expect(await cache.get(full.id, replayCurrent, display)).toBe(bounded)
  cache.clear()
})

it('refetches when a run settles during replay even with an unchanged durable revision', async () => {
  const cache = createConversationWarmCache(async id => windowFor(id))
  let running = true
  cache.remember(windowFor('run'), undefined, () => running)
  expect(await cache.get('run', async (id, after) => { running = false; return replayCurrent(id, after) }, () => {})).toBeNull()
  cache.clear()
})

it('replays contiguous metadata updates on the retained rows and reloads a broken chain', async () => {
  const read = vi.fn(async id => windowFor(id))
  const cache = createConversationWarmCache(read)
  const window = windowFor('a')
  cache.remember(window)
  const after = { revision: 1, source: 'first' }
  const through = { revision: 3, source: 'third' }
  const todo = { updatedAt: 20, items: [] }
  const patches = [
    { revision: 2, updatedAt: 10, events: [{ type: 'title_updated' as const, title: 'New title' }] },
    { revision: 3, updatedAt: 20, events: [{ type: 'todo_updated' as const, todoState: todo }] },
  ]
  const updated = await cache.get('a', async () => ({ conversationRevision: 3, historyReplay: { after, through, patches } }), () => {})
  expect(updated).toMatchObject({ revision: 3, title: 'New title', updated_at: 20, agent_todo_state: todo, history_source: 'third' })
  expect(updated?.messages).toBe(window.messages)
  expect(read).not.toHaveBeenCalled()
  const gap = await cache.get('a', async () => ({ conversationRevision: 3, historyReplay: { after, through, patches: patches.slice(1) } }), () => {})
  expect(gap).toBeNull()
  cache.clear()
})

it('bounds the combined payload including nested tool arguments and rejects a single oversized entry', async () => {
  const cache = createConversationWarmCache(async id => windowFor(id))
  const remember = (id: string, length: number) => cache.remember({ ...windowFor(id), messages: [{
    id: 'answer', role: 'assistant', content: '', timestamp: 1,
    tool_calls: [{ id: 'tool', name: 'read', arguments: { payload: 'x'.repeat(length) } }],
  }] })
  const display = vi.fn()
  remember('first', 7 * 1024 * 1024)
  remember('second', 7 * 1024 * 1024)
  expect((await cache.get('first', replayCurrent, display))?.id).toBeUndefined()
  expect((await cache.get('second', replayCurrent, display))?.id).toBe('second')
  remember('oversized', 13 * 1024 * 1024)
  expect((await cache.get('oversized', replayCurrent, display))?.id).toBeUndefined()
  cache.clear()
})
