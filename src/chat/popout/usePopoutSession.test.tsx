import { act, renderHook } from '@testing-library/react'
import { useLayoutEffect } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { ChatStreamPayload, ChatSubagentPayload } from '../../api/tauri'
import { chatApi } from '../api'
import { getCoarse, getSnapshot, reset as resetStreamStore, subscribeSnapshot } from '../streamingStore'
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
    onChatUserPromptResolved: listen('userPromptResolved'),
    onChatHook: listen('hook'),
    onChatQueuedTextsRestored: listen('queuedTexts'),
    onChatStatusNote: listen('statusNote'),
    onChatTodo: listen('todo'),
    onChatPlan: listen('plan'),
    onChatGoal: listen('goal'),
    onChatTitle: listen('title'),
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
  chatApi: { getConversation: vi.fn(), sendMessage: vi.fn(), cancelStream: vi.fn() },
  agentRuntimesEqual: () => true,
  normalizeAgentRuntime: () => ({ kind: 'builtin' }),
}))
vi.mock('./usePopoutComposer', () => ({
  usePopoutComposer: ({ onSend, onCancel }: { onSend: unknown; onCancel: unknown }) => ({ onSend, onCancel }),
}))

const mockGetConversation = vi.mocked(chatApi.getConversation)
const mockSendMessage = vi.mocked(chatApi.sendMessage)
const mockCancelStream = vi.mocked(chatApi.cancelStream)

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
  let committedMessages: Conversation['messages'] = []
  const rendered = renderHook(() => {
    const session = usePopoutSession(CONVERSATION_ID, 'zh')
    useLayoutEffect(() => { committedMessages = session.conversation?.messages ?? [] }, [session.conversation])
    return session
  })
  await flush()
  expect(rendered.result.current.conversation?.messages).toHaveLength(1)
  await act(async () => {
    emitStream(packet('run_started', 'run-1'))
    emitStream(packet('text_delta', 'run-1', 'answer'))
  })
  await flush()
  expect(getSnapshot().content).toBe('answer')
  expect(getCoarse()).toMatchObject({ streaming: true, streamFrozen: false })
  return { ...rendered, getCommittedMessages: () => committedMessages }
}

const subagentProgress = (overrides: Partial<ChatSubagentPayload> = {}): ChatSubagentPayload => ({
  parentConversationId: CONVERSATION_ID, parentRunId: 'run-1', parentToolCallId: 'old-parent',
  taskId: 'old-worker', name: 'Worker', depth: 1, status: 'running', preview: 'Working',
  ...overrides,
})

describe('usePopoutSession run settle', () => {
  it('withdraws only the resolved question while the run continues', async () => {
    const { result } = await setupStreamedAnswer()
    const prompt = (runId: string, toolCallId: string) => ({
      conversationId: CONVERSATION_ID, runId, messageId: TWIN_ID, toolCallId,
      id: toolCallId, name: 'ask_user', source: 'native', prompt: { questions: [] },
    })
    await act(async () => {
      handlers.get('userPrompt')?.(prompt('run-1', 'first'))
      handlers.get('userPrompt')?.(prompt('run-1', 'second'))
      handlers.get('userPromptResolved')?.(prompt('old-run', 'first'))
    })
    expect(result.current.pendingUserPrompt?.toolCallId).toBe('first')
    await act(async () => { handlers.get('userPromptResolved')?.(prompt('run-1', 'first')) })
    expect(result.current.pendingUserPrompt?.toolCallId).toBe('second')
    await act(async () => { handlers.get('userPromptResolved')?.(prompt('run-1', 'second')) })
    expect(result.current.pendingUserPrompt).toBeNull()
    expect(getCoarse().streaming).toBe(true)
  })

  beforeEach(() => {
    vi.useFakeTimers()
    vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) =>
      setTimeout(() => callback(performance.now()), 0) as unknown as number)
    vi.stubGlobal('cancelAnimationFrame', (handle: number) => clearTimeout(handle))
    handlers.clear()
    mockGetConversation.mockReset()
    mockSendMessage.mockReset()
    mockCancelStream.mockReset()
    resetStreamStore()
  })

  afterEach(() => {
    resetStreamStore()
    vi.unstubAllGlobals()
    vi.useRealTimers()
  })

  it('updates the persisted parent after its run ends and ignores other conversations', async () => {
    const rendered = await setupStreamedAnswer()
    const unrelated = { id: 'other-parent', name: 'Task', source: 'external_cli', status: 'running' as const }
    const persisted = conversationWith([
      ...userOnly.messages,
      { id: TWIN_ID, role: 'assistant', content: 'answer', timestamp: 2, tool_calls: [
        { id: 'old-parent', name: 'task', source: 'native', status: 'success' },
      ] },
      { id: 'other-message', role: 'assistant', content: '', timestamp: 3, tool_calls: [unrelated] },
    ])
    mockGetConversation.mockResolvedValueOnce(persisted)
    await act(async () => { emitStream(packet('run_completed', 'run-1')) })
    await flush()
    const before = rendered.result.current.conversation
    await act(async () => {
      handlers.get('subagent')?.(subagentProgress({ parentConversationId: 'other' }))
      handlers.get('subagent')?.(subagentProgress({ parentToolCallId: 'missing', taskId: 'unknown' }))
    })
    expect(rendered.result.current.conversation).toBe(before)

    await act(async () => { handlers.get('subagent')?.(subagentProgress()) })
    const message = rendered.result.current.conversation!.messages[1]
    expect(message.toolCalls?.[0].structuredContent).toMatchObject({
      subagentProgressByTask: { 'old-worker': { taskId: 'old-worker', status: 'running', preview: 'Working' } },
    })
    expect(message.tool_calls).toEqual(message.toolCalls)
    expect(rendered.result.current.conversation!.messages[2].tool_calls?.[0]).toEqual(unrelated)
    await act(async () => {
      handlers.get('subagent')?.(subagentProgress({ taskId: 'second-worker', preview: 'Other worker' }))
    })
    await act(async () => {
      handlers.get('subagent')?.(subagentProgress({ status: 'completed', preview: 'Done' }))
    })
    expect(rendered.result.current.conversation!.messages[1].toolCalls?.[0]).toMatchObject({
      status: 'success',
      structuredContent: { subagentProgressByTask: {
        'old-worker': { status: 'completed', preview: 'Done' },
        'second-worker': { status: 'running', preview: 'Other worker' },
      } },
    })
  })

  it('keeps live progress in the current run while applying older worker updates only to their exact parent', async () => {
    const unrelated = { id: 'unrelated', name: 'Agent', source: 'external_cli', status: 'running' as const }
    const persisted = conversationWith([
      { id: 'previous', role: 'assistant', content: '', timestamp: 1, toolCalls: [
        { id: 'old-parent', name: 'task', source: 'native', status: 'success' },
        unrelated,
      ] },
    ])
    mockGetConversation.mockResolvedValueOnce(persisted)
    const rendered = renderHook(() => usePopoutSession(CONVERSATION_ID, 'zh'))
    await flush()
    await act(async () => {
      emitStream(packet('run_started', 'run-2'))
      handlers.get('tool')?.({
        conversationId: CONVERSATION_ID, runId: 'run-2', messageId: TWIN_ID,
        toolCallId: 'live-parent', name: 'task', source: 'native', status: 'running',
      })
    })
    await flush()
    await act(async () => {
      handlers.get('subagent')?.(subagentProgress({
        parentRunId: 'run-2', parentToolCallId: 'live-parent', taskId: 'live-worker', preview: 'Live step',
      }))
    })
    await flush()
    expect(getSnapshot().toolCalls[0].structuredContent).toMatchObject({
      subagentProgressByTask: { 'live-worker': { taskId: 'live-worker', preview: 'Live step' } },
    })

    await act(async () => { handlers.get('subagent')?.(subagentProgress({ preview: 'Late step' })) })
    expect(rendered.result.current.conversation!.messages[0].toolCalls?.[0].structuredContent).toMatchObject({
      subagentProgressByTask: { 'old-worker': { taskId: 'old-worker', preview: 'Late step' } },
    })
    expect(rendered.result.current.conversation!.messages[0].toolCalls?.[1]).toEqual(unrelated)
    expect(getSnapshot().toolCalls[0].structuredContent).toMatchObject({
      subagentProgressByTask: { 'live-worker': { taskId: 'live-worker', preview: 'Live step' } },
    })
    const before = rendered.result.current.conversation
    await act(async () => {
      handlers.get('subagent')?.(subagentProgress({ parentToolCallId: '', taskId: 'unknown', preview: 'Wrong' }))
      handlers.get('subagent')?.(subagentProgress({ parentRunId: '', parentToolCallId: 'missing', taskId: 'unknown' }))
    })
    expect(rendered.result.current.conversation).toBe(before)
    await act(async () => {
      handlers.get('subagent')?.(subagentProgress({ parentRunId: '', preview: 'Unsequenced update' }))
    })
    expect(rendered.result.current.conversation!.messages[0].toolCalls?.[0].structuredContent).toMatchObject({
      subagentProgressByTask: { 'old-worker': { preview: 'Unsequenced update' } },
    })
    expect(getSnapshot().toolCalls[0].structuredContent).toMatchObject({
      subagentProgressByTask: { 'live-worker': { taskId: 'live-worker', preview: 'Live step' } },
    })
  })

  it('projects unsequenced progress only onto the exact live parent while terminal reload is pending', async () => {
    const rendered = await setupStreamedAnswer()
    await act(async () => {
      handlers.get('tool')?.({
        conversationId: CONVERSATION_ID, runId: 'run-1', messageId: TWIN_ID,
        toolCallId: 'live-parent', name: 'task', source: 'native', status: 'running',
      })
    })
    await flush()
    let resolveReload: (conversation: Conversation) => void = () => {}
    mockGetConversation.mockImplementationOnce(() => new Promise((resolve) => { resolveReload = resolve }))
    await act(async () => { emitStream(packet('run_completed', 'run-1')) })
    await flush()
    expect(getCoarse().streamFrozen).toBe(true)
    const before = rendered.result.current.conversation

    await act(async () => {
      handlers.get('subagent')?.(subagentProgress({
        parentRunId: '', parentToolCallId: 'live-parent', taskId: 'live-worker', preview: 'Unsequenced step',
      }))
    })
    await flush()
    expect(rendered.result.current.conversation).toBe(before)
    expect(getSnapshot().toolCalls[0].structuredContent).toMatchObject({
      subagentProgressByTask: { 'live-worker': { preview: 'Unsequenced step' } },
    })

    await act(async () => {
      handlers.get('subagent')?.(subagentProgress({
        parentRunId: '', parentToolCallId: 'missing', taskId: 'other-worker', preview: 'Wrong parent',
      }))
      handlers.get('subagent')?.(subagentProgress({
        parentRunId: 'older-run', parentToolCallId: 'live-parent', taskId: 'live-worker', preview: 'Wrong run',
      }))
    })
    await flush()
    expect(getSnapshot().toolCalls[0].structuredContent).toMatchObject({
      subagentProgressByTask: { 'live-worker': { preview: 'Unsequenced step' } },
    })
    await act(async () => { resolveReload(withTwin) })
    await flush()
  })

  it('keeps the idle external CLI fallback when progress has no parent tool id', async () => {
    mockGetConversation.mockResolvedValueOnce(conversationWith([
      { id: TWIN_ID, role: 'assistant', content: '', timestamp: 1, toolCalls: [
        { id: 'external-parent', name: 'Agent', source: 'external_cli', status: 'running' },
      ] },
    ]))
    const rendered = renderHook(() => usePopoutSession(CONVERSATION_ID, 'zh'))
    await flush()
    await act(async () => {
      handlers.get('subagent')?.(subagentProgress({ parentRunId: '', parentToolCallId: '', taskId: 'external-worker' }))
    })
    expect(rendered.result.current.conversation!.messages[0].toolCalls?.[0].structuredContent).toMatchObject({
      subagentProgress: { taskId: 'external-worker', preview: 'Working' },
    })
  })

  it('ignores progress for the previous conversation during a route change', async () => {
    mockGetConversation.mockResolvedValueOnce(conversationWith([
      { id: TWIN_ID, role: 'assistant', content: '', timestamp: 1, toolCalls: [{ id: 'old-parent', name: 'Task' }] },
    ]))
    const rendered = renderHook(({ id }) => usePopoutSession(id, 'zh'), { initialProps: { id: CONVERSATION_ID } })
    await flush()
    const before = rendered.result.current.conversation
    mockGetConversation.mockImplementationOnce(() => new Promise(() => {}))
    rendered.rerender({ id: 'c2' })
    await act(async () => { handlers.get('subagent')?.(subagentProgress()) })
    expect(rendered.result.current.conversation).toBe(before)
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

    // Observe the synchronous store notification, not just act's final render:
    // clearing here before React commits leaves neither answer visible.
    const clearedBeforeCommit: boolean[] = []
    const unsubscribe = subscribeSnapshot(() => {
      if (!getSnapshot().content) {
        clearedBeforeCommit.push(!rendered.getCommittedMessages().some((m) => m.id === TWIN_ID))
      }
    })
    await act(async () => { resolveReload(withTwin) })
    await flush()
    unsubscribe()
    expect(clearedBeforeCommit).toEqual([false])

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
    expect(vi.getTimerCount()).toBeGreaterThan(0)
    expect(getSnapshot().content).toBe('answer')

    await act(async () => { await vi.advanceTimersByTimeAsync(200) })
    expect(getSnapshot().content).toBe('')
    expect(getCoarse().streamFrozen).toBe(false)
  })

  it.each(['invoke result', 'terminal reload'] as const)('waits for the committed twin after a local send: %s', async (resultPath) => {
    mockGetConversation.mockResolvedValueOnce(userOnly)
    let resolveSend: (conversation: Conversation) => void = () => {}
    let rejectSend: (error: Error) => void = () => {}
    mockSendMessage.mockImplementationOnce(() => new Promise((resolve, reject) => {
      resolveSend = resolve
      rejectSend = reject
    }))
    let committedMessages: Conversation['messages'] = []
    const rendered = renderHook(() => {
      const session = usePopoutSession(CONVERSATION_ID, 'zh')
      useLayoutEffect(() => { committedMessages = session.conversation?.messages ?? [] }, [session.conversation])
      return session
    })
    await flush()
    let sending: Promise<boolean | void> = Promise.resolve(false)
    await act(async () => {
      sending = Promise.resolve(rendered.result.current.inputBarProps.onSend('hello', []))
      emitStream(packet('run_started', 'run-1'))
      emitStream(packet('text_delta', 'run-1', 'answer'))
    })
    await flush()
    expect(getSnapshot().content).toBe('answer')
    const clearedBeforeCommit: boolean[] = []
    const unsubscribe = subscribeSnapshot(() => {
      if (!getSnapshot().content) clearedBeforeCommit.push(!committedMessages.some((m) => m.id === TWIN_ID))
    })
    await act(async () => {
      if (resultPath === 'terminal reload') {
        mockGetConversation.mockResolvedValueOnce(withTwin)
        emitStream(packet('run_completed', 'run-1'))
        rejectSend(new Error('invoke response unavailable'))
      } else {
        resolveSend(withTwin)
      }
      await sending
    })
    await flush()
    unsubscribe()
    expect(clearedBeforeCommit).toEqual([false])
    expect(rendered.result.current.conversation?.messages).toEqual(withTwin.messages)
    expect(getCoarse()).toMatchObject({ streaming: false, streamFrozen: false })
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

  it('returns uncommitted failure to the composer so an accepted draft can be restored', async () => {
    mockGetConversation.mockResolvedValueOnce(userOnly)
    mockSendMessage.mockRejectedValueOnce(new Error('disk unavailable'))
    const rendered = renderHook(() => usePopoutSession(CONVERSATION_ID, 'zh'))
    await flush()
    const accepted = vi.fn()

    let sent: boolean | void = undefined
    await act(async () => {
      sent = await rendered.result.current.inputBarProps.onSend('hello', [], { onAccepted: accepted })
    })

    expect(accepted).toHaveBeenCalledTimes(1)
    expect(sent).toBe(false)
    expect(mockSendMessage).toHaveBeenCalledTimes(1)
    expect(rendered.result.current.messageListProps.messages.map((message) => message.id))
      .toEqual(['user-1'])
  })

  it('shows one optimistic send, rejects a duplicate, then replaces it with persisted messages', async () => {
    mockGetConversation.mockResolvedValueOnce(userOnly)
    let resolveSend: (conversation: Conversation) => void = () => {}
    mockSendMessage.mockImplementationOnce(() => new Promise((resolve) => { resolveSend = resolve }))
    const rendered = renderHook(() => usePopoutSession(CONVERSATION_ID, 'zh'))
    await flush()
    const accepted = vi.fn()
    let first: Promise<boolean | void> = Promise.resolve(false)
    await act(async () => {
      first = Promise.resolve(rendered.result.current.inputBarProps.onSend(' hello ', [], { onAccepted: accepted }))
    })

    expect(accepted).toHaveBeenCalledTimes(1)
    expect(rendered.result.current.messageListProps.messages.map((message) => message.content))
      .toEqual(['hi', 'hello'])
    let duplicate: boolean | void = undefined
    await act(async () => {
      duplicate = await rendered.result.current.inputBarProps.onSend('again', [], { onAccepted: accepted })
    })
    expect(duplicate).toBe(false)
    expect(mockSendMessage).toHaveBeenCalledTimes(1)

    const persisted = conversationWith([
      ...userOnly.messages,
      { id: 'user-2', role: 'user', content: 'hello', timestamp: 2 },
      { id: TWIN_ID, role: 'assistant', content: 'answer', timestamp: 3 },
    ])
    await act(async () => { resolveSend(persisted); await first })
    expect(rendered.result.current.messageListProps.messages.map((message) => message.id))
      .toEqual(['user-1', 'user-2', TWIN_ID])
    mockSendMessage.mockResolvedValueOnce(persisted)
    await act(async () => {
      expect(await rendered.result.current.inputBarProps.onSend('next', [])).toBe(true)
    })
    expect(mockSendMessage).toHaveBeenCalledTimes(2)
  })

  it('keeps an accepted draft cleared when the send committed before an error', async () => {
    mockGetConversation.mockResolvedValueOnce(userOnly)
    const persisted = conversationWith([
      ...userOnly.messages,
      { id: 'user-2', role: 'user', content: 'hello', timestamp: 2 },
    ])
    mockSendMessage.mockRejectedValueOnce(Object.assign(new Error('generation failed'), {
      conversation: persisted,
    }))
    const rendered = renderHook(() => usePopoutSession(CONVERSATION_ID, 'zh'))
    await flush()
    const accepted = vi.fn()
    let sent: boolean | void = undefined
    await act(async () => {
      sent = await rendered.result.current.inputBarProps.onSend('hello', [], { onAccepted: accepted })
    })

    expect(sent).toBe(true)
    expect(accepted).toHaveBeenCalledTimes(1)
    expect(rendered.result.current.messageListProps.messages.map((message) => message.id))
      .toEqual(['user-1', 'user-2'])
    expect(rendered.result.current.streamError).toBe('generation failed')
  })

  it('keeps a later title update when an older send result arrives afterward', async () => {
    mockGetConversation.mockResolvedValueOnce(userOnly)
    let resolveSend: (conversation: Conversation) => void = () => {}
    mockSendMessage.mockImplementationOnce(() => new Promise((resolve) => { resolveSend = resolve }))
    const rendered = renderHook(() => usePopoutSession(CONVERSATION_ID, 'zh'))
    await flush()
    let sending: Promise<boolean | void> = Promise.resolve(false)
    await act(async () => {
      sending = Promise.resolve(rendered.result.current.inputBarProps.onSend('hello', []))
    })
    const titled = { ...withTwin, revision: 2, title: 'Summary' }
    mockGetConversation.mockResolvedValueOnce(titled)
    await act(async () => {
      handlers.get('title')?.({ conversationId: CONVERSATION_ID, revision: 2, title: 'Summary' })
    })
    await flush()
    await act(async () => { resolveSend(withTwin); await sending })
    expect(rendered.result.current.conversation?.title).toBe('Summary')
    expect(rendered.result.current.conversation?.revision).toBe(2)
    expect(rendered.result.current.conversation?.messages).toHaveLength(2)
  })

  it('resumes a recovered stream when cancellation fails', async () => {
    const rendered = await setupStreamedAnswer()
    mockCancelStream.mockRejectedValueOnce(new Error('cancel unavailable'))
    const report = vi.spyOn(console, 'error').mockImplementation(() => {})

    await act(async () => { await rendered.result.current.inputBarProps.onCancel?.() })

    expect(mockCancelStream).toHaveBeenCalledWith(CONVERSATION_ID)
    expect(getCoarse()).toMatchObject({ streaming: true, cancelling: false })
    expect(rendered.result.current.streamError).toBe('cancel unavailable')
    report.mockRestore()
  })
})
