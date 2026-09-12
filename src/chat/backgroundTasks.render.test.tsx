import { StrictMode } from 'react'
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { api, type BackgroundTaskInfo } from '../api/tauri'
import { BackgroundJobsIndicator } from './BackgroundJobsIndicator'
import { BackgroundTasksPanel } from './dock/BackgroundTasksPanel'

vi.mock('../api/tauri', () => ({ api: {
  chatListBackgroundTasks: vi.fn(),
  chatKillBackgroundCommand: vi.fn(),
  chatStopExternalBackgroundTask: vi.fn(),
  chatClearFinishedBackgroundTasks: vi.fn(),
} }))

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
  it('shares each request between the header indicator and open task panel', async () => {
    render(<>
      <BackgroundJobsIndicator conversationId="conv_shared" onOpen={() => {}} />
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
    const view = render(<StrictMode><BackgroundJobsIndicator conversationId="conv_slow" onOpen={() => {}} /></StrictMode>)
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
    render(<BackgroundJobsIndicator conversationId="conv_visibility" onOpen={() => {}} />)
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
      <BackgroundJobsIndicator conversationId="conv_stop" onOpen={() => {}} />
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
