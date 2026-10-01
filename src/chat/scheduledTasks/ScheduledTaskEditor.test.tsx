import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { api } from '../../api/tauri'
import { confirmDialog } from '../../components/dialogQueue'
import type { ScheduledTask } from '../../api/scheduledTaskContracts'
import { chatApi } from '../api'
import { ScheduledTaskEditor } from './ScheduledTaskEditor'

vi.mock('../../api/tauri', () => ({ api: {
  scheduledTaskPreview: vi.fn(), scheduledTaskSave: vi.fn(), scheduledTaskRunNow: vi.fn(),
  scheduledTaskRuns: vi.fn(), onScheduledTasksChanged: vi.fn(),
} }))
vi.mock('../api', () => ({ chatApi: { getProjects: vi.fn().mockResolvedValue([]), getConversation: vi.fn(), getConversations: vi.fn().mockResolvedValue([]) } }))
vi.mock('../ModelSelector', () => ({ ModelSelector: () => null }))
vi.mock('../../components/dialogQueue', () => ({ confirmDialog: vi.fn() }))

const task: ScheduledTask = {
  id: 'task', name: 'Research', prompt: 'Summarize research', schedule: { kind: 'interval', minutes: 90, anchorAt: 1234567890 },
  conversationId: 'bound-conversation', enabled: true, status: 'active', nextRunAt: 2000000000, lastRunAt: null,
  runCount: 0, lastError: null, source: 'user', createdAt: 1, updatedAt: 1,
}
beforeAll(() => {
  HTMLDialogElement.prototype.showModal ??= function showModal(this: HTMLDialogElement) { this.setAttribute('open', '') }
})
beforeEach(() => {
  vi.clearAllMocks()
  vi.mocked(api.scheduledTaskPreview).mockResolvedValue([2000000000])
  vi.mocked(api.scheduledTaskRuns).mockResolvedValue([])
  vi.mocked(api.onScheduledTasksChanged).mockResolvedValue(() => undefined)
  vi.mocked(chatApi.getConversation).mockResolvedValue({
    id: task.conversationId, title: 'Research conversation', revision: 0, provider_id: 'provider', model: 'model',
    messages: [], created_at: 1, updated_at: 1,
  })
  vi.mocked(chatApi.getConversations).mockResolvedValue([{
    id: 'picked', title: 'Picked conversation', preview: '', provider_id: 'provider', model: 'model',
    message_count: 0, created_at: 1, updated_at: 1,
  }])
  vi.mocked(api.scheduledTaskSave).mockImplementation(async input => ({
    ...task, id: input.id || 'created', name: input.name, prompt: input.prompt, schedule: input.schedule,
    enabled: input.enabled ?? true,
    conversationId: input.target.kind === 'conversation' ? input.target.conversationId : 'created-conversation',
  }))
  vi.mocked(api.scheduledTaskRunNow).mockRejectedValue('Run unavailable')
  vi.mocked(confirmDialog).mockResolvedValue(false)
})

async function ready() {
  await waitFor(() => expect(screen.getByRole('button', { name: '保存' })).toBeEnabled())
}

describe('ScheduledTaskEditor', () => {
  it('allows unchanged close but keeps changes to the selected new-conversation settings when discard is canceled', async () => {
    const close = vi.fn()
    const view = render(<ScheduledTaskEditor task={null} onClose={close} onSaved={() => undefined} />)
    fireEvent.click(screen.getByRole('button', { name: '取消' }))
    await waitFor(() => expect(close).toHaveBeenCalledTimes(1))
    view.unmount()
    close.mockClear()
    render(<ScheduledTaskEditor task={null} onClose={close} onSaved={() => undefined} />)
    fireEvent.click(screen.getByRole('button', { name: '思考等级' }))
    fireEvent.click(screen.getByRole('option', { name: '高' }))
    fireEvent.click(screen.getByRole('button', { name: /已有对话/ }))
    fireEvent.click(screen.getByRole('button', { name: /新建对话 保存时/ }))
    fireEvent.click(screen.getByRole('button', { name: '取消' }))
    await waitFor(() => expect(confirmDialog).toHaveBeenCalledTimes(1))
    expect(close).not.toHaveBeenCalled()
    expect(screen.getByRole('button', { name: '思考等级' })).toHaveTextContent('高')
  })

  it('restores per-kind edits across switches and retains an existing interval anchor in the saved rule', async () => {
    render(<ScheduledTaskEditor task={task} onClose={() => undefined} onSaved={() => undefined} />)
    fireEvent.change(screen.getByRole('spinbutton', { name: '分钟' }), { target: { value: '135' } })
    fireEvent.click(screen.getByRole('button', { name: '每月' }))
    fireEvent.click(screen.getByRole('button', { name: '31' }))
    fireEvent.change(screen.getByLabelText('时间'), { target: { value: '17:45' } })
    fireEvent.click(screen.getByRole('button', { name: '每年' }))
    fireEvent.click(screen.getByRole('button', { name: '每月' }))
    expect(screen.getByRole('button', { name: '31' })).toHaveAttribute('aria-pressed', 'true')
    expect(screen.getByLabelText('时间')).toHaveValue('17:45')
    fireEvent.click(screen.getAllByRole('button', { name: '间隔' })[0])
    expect(screen.getByRole('spinbutton', { name: '分钟' })).toHaveValue(135)
    await ready()
    fireEvent.click(screen.getByRole('button', { name: '保存' }))
    await waitFor(() => expect(api.scheduledTaskSave).toHaveBeenCalledWith(expect.objectContaining({ schedule: { kind: 'interval', minutes: 135, anchorAt: 1234567890 } })))
  })

  it('does not run after failed save, preserves inputs after run failure, and reuses the created id on retry', async () => {
    const saved = vi.fn()
    render(<ScheduledTaskEditor task={null} onClose={() => undefined} onSaved={saved} />)
    fireEvent.change(screen.getByRole('textbox', { name: '名称' }), { target: { value: 'New task' } })
    fireEvent.change(screen.getByRole('textbox', { name: '要让 AI 做什么' }), { target: { value: 'Prompt draft' } })
    await ready()
    vi.mocked(api.scheduledTaskSave).mockRejectedValueOnce('Save failed')
    fireEvent.click(screen.getByRole('button', { name: '保存并运行' }))
    await screen.findByText('Save failed')
    expect(api.scheduledTaskRunNow).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: '保存并运行' }))
    await screen.findByText('Run unavailable')
    expect(screen.getByRole('textbox', { name: '要让 AI 做什么' })).toHaveValue('Prompt draft')
    expect(saved).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: '保存并运行' }))
    await waitFor(() => expect(api.scheduledTaskSave).toHaveBeenLastCalledWith(expect.objectContaining({
      id: 'created', target: { kind: 'conversation', conversationId: 'created-conversation' },
    })))
  })

  it('shows accessible required errors and keeps a draft through history tab switches', async () => {
    render(<ScheduledTaskEditor task={task} onClose={() => undefined} onSaved={() => undefined} />)
    fireEvent.change(screen.getByRole('textbox', { name: '名称' }), { target: { value: '' } })
    fireEvent.change(screen.getByRole('textbox', { name: '要让 AI 做什么' }), { target: { value: '' } })
    await ready()
    fireEvent.click(screen.getByRole('button', { name: '保存' }))
    expect(screen.getByRole('textbox', { name: '名称' })).toHaveAttribute('aria-invalid', 'true')
    expect(screen.getByRole('textbox', { name: '要让 AI 做什么' })).toHaveAccessibleDescription('请输入要让 AI 做的事。')
    expect(api.scheduledTaskSave).not.toHaveBeenCalled()
    fireEvent.change(screen.getByRole('textbox', { name: '要让 AI 做什么' }), { target: { value: 'Retained draft' } })
    fireEvent.click(screen.getByRole('tab', { name: '运行记录' }))
    fireEvent.click(screen.getByRole('tab', { name: '设置' }))
    expect(screen.getByRole('textbox', { name: '要让 AI 做什么' })).toHaveValue('Retained draft')
  })

  it('creates a conversation on save by default for a new task', async () => {
    render(<ScheduledTaskEditor task={null} onClose={() => undefined} onSaved={() => undefined} />)
    fireEvent.change(screen.getByRole('textbox', { name: '名称' }), { target: { value: 'Morning brief' } })
    fireEvent.change(screen.getByRole('textbox', { name: '要让 AI 做什么' }), { target: { value: 'Summarize news' } })
    await ready()
    fireEvent.click(screen.getByRole('button', { name: '保存' }))
    await waitFor(() => expect(api.scheduledTaskSave).toHaveBeenCalledWith(expect.objectContaining({
      target: { kind: 'newConversation' },
    })))
  })

  it('preserves an existing binding and guards opening its conversation when the draft is dirty', async () => {
    const open = vi.fn()
    const close = vi.fn()
    render(<ScheduledTaskEditor task={task} onClose={close} onSaved={() => undefined} onOpenConversation={open} />)
    await screen.findByText('Research conversation')
    expect(screen.queryByRole('button', { name: /新建对话/ })).not.toBeInTheDocument()
    fireEvent.change(screen.getByRole('textbox', { name: '名称' }), { target: { value: 'Renamed' } })
    fireEvent.click(screen.getByRole('button', { name: '打开对话' }))
    await waitFor(() => expect(confirmDialog).toHaveBeenCalledTimes(1))
    expect(open).not.toHaveBeenCalled()
    expect(close).not.toHaveBeenCalled()
    await ready()
    fireEvent.click(screen.getByRole('button', { name: '保存' }))
    await waitFor(() => expect(api.scheduledTaskSave).toHaveBeenCalledWith(expect.objectContaining({
      target: { kind: 'conversation', conversationId: task.conversationId },
    })))
  })

  it('requires a conversation pick before saving to an existing conversation', async () => {
    render(<ScheduledTaskEditor task={null} onClose={() => undefined} onSaved={() => undefined} />)
    fireEvent.change(screen.getByRole('textbox', { name: '名称' }), { target: { value: 'Morning brief' } })
    fireEvent.change(screen.getByRole('textbox', { name: '要让 AI 做什么' }), { target: { value: 'Summarize news' } })
    fireEvent.click(screen.getByRole('button', { name: /已有对话/ }))
    await ready()
    fireEvent.click(screen.getByRole('button', { name: '保存' }))
    expect(api.scheduledTaskSave).not.toHaveBeenCalled()
    expect(screen.getByRole('textbox', { name: '搜索对话' })).toHaveAccessibleDescription('请选择要发送到的对话。')
    fireEvent.click(await screen.findByRole('button', { name: /Picked conversation/ }))
    fireEvent.click(screen.getByRole('button', { name: '保存' }))
    await waitFor(() => expect(api.scheduledTaskSave).toHaveBeenCalledWith(expect.objectContaining({
      target: { kind: 'conversation', conversationId: 'picked' },
    })))
  })

  it('does not mark unused conversation choices dirty after switching back to the original target', async () => {
    const close = vi.fn()
    render(<ScheduledTaskEditor task={null} onClose={close} onSaved={() => undefined} />)
    fireEvent.click(screen.getByRole('button', { name: /已有对话/ }))
    fireEvent.click(await screen.findByRole('button', { name: /Picked conversation/ }))
    fireEvent.click(screen.getByRole('button', { name: /新建对话 保存时/ }))
    fireEvent.click(screen.getByRole('button', { name: '取消' }))
    await waitFor(() => expect(close).toHaveBeenCalledTimes(1))
    expect(confirmDialog).not.toHaveBeenCalled()
  })

  it('rebinds an existing task to the selected replacement conversation', async () => {
    render(<ScheduledTaskEditor task={task} onClose={() => undefined} onSaved={() => undefined} />)
    fireEvent.click(screen.getByRole('button', { name: '更换' }))
    fireEvent.click(await screen.findByRole('button', { name: /Picked conversation/ }))
    await ready()
    fireEvent.click(screen.getByRole('button', { name: '保存' }))
    await waitFor(() => expect(api.scheduledTaskSave).toHaveBeenCalledWith(expect.objectContaining({
      target: { kind: 'conversation', conversationId: 'picked' },
    })))
  })
})
