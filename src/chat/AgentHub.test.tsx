import { act, fireEvent, render, screen, within } from '@testing-library/react'
import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { SubagentInspectionDialog, SubagentTaskList } from './AgentHub'
import type { SubagentRoster, SubagentTranscriptPage, SubagentView } from '../generated/chatProtocol'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

function agent(id = 'worker', overrides: Partial<SubagentView> = {}): SubagentView {
  return { id, parentId: 'main', generation: 1, displayName: id, agentType: 'scout', status: 'idle', depth: 1,
    lastActivity: 1000, activity: null, model: 'provider/model', progress: null, revivable: true, reviveError: null, ...overrides }
}
const page = (overrides: Partial<SubagentTranscriptPage> = {}): SubagentTranscriptPage => ({ id: 'worker', generation: 1, cursor: { fileId: 'file', offset: 10 }, reset: true,
  entries: [
    { id: 'instruction', kind: 'message', role: 'user', content: 'Check the deployment logs.', timestamp: null },
    { id: 'entry', kind: 'message', role: 'assistant', content: 'Retained history', timestamp: null },
    { id: 'tool-entry', kind: 'message', role: 'tool', content: 'read README.md', timestamp: null },
  ], hasMore: false, ...overrides })
const flush = () => act(async () => { await Promise.resolve() })
let roster: SubagentRoster
beforeEach(() => {
  vi.useFakeTimers()
  HTMLDialogElement.prototype.showModal ??= function () { this.setAttribute('open', '') }
  HTMLDialogElement.prototype.close ??= function () { this.removeAttribute('open') }
  roster = { agents: [agent()], runningCount: 0 }
  vi.mocked(invoke).mockReset().mockImplementation(async command => {
    if (command === 'chat_subagent_snapshot') return roster
    if (command === 'chat_subagent_transcript') return page()
    if (command === 'chat_subagent_activity') return [
      { agentId: 'worker', entryId: 'entry', kind: 'response', summary: 'Retained history', timestamp: null },
      { agentId: 'worker', entryId: 'tool-entry', kind: 'tool', summary: 'read README.md', timestamp: null },
    ]
    if (command === 'chat_subagent_send') return { id: 'receipt', recipient: 'worker', status: 'accepted' }
    return agent()
  })
})
afterEach(() => { vi.useRealTimers() })

describe('subagent tasks', () => {
  it('opens a separate detail dialog and returns focus to the task without expanding the page', async () => {
    const { container } = render(<SubagentTaskList conversationId="root" roster={roster} />)
    const task = screen.getByRole('button', { name: /worker/ })
    task.focus()
    fireEvent.click(task)
    await flush()
    const dialog = screen.getByRole('dialog', { name: 'worker' })
    expect(container.contains(dialog)).toBe(false)
    expect(dialog.querySelector('details, summary, [aria-expanded]')).toBeNull()
    expect(within(dialog).getByText('Retained history')).toBeVisible()
    expect(within(dialog).queryByText('Check the deployment logs.')).not.toBeInTheDocument()
    fireEvent.click(within(dialog).getByRole('button', { name: '对话' }))
    expect(within(dialog).getByText('Check the deployment logs.')).toBeVisible()
    expect(screen.queryByRole('button', { name: /恢复|复活/ })).not.toBeInTheDocument()
    fireEvent.click(within(dialog).getByRole('button', { name: '关闭' }))
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    expect(task).toHaveFocus()
  })

  it('searches recent activity and focuses the exact matching diagnostic entry', async () => {
    render(<SubagentTaskList conversationId="root" roster={roster} />)
    fireEvent.click(screen.getByRole('button', { name: '查看子代理活动' }))
    await flush()
    const dialog = screen.getByRole('dialog', { name: '子代理活动' })
    expect(within(dialog).getByText('Retained history')).toBeVisible()
    fireEvent.click(within(dialog).getByRole('button', { name: '工具' }))
    expect(within(dialog).queryByText('Retained history')).not.toBeInTheDocument()
    expect(within(dialog).getByText('read README.md')).toBeVisible()
    fireEvent.click(within(dialog).getByRole('button', { name: '全部' }))
    fireEvent.change(within(dialog).getByRole('textbox', { name: '搜索活动' }), { target: { value: 'readme' } })
    expect(within(dialog).queryByText('Retained history')).not.toBeInTheDocument()
    fireEvent.click(within(dialog).getByRole('button', { name: /read README.md/ }))
    await flush()
    expect(screen.getByRole('button', { name: '诊断' })).toHaveAttribute('aria-pressed', 'true')
    expect(within(screen.getByRole('region', { name: '诊断' })).getByText('read README.md').closest('article')).toHaveFocus()
    expect(invoke).toHaveBeenCalledWith('chat_subagent_transcript', { conversationId: 'root', agentId: 'worker', expectedGeneration: 1, cursor: null })
    fireEvent.click(screen.getByRole('button', { name: '返回活动' }))
    expect(screen.getByRole('dialog', { name: '子代理活动' })).toBeVisible()
    expect(screen.getByRole('textbox', { name: '搜索活动' })).toHaveValue('readme')
  })

  it('preserves a failed draft and stops the selected running generation', async () => {
    roster = { agents: [agent('worker', { status: 'running' })], runningCount: 1 }
    render(<SubagentInspectionDialog conversationId="root" initial={{ agentId: 'worker' }} onClose={() => {}} />)
    await flush()
    const draft = screen.getByRole('textbox', { name: '发送给代理的消息' })
    fireEvent.change(draft, { target: { value: 'Continue the investigation' } })
    vi.mocked(invoke).mockRejectedValueOnce(new Error('Provider unavailable'))
    fireEvent.click(screen.getByRole('button', { name: '发送消息' }))
    await flush()
    expect(screen.getByRole('alert')).toHaveTextContent('Provider unavailable')
    expect(draft).toHaveValue('Continue the investigation')
    fireEvent.click(screen.getByRole('button', { name: '发送消息' }))
    await flush()
    expect(draft).toHaveValue('')
    expect(invoke).toHaveBeenCalledWith('chat_subagent_send', { conversationId: 'root', agentId: 'worker', expectedGeneration: 1, content: 'Continue the investigation' })
    fireEvent.click(screen.getByRole('button', { name: '停止' }))
    await flush()
    expect(invoke).toHaveBeenCalledWith('chat_subagent_kill', { conversationId: 'root', agentId: 'worker', expectedGeneration: 1 })
  })

  it('tails append pages and replaces rewritten history without hiding a read failure', async () => {
    const { unmount } = render(<SubagentInspectionDialog conversationId="root" initial={{ agentId: 'worker' }} onClose={() => {}} />)
    await flush()
    vi.mocked(invoke).mockImplementation(async command => command === 'chat_subagent_snapshot' ? roster : page({ reset: false, cursor: { fileId: 'file', offset: 20 }, entries: [{ id: 'second', kind: 'message', role: 'assistant', content: 'Appended reply', timestamp: null }] }))
    await act(async () => { await vi.advanceTimersByTimeAsync(500) })
    expect(screen.getByText('Appended reply')).toBeVisible()
    expect(invoke).toHaveBeenCalledWith('chat_subagent_transcript', { conversationId: 'root', agentId: 'worker', expectedGeneration: 1, cursor: { fileId: 'file', offset: 10 } })
    vi.mocked(invoke).mockImplementation(async command => command === 'chat_subagent_snapshot' ? roster : page({ entries: [{ id: 'new', kind: 'message', role: 'assistant', content: 'Rewritten history', timestamp: null }] }))
    await act(async () => { await vi.advanceTimersByTimeAsync(500) })
    expect(screen.queryByText('Appended reply')).not.toBeInTheDocument()
    expect(screen.getByText('Rewritten history')).toBeVisible()
    vi.mocked(invoke).mockImplementation(async command => { if (command === 'chat_subagent_snapshot') return roster; throw new Error('Cannot read file') })
    await act(async () => { await vi.advanceTimersByTimeAsync(500) })
    expect(screen.getByRole('alert')).toHaveTextContent('Cannot read file')
    unmount()
    const calls = vi.mocked(invoke).mock.calls.length
    await act(async () => { await vi.advanceTimersByTimeAsync(3000) })
    expect(vi.mocked(invoke).mock.calls).toHaveLength(calls)
  })

  it('keeps advisor history read-only', async () => {
    roster = { agents: [agent('advisor', { agentType: 'advisor', status: 'parked', revivable: false })], runningCount: 0 }
    render(<SubagentInspectionDialog conversationId="root" initial={{ agentId: 'advisor' }} onClose={() => {}} />)
    await flush()
    expect(screen.getByText('此代理仅供查看历史')).toBeVisible()
    expect(screen.queryByRole('textbox', { name: '发送给代理的消息' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /恢复|复活/ })).not.toBeInTheDocument()
  })
})
