import { act, renderHook } from '@testing-library/react'
import { useState } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { api } from '../../api/tauri'
import { chatApi } from '../api'
import type { Conversation, ConversationContextState } from '../types'
import { useConversationContext } from './useConversationContext'

vi.mock('../../api/tauri', () => ({
  api: {
    onChatContext: vi.fn(),
    onChatCompaction: vi.fn(),
  },
}))
vi.mock('../api', () => ({
  chatApi: {
    getContextStats: vi.fn(),
    compressContext: vi.fn(),
    clearContext: vi.fn(),
  },
}))

const mockStats = vi.mocked(chatApi.getContextStats)
const mockCompress = vi.mocked(chatApi.compressContext)
const mockClear = vi.mocked(chatApi.clearContext)
const mockOnContext = vi.mocked(api.onChatContext)
const mockOnCompaction = vi.mocked(api.onChatCompaction)

type ContextListener = Parameters<typeof api.onChatContext>[0]
type CompactionListener = Parameters<typeof api.onChatCompaction>[0]
let contextListener: ContextListener | null
let compactionListener: CompactionListener | null

const state = (overrides: Partial<ConversationContextState> = {}): ConversationContextState => ({
  estimated_input_tokens: 100,
  context_window_tokens: 1000,
  ...overrides,
})

const conversation = (id = 'c1'): Conversation => ({
  id, revision: 1, title: 't', provider_id: 'p', model: 'm', messages: [], created_at: 1, updated_at: 1,
} as Conversation)

function setup(initialConversation: Conversation | null = conversation()) {
  const currentConversationIdRef = { current: initialConversation?.id ?? null }
  const refreshSidebar = vi.fn()
  const rendered = renderHook(() => {
    const [current, setCurrent] = useState<Conversation | null>(initialConversation)
    const ctx = useConversationContext({
      currentConversation: current,
      currentConversationIdRef,
      setCurrentConversation: setCurrent,
      refreshSidebar,
    })
    return { ctx, current, setCurrent }
  })
  return { ...rendered, currentConversationIdRef, refreshSidebar }
}

beforeEach(() => {
  vi.useRealTimers()
  contextListener = null
  compactionListener = null
  mockStats.mockReset()
  mockCompress.mockReset()
  mockClear.mockReset()
  mockOnContext.mockReset()
  mockOnContext.mockImplementation(async (listener) => {
    contextListener = listener
    return () => { contextListener = null }
  })
  mockOnCompaction.mockReset()
  mockOnCompaction.mockImplementation(async (listener) => {
    compactionListener = listener
    return () => { compactionListener = null }
  })
})

describe('useConversationContext: stats', () => {
  it('reads context from the accepted conversation when newer and older snapshots arrive in one batch', () => {
    const { result } = setup()
    const newer = { ...conversation(), revision: 3, context_state: state({ estimated_input_tokens: 300 }) }
    const older = { ...conversation(), revision: 2, context_state: state({ estimated_input_tokens: 200 }) }
    act(() => {
      for (const snapshot of [newer, older]) {
        result.current.setCurrent((previous) => previous?.id === snapshot.id && previous.revision > snapshot.revision
          ? previous : snapshot)
      }
    })
    expect(result.current.current?.revision).toBe(3)
    expect(result.current.ctx.contextState).toBe(result.current.current?.context_state)
    expect(result.current.ctx.contextState?.estimated_input_tokens).toBe(300)
  })

  it('replaces context on navigation and clears it when the conversation is removed', () => {
    const { result } = setup({ ...conversation(), context_state: state() })
    expect(result.current.ctx.contextState?.estimated_input_tokens).toBe(100)
    act(() => { result.current.setCurrent({ ...conversation('c2'), contextState: state({ estimated_input_tokens: 20 }) }) })
    expect(result.current.ctx.contextState?.estimated_input_tokens).toBe(20)
    act(() => { result.current.setCurrent(null) })
    expect(result.current.ctx.contextState).toBeNull()
  })

  it('loads stats for the current conversation and mirrors them into the conversation object', async () => {
    mockStats.mockResolvedValue({ contextState: state({ estimated_input_tokens: 42 }), conversation: conversation() })
    const { result } = setup()
    await act(async () => { await result.current.ctx.refreshContextStats('c1') })
    expect(result.current.ctx.contextState?.estimated_input_tokens).toBe(42)
    expect(result.current.ctx.contextLoading).toBe(false)
    expect(result.current.current?.context_state?.estimated_input_tokens).toBe(42)
    expect(result.current.current?.contextState?.estimated_input_tokens).toBe(42)
  })

  it('drops late results after the user switched conversations', async () => {
    let resolveStats: (value: { contextState: ConversationContextState; conversation: Conversation }) => void = () => {}
    mockStats.mockImplementation(() => new Promise((resolve) => { resolveStats = resolve }))
    const { result, currentConversationIdRef } = setup()
    let pending: Promise<void> = Promise.resolve()
    act(() => { pending = result.current.ctx.refreshContextStats('c1') })
    expect(result.current.ctx.contextLoading).toBe(true)
    currentConversationIdRef.current = 'c2'
    await act(async () => {
      resolveStats({ contextState: state({ estimated_input_tokens: 999 }), conversation: conversation() })
      await pending
    })
    expect(result.current.ctx.contextState).toBeNull()
    // loading is owned by the conversation that started it; the new one resets it itself.
    expect(result.current.ctx.contextLoading).toBe(true)
  })

  it('surfaces the error only for the conversation that failed and clears state without an id', async () => {
    mockStats.mockRejectedValue(new Error('boom'))
    const { result, currentConversationIdRef } = setup()
    await act(async () => { await result.current.ctx.refreshContextStats('c1') })
    expect(result.current.ctx.contextError).toBe('boom')

    currentConversationIdRef.current = null
    await act(async () => { await result.current.ctx.refreshContextStats() })
    expect(result.current.ctx.contextState).toBeNull()
    expect(result.current.ctx.contextError).toBe('')
    expect(mockStats).toHaveBeenCalledTimes(1)
  })
})

describe('useConversationContext: compaction', () => {
  it('marks the conversation compacting for the whole call, flashes the boundary and refreshes the sidebar', async () => {
    vi.useFakeTimers()
    let resolveCompress: (value: { contextState: ConversationContextState; conversation: Conversation }) => void = () => {}
    mockCompress.mockImplementation(() => new Promise((resolve) => { resolveCompress = resolve }))
    const { result, refreshSidebar } = setup()
    let pending: Promise<void> = Promise.resolve()
    act(() => { pending = result.current.ctx.compressCurrent() })
    expect(result.current.ctx.compactingConversationIds.has('c1')).toBe(true)

    await act(async () => {
      resolveCompress({
        contextState: state({ compaction_boundaries: [{ id: 'b1', created_at: 1 }] } as Partial<ConversationContextState>),
        conversation: conversation(),
      })
      await Promise.resolve()
    })
    expect(result.current.ctx.animateCompactionBoundaryId).toBe('b1')
    expect(refreshSidebar).toHaveBeenCalledTimes(1)
    // still held during the 360ms settle window
    expect(result.current.ctx.compactingConversationIds.has('c1')).toBe(true)

    await act(async () => {
      vi.advanceTimersByTime(400)
      await pending
    })
    expect(result.current.ctx.compactingConversationIds.has('c1')).toBe(false)

    act(() => { vi.advanceTimersByTime(1800) })
    expect(result.current.ctx.animateCompactionBoundaryId).toBeNull()
  })

  it('is idempotent while a compaction is already running', async () => {
    mockCompress.mockImplementation(() => new Promise(() => {}))
    const { result } = setup()
    act(() => { void result.current.ctx.compressCurrent() })
    act(() => { void result.current.ctx.compressCurrent() })
    expect(mockCompress).toHaveBeenCalledTimes(1)
  })

  it('clears the compacting flag even after switching away (no stuck spinner)', async () => {
    mockCompress.mockRejectedValue(new Error('fail'))
    const { result, currentConversationIdRef } = setup()
    let pending: Promise<void> = Promise.resolve()
    act(() => { pending = result.current.ctx.compressCurrent() })
    currentConversationIdRef.current = 'other'
    await act(async () => { await pending })
    expect(result.current.ctx.compactingConversationIds.has('c1')).toBe(false)
    expect(result.current.ctx.contextError).toBe('')
  })

  it('tracks automatic compaction of background conversations from events', async () => {
    const { result } = setup()
    await act(async () => {})
    act(() => {
      compactionListener?.({ conversationId: 'bg', trigger: 'auto', phase: 'started' } as never)
    })
    expect(result.current.ctx.compactingConversationIds.has('bg')).toBe(true)
    act(() => {
      compactionListener?.({ conversationId: 'bg', trigger: 'auto', phase: 'completed' } as never)
    })
    expect(result.current.ctx.compactingConversationIds.has('bg')).toBe(false)
    // background completion never touches the current conversation's boundaries
    expect(result.current.ctx.animateCompactionBoundaryId).toBeNull()
  })

  it('appends a completed boundary to the current conversation exactly once', async () => {
    const { result } = setup()
    await act(async () => {})
    const boundary = { id: 'b9', created_at: 1 }
    act(() => {
      compactionListener?.({ conversationId: 'c1', trigger: 'auto', phase: 'completed', boundary } as never)
      compactionListener?.({ conversationId: 'c1', trigger: 'auto', phase: 'completed', boundary } as never)
    })
    expect(result.current.current?.context_state?.compaction_boundaries).toEqual([boundary])
    expect(result.current.ctx.contextState?.compactionBoundaries).toEqual([boundary])
    expect(result.current.ctx.animateCompactionBoundaryId).toBe('b9')
  })
})

describe('useConversationContext: clear + live usage', () => {
  it('clears context, flashes the clear boundary and refreshes the sidebar', async () => {
    mockClear.mockResolvedValue({
      contextState: state({ clear_boundaries: [{ id: 'clr', created_at: 1 }] } as Partial<ConversationContextState>),
      conversation: conversation(),
    })
    const { result, refreshSidebar } = setup()
    await act(async () => { await result.current.ctx.clearCurrent() })
    expect(result.current.ctx.animateClearBoundaryId).toBe('clr')
    expect(refreshSidebar).toHaveBeenCalledTimes(1)
  })

  it('applies live token usage on top of the existing snapshot for the current conversation only', async () => {
    mockStats.mockResolvedValue({ contextState: state({ estimated_input_tokens: 100 } as Partial<ConversationContextState>), conversation: conversation() })
    const { result } = setup()
    await act(async () => { await result.current.ctx.refreshContextStats('c1') })
    act(() => {
      contextListener?.({ conversationId: 'other', live: { usedTokens: 5 } } as never)
    })
    expect(result.current.ctx.contextState?.estimated_input_tokens).toBe(100)
    act(() => {
      contextListener?.({ conversationId: 'c1', live: { usedTokens: 5 } } as never)
    })
    expect(result.current.ctx.contextState?.estimated_input_tokens).toBe(5)
    expect(result.current.current?.context_state).toEqual(result.current.ctx.contextState)
  })

  it('takes an authoritative snapshot from the event and clears the error', async () => {
    mockStats.mockRejectedValue(new Error('boom'))
    const { result } = setup()
    await act(async () => { await result.current.ctx.refreshContextStats('c1') })
    expect(result.current.ctx.contextError).toBe('boom')
    act(() => {
      contextListener?.({ conversationId: 'c1', contextState: state({ estimated_input_tokens: 7 }) } as never)
    })
    expect(result.current.ctx.contextState?.estimated_input_tokens).toBe(7)
    expect(result.current.ctx.contextError).toBe('')
  })
})
