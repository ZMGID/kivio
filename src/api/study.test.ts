import { beforeEach, describe, expect, it, vi } from 'vitest'
import { streamStudyCompletion, type StudyCompletionInput, type StudyStreamEvent } from './study'

const { invokeMock, isTauriRuntimeMock, channels } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  isTauriRuntimeMock: vi.fn(),
  channels: [] as Array<{ onmessage: (event: StudyStreamEvent) => void }>,
}))
// Mocked IPC only. These tests do not contact a provider or generate tutoring content.
vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
  Channel: class {
    onmessage: (event: StudyStreamEvent) => void = () => {}
    constructor() { channels.push(this) }
  },
}))
vi.mock('./tauri', () => ({ isTauriRuntime: isTauriRuntimeMock }))

function input(): StudyCompletionInput {
  return { requestId: 'request-1', providerId: 'provider-1', model: 'model-1', systemPrompt: 'Tutor prompt', userPrompt: 'Page context', history: [] }
}
function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: unknown) => void
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}
function event(index: number, type: 'started' | 'delta', delta = 'part'): StudyStreamEvent {
  const request = invokeMock.mock.calls.filter(call => call[0] === 'study_request_help')[index][1].input
  return type === 'started'
    ? { type, requestId: request.requestId, transportId: request.transportId }
    : { type, delta, requestId: request.requestId, transportId: request.transportId }
}

beforeEach(() => {
  vi.resetAllMocks()
  channels.length = 0
  isTauriRuntimeMock.mockReturnValue(true)
})

describe('isolated Study IPC lifecycle', () => {
  it('refuses browser previews instead of producing scripted AI answers', async () => {
    isTauriRuntimeMock.mockReturnValue(false)
    await expect(streamStudyCompletion(input(), vi.fn(), new AbortController().signal)).rejects.toThrow('desktop app')
    expect(invokeMock).not.toHaveBeenCalled()
  })

  it('does not start a model request when already cancelled', async () => {
    const controller = new AbortController()
    controller.abort()
    await expect(streamStudyCompletion(input(), vi.fn(), controller.signal)).rejects.toMatchObject({ name: 'AbortError' })
    expect(invokeMock).not.toHaveBeenCalled()
  })

  it('delivers only correlated text and resolves the provider result without touching Chat or Lens', async () => {
    const pending = deferred<{ requestId: string; content: string }>()
    invokeMock.mockReturnValue(pending.promise)
    const onDelta = vi.fn()
    const controller = new AbortController()
    const removeListener = vi.spyOn(controller.signal, 'removeEventListener')
    const result = streamStudyCompletion(input(), onDelta, controller.signal)
    channels[0].onmessage(event(0, 'started'))
    channels[0].onmessage({ ...event(0, 'delta'), requestId: 'another-document' })
    channels[0].onmessage(event(0, 'delta', 'First hint'))
    pending.resolve({ requestId: 'request-1', content: 'First hint complete' })
    await expect(result).resolves.toEqual({ requestId: 'request-1', content: 'First hint complete' })
    channels[0].onmessage(event(0, 'delta', 'late text'))
    expect(onDelta).toHaveBeenCalledTimes(1)
    expect(onDelta).toHaveBeenCalledWith('First hint')
    expect(removeListener).toHaveBeenCalledWith('abort', expect.any(Function))
    expect(invokeMock.mock.calls.map(call => call[0])).toEqual(['study_request_help'])
  })

  it('cancels after registration when Stop arrives before the backend starts', async () => {
    const pending = deferred<{ requestId: string; content: string }>()
    invokeMock.mockImplementation(command => command === 'study_request_help' ? pending.promise : Promise.resolve())
    const controller = new AbortController()
    const onDelta = vi.fn()
    const result = streamStudyCompletion(input(), onDelta, controller.signal)
    const rejection = expect(result).rejects.toMatchObject({ name: 'AbortError' })
    controller.abort()
    await rejection
    expect(invokeMock).toHaveBeenCalledTimes(1)
    channels[0].onmessage(event(0, 'started'))
    channels[0].onmessage(event(0, 'delta'))
    expect(invokeMock).toHaveBeenLastCalledWith('study_cancel_request', { transportId: event(0, 'started').transportId })
    expect(onDelta).not.toHaveBeenCalled()
    pending.resolve({ requestId: 'request-1', content: 'late answer' })
    await Promise.resolve()
  })

  it('isolates cancelled retries with distinct transport attempt IDs', async () => {
    const old = deferred<{ requestId: string; content: string }>()
    const next = deferred<{ requestId: string; content: string }>()
    let starts = 0
    invokeMock.mockImplementation(command => command === 'study_request_help' ? (++starts === 1 ? old.promise : next.promise) : Promise.resolve())
    const controller = new AbortController()
    const oldDelta = vi.fn()
    const oldResult = streamStudyCompletion(input(), oldDelta, controller.signal)
    const rejection = expect(oldResult).rejects.toMatchObject({ name: 'AbortError' })
    channels[0].onmessage(event(0, 'started'))
    controller.abort()
    await rejection
    const nextDelta = vi.fn()
    const nextResult = streamStudyCompletion(input(), nextDelta, new AbortController().signal)
    expect(event(0, 'started').transportId).not.toBe(event(1, 'started').transportId)
    channels[1].onmessage(event(1, 'started'))
    channels[0].onmessage(event(0, 'delta', 'old response'))
    channels[1].onmessage(event(0, 'delta', 'wrong correlation'))
    channels[1].onmessage(event(1, 'delta', 'new response'))
    old.resolve({ requestId: 'request-1', content: 'old terminal' })
    next.resolve({ requestId: 'request-1', content: 'new terminal' })
    await expect(nextResult).resolves.toMatchObject({ content: 'new terminal' })
    expect(oldDelta).not.toHaveBeenCalled()
    expect(nextDelta).toHaveBeenCalledTimes(1)
    expect(nextDelta).toHaveBeenCalledWith('new response')
    expect(invokeMock.mock.calls.filter(call => call[0] === 'study_cancel_request')).toHaveLength(1)
  })

  it('reports a provider error and leaves the request retryable', async () => {
    invokeMock.mockRejectedValueOnce('Provider authentication expired')
      .mockResolvedValueOnce({ requestId: 'request-1', content: 'Retry response' })
    await expect(streamStudyCompletion(input(), vi.fn(), new AbortController().signal)).rejects.toThrow('Provider authentication expired')
    await expect(streamStudyCompletion(input(), vi.fn(), new AbortController().signal)).resolves.toMatchObject({ content: 'Retry response' })
  })

  it('rejects a mismatched terminal response', async () => {
    invokeMock.mockResolvedValue({ requestId: 'wrong', content: 'Other request content' })
    await expect(streamStudyCompletion(input(), vi.fn(), new AbortController().signal)).rejects.toThrow('another request')
  })

  it('cancels the provider if the streaming consumer fails', async () => {
    const pending = deferred<{ requestId: string; content: string }>()
    invokeMock.mockImplementation(command => command === 'study_request_help' ? pending.promise : Promise.resolve())
    const result = streamStudyCompletion(input(), () => { throw new Error('View failed') }, new AbortController().signal)
    channels[0].onmessage(event(0, 'started'))
    channels[0].onmessage(event(0, 'delta'))
    await expect(result).rejects.toThrow('View failed')
    expect(invokeMock).toHaveBeenLastCalledWith('study_cancel_request', expect.any(Object))
    pending.reject('cancelled')
    await Promise.resolve()
  })
})
