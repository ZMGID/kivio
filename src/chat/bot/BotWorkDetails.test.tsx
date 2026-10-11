// @vitest-environment jsdom
import { act, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, expect, it, vi } from 'vitest'
import { api } from '../../api/tauri'
import type { ScheduledTask } from '../../api/scheduledTaskContracts'
import { useScheduledTasks } from '../scheduledTasks/useScheduledTasks'
import { BotWorkDetails } from './BotWorkDetails'

vi.mock('../scheduledTasks/useScheduledTasks', () => ({ useScheduledTasks: vi.fn() }))
vi.mock('../../api/tauri', () => ({ api: {
  chatArtifactsList: vi.fn(), scheduledTaskRunNow: vi.fn(),
  scheduledTaskSetEnabled: vi.fn(), scheduledTaskRuns: vi.fn(),
} }))

afterEach(() => vi.restoreAllMocks())

it('keeps failures visible while task actions disclose and require confirmation before real execution', async () => {
  const task: ScheduledTask = {
    id: 'routine', name: '每日简报', prompt: '整理简报', conversationId: 'conv_bot_ada',
    schedule: { kind: 'daily', hour: 9, minute: 0 }, enabled: true, status: 'active',
    nextRunAt: null, lastRunAt: null, runCount: 0, lastError: '来源无法连接', source: 'user', createdAt: 1, updatedAt: 1,
  }
  vi.mocked(useScheduledTasks).mockReturnValue({ tasks: [task], error: '', revision: 1, liveRuns: new Map(), refresh: vi.fn() })
  vi.mocked(api.chatArtifactsList).mockResolvedValue({ items: [], warnings: 0 })
  vi.mocked(api.scheduledTaskRunNow).mockRejectedValue(new Error('运行失败，请重试'))
  const user = userEvent.setup()
  render(<BotWorkDetails conversationId={task.conversationId} onOpenArtifacts={vi.fn()} />)
  expect(screen.getByRole('alert')).toHaveTextContent('来源无法连接')
  expect(screen.getByRole('button', { name: '立即运行' })).not.toBeVisible()
  const summary = screen.getByText(task.name).closest('summary')!
  await user.click(summary)
  const run = screen.getByRole('button', { name: '立即运行' })
  expect(run).toBeVisible()
  const confirm = vi.spyOn(window, 'confirm').mockReturnValueOnce(false).mockReturnValueOnce(true)
  await user.click(run)
  expect(confirm).toHaveBeenCalledOnce()
  expect(api.scheduledTaskRunNow).not.toHaveBeenCalled()
  await user.click(run)
  expect(api.scheduledTaskRunNow).toHaveBeenCalledWith(task.id)
  expect(await screen.findByText('运行失败，请重试')).toHaveAttribute('role', 'alert')
  expect(run).toBeEnabled()
})


it('shows read errors only in their own tab and ignores an artifact request after leaving', async () => {
  vi.mocked(useScheduledTasks).mockReturnValue({ tasks: [], error: '任务读取失败', revision: 1, liveRuns: new Map(), refresh: vi.fn() })
  let resolveQuick!: (page: Awaited<ReturnType<typeof api.chatArtifactsList>>) => void
  vi.mocked(api.chatArtifactsList).mockImplementationOnce(() => new Promise(resolve => { resolveQuick = resolve }))
  const props = { conversationId: 'conv_bot_ada', onOpenArtifacts: vi.fn() }
  const { rerender } = render(<BotWorkDetails {...props} />)
  expect(screen.getByRole('alert')).toHaveTextContent('任务读取失败')
  rerender(<BotWorkDetails {...props} section="artifacts" />)
  expect(screen.queryByRole('alert')).not.toBeInTheDocument()
  rerender(<BotWorkDetails {...props} />)
  await act(async () => { resolveQuick({ items: [], warnings: 0 }) })
  expect(api.chatArtifactsList).toHaveBeenCalledTimes(1)
  vi.mocked(api.chatArtifactsList).mockRejectedValueOnce(new Error('成果读取失败'))
  rerender(<BotWorkDetails {...props} section="artifacts" />)
  expect(await screen.findByRole('alert')).toHaveTextContent('成果读取失败')
  rerender(<BotWorkDetails {...props} />)
  expect(screen.getByRole('alert')).toHaveTextContent('任务读取失败')
  expect(screen.queryByText('成果读取失败')).not.toBeInTheDocument()
})
