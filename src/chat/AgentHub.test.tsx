import { act, fireEvent, render, screen, within } from '@testing-library/react'
import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { AgentHub, AgentHubEntry } from './AgentHub'
import { IrcObservations } from './IrcMessageCard'
import type { SubagentIrcMessage, SubagentRoster, SubagentTranscriptPage, SubagentView } from '../generated/chatProtocol'

const listeners = vi.hoisted(() => new Set<(message: SubagentIrcMessage) => void>())
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('../api/tauri', () => ({ api: { onChatSubagentIrc: (handler: (message: SubagentIrcMessage) => void) => {
  listeners.add(handler)
  return Promise.resolve(() => { listeners.delete(handler) })
} } }))

function agent(id = 'worker', overrides: Partial<SubagentView> = {}): SubagentView {
  return { id, parentId: 'main', generation: 1, displayName: id, agentType: 'scout', status: 'idle', depth: 1,
    lastActivity: 1000, activity: null, model: 'provider/model', progress: null, revivable: true, reviveError: null, ...overrides }
}
const page = (overrides: Partial<SubagentTranscriptPage> = {}): SubagentTranscriptPage => ({ id: 'worker', generation: 1, cursor: { fileId: 'file', offset: 10 }, reset: true,
  entries: [{ id: 'entry', kind: 'message', role: 'assistant', content: 'Retained history', timestamp: null }], hasMore: false, ...overrides })
function deferred<T>() { let resolve: (value: T) => void = () => {}; const promise = new Promise<T>(done => { resolve = done }); return { promise, resolve } }
const flush = () => act(async () => { await Promise.resolve() })
let roster: SubagentRoster
beforeEach(() => {
  vi.useFakeTimers()
  listeners.clear()
  roster = { agents: [agent()], runningCount: 0 }
  HTMLDialogElement.prototype.showModal = function () { this.setAttribute('open', '') }
  vi.mocked(invoke).mockReset().mockImplementation(async command => {
    if (command === 'chat_subagent_snapshot' || command === 'chat_subagent_restore') return roster
    if (command === 'chat_subagent_transcript') return page()
    if (command === 'chat_subagent_send') return { id: 'receipt', recipient: 'worker', status: 'accepted' }
    return agent()
  })
})
afterEach(() => { vi.useRealTimers() })

describe('Agent Hub lifecycle', () => {
  it('separates running and inspectable agents, opens a modal and returns focus on close', async () => {
    roster = { agents: [agent(), agent('parked', { status: 'parked', lastActivity: 0 })], runningCount: 0 }
    render(<AgentHubEntry conversationId="root" />)
    await flush()
    const entry = screen.getByRole('button', { name: /0 个运行中 \/ 2 个可查看/ })
    entry.focus(); fireEvent.click(entry)
    await flush()
    expect(screen.getByRole('dialog')).toBeVisible()
    expect(screen.getByText('Retained history')).toBeVisible()
    expect(vi.mocked(invoke)).toHaveBeenCalledWith('chat_subagent_transcript', { conversationId: 'root', agentId: 'worker', expectedGeneration: 1, cursor: null })
    fireEvent.click(screen.getByRole('button', { name: '关闭子代理' }))
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    expect(entry).toHaveFocus()
  })

  it('preserves a failed message draft, retries, and sends every action through the expected generation', async () => {
    render(<AgentHub conversationId="root" onClose={() => {}} />)
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
    expect(screen.getByText(/消息已受理/)).toBeVisible()
    expect(invoke).toHaveBeenCalledWith('chat_subagent_send', { conversationId: 'root', agentId: 'worker', expectedGeneration: 1, content: 'Continue the investigation' })
    for (const [label, command] of [['打开会话', 'chat_subagent_open'], ['释放会话', 'chat_subagent_park'], ['终止', 'chat_subagent_kill']]) {
      fireEvent.click(screen.getByRole('button', { name: label }))
      await flush()
      expect(invoke).toHaveBeenCalledWith(command, { conversationId: 'root', agentId: 'worker', expectedGeneration: 1 })
    }
    fireEvent.click(screen.getByRole('button', { name: '恢复中断运行' }))
    await flush()
    expect(invoke).toHaveBeenCalledWith('chat_subagent_restore', { conversationId: 'root' })
  })

  it('keeps rank stable across heartbeats and preserves the selected identity when its generation changes', async () => {
    roster = { agents: [agent('older', { lastActivity: 1 }), agent('worker', { lastActivity: 2 })], runningCount: 0 }
    render(<AgentHub conversationId="root" onClose={() => {}} />)
    await flush()
    const list = screen.getByRole('navigation', { name: '代理列表' })
    expect(within(list).getAllByRole('button').map(button => button.textContent?.split('空闲')[0])).toEqual(['worker', 'older'])
    fireEvent.click(within(list).getByRole('button', { name: /older/ }))
    roster = { agents: [agent('older', { lastActivity: 9999, generation: 2 }), agent('worker', { lastActivity: 3 })], runningCount: 0 }
    await act(async () => { await vi.advanceTimersByTimeAsync(500) })
    expect(within(list).getAllByRole('button')[0]).toHaveTextContent('worker')
    expect(within(screen.getByRole('region', { name: '代理详情' })).getByRole('heading')).toHaveTextContent('older')
    fireEvent.click(screen.getByRole('button', { name: '终止' }))
    await flush()
    expect(invoke).toHaveBeenCalledWith('chat_subagent_kill', { conversationId: 'root', agentId: 'older', expectedGeneration: 2 })
  })

  it('tails append pages, replaces rewritten history, and keeps errors recoverable', async () => {
    const { unmount } = render(<AgentHub conversationId="root" onClose={() => {}} />)
    await flush()
    vi.mocked(invoke).mockImplementation(async command => command === 'chat_subagent_snapshot' ? roster : page({ reset: false, cursor: { fileId: 'file', offset: 20 }, entries: [{ id: 'second', kind: 'compaction', role: '', content: 'Compacted history', timestamp: null }] }))
    await act(async () => { await vi.advanceTimersByTimeAsync(500) })
    expect(screen.getByText('Retained history')).toBeVisible()
    expect(screen.getByText('Compacted history')).toBeVisible()
    expect(invoke).toHaveBeenCalledWith('chat_subagent_transcript', { conversationId: 'root', agentId: 'worker', expectedGeneration: 1, cursor: { fileId: 'file', offset: 10 } })
    vi.mocked(invoke).mockImplementation(async command => command === 'chat_subagent_snapshot' ? roster : page({ entries: [{ id: 'new', kind: 'message', role: 'assistant', content: 'Rewritten history', timestamp: null }] }))
    await act(async () => { await vi.advanceTimersByTimeAsync(500) })
    expect(screen.queryByText('Retained history')).not.toBeInTheDocument()
    expect(screen.getByText('Rewritten history')).toBeVisible()
    vi.mocked(invoke).mockImplementation(async command => { if (command === 'chat_subagent_snapshot') return roster; throw new Error('Cannot read file') })
    await act(async () => { await vi.advanceTimersByTimeAsync(500) })
    expect(screen.getByRole('alert')).toHaveTextContent('Cannot read file')
    vi.mocked(invoke).mockResolvedValue(page())
    fireEvent.click(screen.getByRole('button', { name: '重试读取' }))
    await flush()
    expect(screen.getByText('Retained history')).toBeVisible()
    unmount()
    const calls = vi.mocked(invoke).mock.calls.length
    await act(async () => { await vi.advanceTimersByTimeAsync(1500) })
    expect(vi.mocked(invoke).mock.calls).toHaveLength(calls)
  })

  it('ignores old root snapshots and late transcript completions after navigation', async () => {
    const pending = deferred<SubagentTranscriptPage>()
    vi.mocked(invoke).mockImplementation(async command => command === 'chat_subagent_snapshot' ? roster : pending.promise)
    const view = render(<AgentHub conversationId="root" onClose={() => {}} />)
    await flush()
    roster = { agents: [agent('new', { displayName: 'New root agent' })], runningCount: 0 }
    vi.mocked(invoke).mockImplementation(async command => command === 'chat_subagent_snapshot' ? roster : page({ id: 'new', entries: [{ id: 'new-entry', kind: 'message', role: 'assistant', content: 'New root history', timestamp: null }] }))
    view.rerender(<AgentHub conversationId="next-root" onClose={() => {}} />)
    await flush()
    await act(async () => { pending.resolve(page({ entries: [{ id: 'late', kind: 'message', role: 'assistant', content: 'Old root secret', timestamp: null }] })); await pending.promise })
    expect(screen.getByText('New root history')).toBeVisible()
    expect(screen.queryByText('Old root secret')).not.toBeInTheDocument()
    expect(screen.queryByText('worker')).not.toBeInTheDocument()
  })

  it('does not send from a read-only advisor and preserves a failed revive for retry', async () => {
    roster = { agents: [agent('advisor', { agentType: 'advisor', status: 'parked', revivable: false })], runningCount: 0 }
    render(<AgentHub conversationId="root" onClose={() => {}} />)
    await flush()
    expect(screen.queryByRole('textbox', { name: '发送给代理的消息' })).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: '复活' })).toBeDisabled()
    roster = { agents: [agent('advisor', { status: 'parked' })], runningCount: 0 }
    await act(async () => { await vi.advanceTimersByTimeAsync(500) })
    vi.mocked(invoke).mockRejectedValueOnce(new Error('Contract missing'))
    fireEvent.click(screen.getByRole('button', { name: '复活' }))
    await flush()
    expect(screen.getByRole('alert')).toHaveTextContent('Contract missing')
    expect(screen.getByRole('button', { name: '复活' })).toBeEnabled()
  })
})

it('shows Main-only relay observations without creating user messages and clears them on root change', async () => {
  const view = render(<IrcObservations conversationId="root" />)
  await flush()
  const observation: SubagentIrcMessage = { id: 'irc', conversationId: 'root', fromAgent: 'alpha', toAgent: 'beta', content: 'Peer update', replyTo: null, progress: false, broadcast: false, observation: true, timestamp: 1 }
  act(() => { for (const listener of listeners) { listener(observation); listener(observation); listener({ ...observation, id: 'incoming', content: 'Durable incoming', observation: false }); listener({ ...observation, id: 'other', conversationId: 'other', content: 'Other root' }) } })
  expect(screen.getAllByText('Peer update')).toHaveLength(1)
  expect(screen.getByText('旁观')).toBeVisible()
  expect(screen.queryByText('Durable incoming')).not.toBeInTheDocument()
  expect(screen.queryByText('Other root')).not.toBeInTheDocument()
  view.rerender(<IrcObservations conversationId="new-root" />)
  expect(screen.queryByText('Peer update')).not.toBeInTheDocument()
})
