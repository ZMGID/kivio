import { act, fireEvent, render, screen, within } from '@testing-library/react'
import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { SubagentSidebar, SubagentTaskList } from './AgentHub'
import { onDockSubagentRequest } from './dock/dockPreview'
import type { SubagentRoster, SubagentTranscriptPage, SubagentView } from '../generated/chatProtocol'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

function agent(id = 'worker', overrides: Partial<SubagentView> = {}): SubagentView {
  return { id, parentId: 'main', generation: 1, displayName: id, agentType: 'scout', status: 'idle', depth: 1,
    lastActivity: 1000, activity: null, model: 'provider/model', progress: null, revivable: true, reviveError: null, ...overrides }
}
const page = (overrides: Partial<SubagentTranscriptPage> = {}): SubagentTranscriptPage => ({ id: 'worker', generation: 1, cursor: { fileId: 'file', offset: 10 }, reset: true,
  entries: [
    { id: 'instruction', kind: 'message', role: 'user', content: 'Complete assignment thoroughly:\n\nCheck the deployment logs.', timestamp: null },
    { id: 'context', kind: 'message', role: 'user', content: '<kivio_runtime_context>\nPrivate runtime configuration\n</kivio_runtime_context>', timestamp: null },
    { id: 'entry', kind: 'message', role: 'assistant', content: 'Retained history', timestamp: null },
    { id: 'line-5-tool-0', kind: 'message', role: 'tool', content: 'read\n{"path":"README.md"}', timestamp: null },
    { id: 'irc', kind: 'message', role: 'irc', content: '<irc>\nSelected peer message\n</irc>', timestamp: null },
    { id: 'other-irc', kind: 'message', role: 'irc', content: 'Unselected peer message', timestamp: null },
    { id: 'run-start', kind: 'notice', role: '', content: '{"type":"turn_start","run":"internal-run-id"}', timestamp: null },
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
      { agentId: 'worker', entryId: 'line-5-tool-0', kind: 'tool', summary: 'read README.md', timestamp: null },
    ]
    if (command === 'chat_subagent_send') return { id: 'receipt', recipient: 'worker', status: 'accepted' }
    return agent()
  })
})
afterEach(() => { vi.useRealTimers() })

describe('subagent tasks', () => {
  it('opens the selected activity in the sidebar and closes the activity dialog', async () => {
    const open = vi.fn()
    const unsubscribe = onDockSubagentRequest(open)
    try {
      render(<SubagentTaskList conversationId="root" roster={roster} />)
      fireEvent.click(screen.getByRole('button', { name: '查看子代理活动' }))
      await flush()
      const dialog = screen.getByRole('dialog', { name: '子代理活动' })
      fireEvent.change(within(dialog).getByRole('textbox', { name: '搜索活动' }), { target: { value: 'readme' } })
      expect(within(dialog).queryByText('Retained history')).not.toBeInTheDocument()
      fireEvent.click(within(dialog).getByRole('button', { name: /read README.md/ }))
      expect(open).toHaveBeenCalledWith({ conversationId: 'root', agentId: 'worker', entryId: 'line-5-tool-0' })
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    } finally {
      unsubscribe()
    }
  })

  it('shows instructions, replies and tool names with no composer or diagnostics', async () => {
    render(<SubagentSidebar target={{ conversationId: 'root', agentId: 'worker', entryId: 'line-5-tool-0' }} />)
    await flush()
    const sidebar = within(screen.getByRole('region', { name: 'worker' }))
    expect(sidebar.getByText('Check the deployment logs.')).toBeVisible()
    expect(sidebar.queryByText(/Complete assignment thoroughly|Private runtime configuration/)).not.toBeInTheDocument()
    expect(sidebar.getByText('Retained history')).toBeVisible()
    expect(sidebar.getByText('read').closest('article')).toHaveFocus()
    expect(sidebar.getByText('provider/model')).toBeVisible()
    expect(sidebar.queryByText(/README.md/)).not.toBeInTheDocument()
    expect(sidebar.queryByText(/peer message|开始执行/)).not.toBeInTheDocument()
    expect(sidebar.queryByRole('textbox')).not.toBeInTheDocument()
    expect(sidebar.queryByRole('button', { name: /诊断|发送|恢复/ })).not.toBeInTheDocument()
  })

  it.each([['irc', /Selected peer message/], ['run-start', /开始执行/]])(
    'focuses the activity %s on demand without adding other hidden records',
    async (entryId, label) => {
      render(<SubagentSidebar target={{ conversationId: 'root', agentId: 'worker', entryId }} />)
      await flush()
      expect(screen.getByText(label).closest('article')).toHaveFocus()
      expect(screen.queryByText('Unselected peer message')).not.toBeInTheDocument()
      expect(screen.queryByText(/internal-run-id/)).not.toBeInTheDocument()
    },
  )

  it('tails append pages and replaces rewritten history without hiding a read failure', async () => {
    const { unmount } = render(<SubagentSidebar target={{ conversationId: 'root', agentId: 'worker' }} />)
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
})
