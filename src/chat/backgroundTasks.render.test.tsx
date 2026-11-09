import { StrictMode } from 'react'
import { act, cleanup, fireEvent, render, renderHook, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { api, type BackgroundTaskInfo } from '../api/tauri'
import { stopBackgroundTask, useBackgroundTasks } from './useBackgroundTasks'
import { BackgroundTasksPanel } from './dock/BackgroundTasksPanel'

vi.mock('../api/tauri', () => ({ api: {
  chatListBackgroundTasks: vi.fn(),
  chatKillBackgroundCommand: vi.fn(),
  chatStopExternalBackgroundTask: vi.fn(),
  chatClearFinishedBackgroundTasks: vi.fn(),
} }))

function TaskSubscriber({ conversationId }: { conversationId: string }) {
  useBackgroundTasks(conversationId)
  return null
}

beforeEach(() => {
  vi.useFakeTimers()
  vi.spyOn(document, 'hidden', 'get').mockReturnValue(false)
  vi.mocked(api.chatListBackgroundTasks).mockResolvedValue([])
})

afterEach(async () => {
  cleanup()
  await vi.runAllTicks()
  vi.useRealTimers()
  vi.clearAllMocks()
  vi.restoreAllMocks()
})

const running: BackgroundTaskInfo = {
  id: 'task-1', source: 'builtin', kind: 'bash', title: 'Build project',
  status: 'running', elapsedSecs: 1, startedAtMs: 1,
}

describe('background task consumers', () => {
  it('deduplicates stops across consumers and updates only the matching task source', async () => {
    const external: BackgroundTaskInfo = { ...running, source: 'external', title: 'External build' }
    vi.mocked(api.chatListBackgroundTasks).mockResolvedValue([running, external])
    let confirmStop: () => void = () => { throw new Error('Stop request not started') }
    vi.mocked(api.chatKillBackgroundCommand).mockReturnValueOnce(new Promise<void>(resolve => { confirmStop = resolve }))
    const { result } = renderHook(() => useBackgroundTasks('conv_dedupe'))
    await act(async () => { await Promise.resolve() })
    const requests: Promise<void>[] = []
    await act(async () => {
      requests.push(stopBackgroundTask('conv_dedupe', running), stopBackgroundTask('conv_dedupe', running))
      await Promise.resolve()
    })
    expect(api.chatKillBackgroundCommand).toHaveBeenCalledTimes(1)
    expect(result.current.map(task => task.status)).toEqual(['running', 'running'])
    await act(async () => { confirmStop(); await Promise.all(requests) })
    expect(result.current.map(task => task.status)).toEqual(['stopped', 'running'])
  })

  it('keeps a failed stop running and allows a retry', async () => {
    vi.mocked(api.chatListBackgroundTasks).mockResolvedValue([running])
    vi.mocked(api.chatKillBackgroundCommand).mockRejectedValueOnce(new Error('Stop failed')).mockResolvedValueOnce(undefined)
    const { result } = renderHook(() => useBackgroundTasks('conv_retry'))
    await act(async () => { await Promise.resolve() })
    await act(async () => { await expect(stopBackgroundTask('conv_retry', running)).rejects.toThrow('Stop failed') })
    expect(result.current[0].status).toBe('running')
    await act(async () => { await stopBackgroundTask('conv_retry', running) })
    expect(result.current[0].status).toBe('stopped')
  })

  it('uses the current conversation for external tasks without a separate conversation id', async () => {
    const external: BackgroundTaskInfo = { ...running, source: 'external' }
    vi.mocked(api.chatListBackgroundTasks).mockResolvedValue([external])
    vi.mocked(api.chatStopExternalBackgroundTask).mockResolvedValueOnce(undefined)
    const { result } = renderHook(() => useBackgroundTasks('conv_external'))
    await act(async () => { await Promise.resolve() })
    await act(async () => { await stopBackgroundTask('conv_external', external) })
    expect(api.chatStopExternalBackgroundTask).toHaveBeenCalledWith('conv_external', running.id)
    expect(result.current[0].status).toBe('stopped')
  })

  it('shares each request between subscribers and the open task panel', async () => {
    render(<>
      <TaskSubscriber conversationId="conv_shared" />
      <BackgroundTasksPanel active lang="en" conversationId="conv_shared" />
    </>)
    await act(async () => { await Promise.resolve() })
    expect(api.chatListBackgroundTasks).toHaveBeenCalledTimes(1)
    await act(async () => { await vi.advanceTimersByTimeAsync(2500) })
    expect(api.chatListBackgroundTasks).toHaveBeenCalledTimes(2)
  })

  it('does not overlap slow reads, and stops when the last consumer leaves', async () => {
    let resolve!: (value: BackgroundTaskInfo[]) => void
    vi.mocked(api.chatListBackgroundTasks).mockImplementationOnce(() => new Promise((done) => { resolve = done }))
    const view = render(<StrictMode><TaskSubscriber conversationId="conv_slow" /></StrictMode>)
    await act(async () => { await vi.advanceTimersByTimeAsync(10000) })
    expect(api.chatListBackgroundTasks).toHaveBeenCalledTimes(1)
    view.unmount()
    await act(async () => { await vi.runAllTicks(); resolve([running]); await Promise.resolve() })
    await act(async () => { await vi.advanceTimersByTimeAsync(10000) })
    expect(api.chatListBackgroundTasks).toHaveBeenCalledTimes(1)
  })

  it('ignores a late response after changing conversations', async () => {
    let resolve!: (value: BackgroundTaskInfo[]) => void
    vi.mocked(api.chatListBackgroundTasks).mockImplementationOnce(() => new Promise((done) => { resolve = done }))
    const view = render(<BackgroundTasksPanel active lang="en" conversationId="conv_old" />)
    view.rerender(<BackgroundTasksPanel active lang="en" conversationId="conv_new" />)
    await act(async () => { await vi.runAllTicks(); resolve([running]); await Promise.resolve() })
    expect(screen.queryByText(running.title)).not.toBeInTheDocument()
    expect(api.chatListBackgroundTasks).toHaveBeenLastCalledWith('conv_new')
  })

  it('pauses while hidden and refreshes immediately when visible again', async () => {
    render(<TaskSubscriber conversationId="conv_visibility" />)
    await act(async () => { await Promise.resolve() })
    vi.spyOn(document, 'hidden', 'get').mockReturnValue(true)
    fireEvent(document, new Event('visibilitychange'))
    await act(async () => { await vi.advanceTimersByTimeAsync(10000) })
    expect(api.chatListBackgroundTasks).toHaveBeenCalledTimes(1)
    vi.spyOn(document, 'hidden', 'get').mockReturnValue(false)
    await act(async () => { fireEvent(document, new Event('visibilitychange')); await Promise.resolve() })
    expect(api.chatListBackgroundTasks).toHaveBeenCalledTimes(2)
  })

  it('shares a confirmed stop and protects it from an older pending read', async () => {
    let resolve!: (value: BackgroundTaskInfo[]) => void
    vi.mocked(api.chatListBackgroundTasks)
      .mockResolvedValueOnce([running])
      .mockImplementationOnce(() => new Promise((done) => { resolve = done }))
    vi.mocked(api.chatKillBackgroundCommand).mockResolvedValueOnce(undefined)
    render(<>
      <TaskSubscriber conversationId="conv_stop" />
      <BackgroundTasksPanel active lang="en" conversationId="conv_stop" />
    </>)
    await act(async () => { await Promise.resolve() })
    await act(async () => { await vi.advanceTimersByTimeAsync(2500) })
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: /Build project/ })); await Promise.resolve() })
    expect(screen.queryByRole('button', { name: /Build project/ })).not.toBeInTheDocument()
    await act(async () => { resolve([running]); await Promise.resolve() })
    expect(screen.queryByRole('button', { name: /Build project/ })).not.toBeInTheDocument()
  })
})
