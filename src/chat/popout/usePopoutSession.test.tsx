import { act, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { ChatStreamPayload } from '../../api/tauri'
import { chatApi } from '../api'
import { getCoarse, getSnapshot, reset as resetStreamStore } from '../streamingStore'
import type { Conversation } from '../types'
import { usePopoutSession } from './usePopoutSession'

type Handler = (payload: unknown) => void
const { handlers, listen } = vi.hoisted(() => {
  const handlers = new Map<string, Handler>()
  const listen = (name: string) => (handler: Handler) => {
    handlers.set(name, handler)
    return Promise.resolve(() => { handlers.delete(name) })
  }
  return { handlers, listen }
})

vi.mock('../../api/tauri', () => ({
  api: {
    onChatStream: listen('stream'),
    onChatTool: listen('tool'),
    onChatSubagent: listen('subagent'),
    onChatToolConfirm: listen('toolConfirm'),
    onChatToolConfirmWithdraw: listen('toolConfirmWithdraw'),
    onChatSessionConsent: listen('sessionConsent'),
    onChatUserPrompt: listen('userPrompt'),
    onChatHook: listen('hook'),
    onChatQueuedTextsRestored: listen('queuedTexts'),
    onChatStatusNote: listen('statusNote'),
    onChatTodo: listen('todo'),
    onChatPlan: listen('plan'),
    onChatGoal: listen('goal'),
    chatConfirmToolCall: vi.fn(),
    chatRespondSessionConsent: vi.fn(),
  },
}))
vi.mock('../../api/chatProtocol', () => ({ syncChatProtocol: vi.fn().mockResolvedValue(undefined) }))
vi.mock('../../api/settingsCache', () => ({
  getSettingsCached: vi.fn().mockResolvedValue({}),
  updateSettingsCached: vi.fn(),
}))
vi.mock('../api', () => ({
  chatApi: { getConversation: vi.fn() },
  agentRuntimesEqual: () => true,
  normalizeAgentRuntime: () => ({ kind: 'builtin' }),
}))
vi.mock('./usePopoutComposer', () => ({ usePopoutComposer: () => ({}) }))

const mockGetConversation = vi.mocked(chatApi.getConversation)

const CONVERSATION_ID = 'c1'
const TWIN_ID = 'assistant-1'

const conversationWith = (messages: Conversation['messages']): Conversation => ({
  id: CONVERSATION_ID, revision: 1, title: 't', provider_id: 'p', model: 'm',
  messages, created_at: 1, updated_at: 1,
} as Conversation)

const userOnly = conversationWith([{ id: 'user-1', role: 'user', content: 'hi', timestamp: 1 }])
const withTwin = conversationWith([
  { id: 'user-1', role: 'user', content: 'hi', timestamp: 1 },
  { id: TWIN_ID, role: 'assistant', content: 'answer', timestamp: 2 },
])

const packet = (type: string, runId: string, delta?: string): ChatStreamPayload => ({
  type, conversationId: CONVERSATION_ID, runId, messageId: TWIN_ID, delta,
} as ChatStreamPayload)

const emitStream = (payload: ChatStreamPayload) => {
  handlers.get('stream')?.(payload)
}

/** Flush microtasks + zero-delay timers (our rAF stub) inside act. */
const flush = () => act(async () => { await vi.advanceTimersByTimeAsync(0) })

async function setupStreamedAnswer() {
  mockGetConversation.mockResolvedValueOnce(userOnly)
  const rendered = renderHook(() => usePopoutSession(CONVERSATION_ID, 'zh'))
  await flush()
  expect(rendered.result.current.conversation?.messages).toHaveLength(1)
  await act(async () => {
    emitStream(packet('run_started', 'run-1'))
    emitStream(packet('text_delta', 'run-1', 'answer'))
  })
  await flush()
  expect(getSnapshot().content).toBe('answer')
  expect(getCoarse()).toMatchObject({ streaming: true, streamFrozen: false })
  return rendered
}

describe('usePopoutSession run settle', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) =>
      setTimeout(() => callback(performance.now()), 0) as unknown as number)
    vi.stubGlobal('cancelAnimationFrame', (handle: number) => clearTimeout(handle))
    handlers.clear()
    mockGetConversation.mockReset()
    resetStreamStore()
  })

  afterEach(() => {
    resetStreamStore()
    vi.unstubAllGlobals()
    vi.useRealTimers()
  })

  it('keeps the live answer frozen through reload and clears it only once the twin is committed', async () => {
    const rendered = await setupStreamedAnswer()

    let resolveReload: (conversation: Conversation) => void = () => {}
    mockGetConversation.mockImplementationOnce(() => new Promise((resolve) => { resolveReload = resolve }))
    await act(async () => { emitStream(packet('run_completed', 'run-1')) })
    await flush()

    // Terminal frame: the live row must stay mounted (frozen), not unmount for the reload window.
    expect(getSnapshot().content).toBe('answer')
    expect(getCoarse()).toMatchObject({ streaming: false, streamFrozen: true })
    expect(mockGetConversation).toHaveBeenCalledTimes(2)

    await act(async () => { resolveReload(withTwin) })
    await flush()

    // React committed the twin → the frozen preview is released in the same pass.
    expect(rendered.result.current.messageListProps.messages.map((m) => m.id)).toContain(TWIN_ID)
    expect(getSnapshot().content).toBe('')
    expect(getCoarse()).toMatchObject({ streaming: false, streamFrozen: false })
  })

  it('does not clear the preview when the reloaded conversation lacks the twin; falls back after a bound', async () => {
    await setupStreamedAnswer()

    // Persistence lagging: reload returns a list without the answer.
    mockGetConversation.mockResolvedValueOnce(userOnly)
    await act(async () => { emitStream(packet('run_completed', 'run-1')) })
    await flush()

    expect(getSnapshot().content).toBe('answer')
    expect(getCoarse()).toMatchObject({ streaming: false, streamFrozen: true })

    await act(async () => { await vi.advanceTimersByTimeAsync(1_400) })
    expect(getSnapshot().content).toBe('answer')

    await act(async () => { await vi.advanceTimersByTimeAsync(200) })
    expect(getSnapshot().content).toBe('')
    expect(getCoarse().streamFrozen).toBe(false)
  })

  it('a new run supersedes a pending twin without its fallback timer resetting the new live answer', async () => {
    await setupStreamedAnswer()

    mockGetConversation.mockResolvedValueOnce(userOnly)
    await act(async () => { emitStream(packet('run_completed', 'run-1')) })
    await flush()
    expect(getCoarse().streamFrozen).toBe(true)

    await act(async () => {
      emitStream({ ...packet('run_started', 'run-2'), messageId: 'assistant-2' } as ChatStreamPayload)
      emitStream({ ...packet('text_delta', 'run-2', 'second'), messageId: 'assistant-2' } as ChatStreamPayload)
    })
    await flush()
    expect(getSnapshot().content).toBe('second')
    expect(getCoarse()).toMatchObject({ streaming: true, streamFrozen: false })

    // The first run's 1.5s fallback must be dead by now.
    await act(async () => { await vi.advanceTimersByTimeAsync(2_000) })
    expect(getSnapshot().content).toBe('second')
    expect(getCoarse().streaming).toBe(true)
  })
})
