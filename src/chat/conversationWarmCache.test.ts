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

it('retains every recently visited window until its own five-minute idle expiry', async () => {
  vi.useFakeTimers()
  const cache = createConversationWarmCache(async id => windowFor(id))
  for (let i = 0; i < 6; i++) cache.remember(windowFor(String(i)))
  vi.advanceTimersByTime(60_000)
  const resume = vi.fn(replayCurrent)
  const display = vi.fn()
  expect((await cache.get('0', resume, display))?.id).toBe('0')
  vi.advanceTimersByTime(240_000)
  expect(await cache.get('0', resume, display)).toBeNull()
  expect(display).toHaveBeenCalledTimes(1)
  expect(resume).toHaveBeenCalledTimes(1)
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
