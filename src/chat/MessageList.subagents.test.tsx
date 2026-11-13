import { fireEvent, render, screen, within } from '@testing-library/react'
import { beforeEach, describe, expect, it } from 'vitest'
import { MessageList } from './MessageList'
import { mergeSubagentProgress } from './streamApply'
import type { ChatMessage } from './types'

beforeEach(() => {
  HTMLDialogElement.prototype.showModal ??= function () { this.setAttribute('open', '') }
  HTMLDialogElement.prototype.close ??= function () { this.removeAttribute('open') }
})

describe('sub-agent result receipts in the parent timeline', () => {
  it('keeps saved IRC bodies out of the chat timeline', () => {
    render(<MessageList conversationId="root" messages={[{ id: 'irc-record', role: 'assistant', content: 'Raw message', timestamp: 1,
      irc_message: { id: 'irc', conversationId: 'root', fromAgent: 'reviewer', toAgent: 'main', content: 'Peer evidence', replyTo: null, progress: false, broadcast: false, observation: false, timestamp: 1 },
    }]} />)
    expect(screen.queryByText('reviewer → main')).not.toBeInTheDocument()
    expect(screen.queryByText('Peer evidence')).not.toBeInTheDocument()
    expect(screen.queryByText('Raw message')).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /重新生成|复制/ })).not.toBeInTheDocument()
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
  })

  it('keeps outgoing IRC tool arguments out of Worked while showing ordinary writes', () => {
    render(<MessageList conversationId="root" messages={[{
      id: 'answer', role: 'assistant', content: 'Done', timestamp: 1, stream_outcome: 'completed',
      tool_calls: [
        { id: 'irc-write', name: 'write', source: 'native', status: 'success',
          arguments: { path: 'agent://Scout', content: 'Peer evidence' }, argument_preview: 'agent://Scout' },
        { id: 'file-write', name: 'write', source: 'native', status: 'success',
          arguments: { path: 'report.txt', content: 'Report' }, argument_preview: 'report.txt' },
      ],
    }]} />)
    const worked = screen.getByRole('button', { name: /^Worked/ })
    expect(worked).toHaveTextContent('Worked+1-0')
    fireEvent.click(worked)
    expect(screen.queryByText(/agent:\/\/Scout|Peer evidence/)).not.toBeInTheDocument()
  })
  it('keeps persisted worker reports out of the timeline while showing the final answer', () => {
    const messages: ChatMessage[] = [
      { id: 'question', role: 'user', content: '调查这个项目', timestamp: 1 },
      { id: 'subagent-result-execution-a', role: 'assistant', content: '[Sub-agent: A · Completed]\nRaw worker report', timestamp: 2 },
      { id: 'answer', role: 'assistant', content: '主代理整理后的结论', timestamp: 3 },
    ]
    render(<MessageList conversationId="subagent-receipt" messages={messages} />)
    expect(screen.getByText('主代理整理后的结论')).toBeInTheDocument()
    expect(screen.queryByText(/Raw worker report/)).not.toBeInTheDocument()
    expect(messages).toHaveLength(3)
  })

  it('shows each terminal task result in reopened conversation history despite a late live event', () => {
    const saved: ChatMessage[] = [{
      id: 'answer', role: 'assistant', content: '汇总结论', timestamp: 2, stream_outcome: 'completed',
      segments: [
        { id: 'task-segment', kind: 'tool', phase: 'tool_loop', order: 1, tool_call_id: 'task-call' },
        { id: 'answer-segment', kind: 'text', phase: 'plain', order: 2, text: '汇总结论' },
      ],
      tool_calls: [{
        id: 'task-call', name: 'task', source: 'native', status: 'success',
        arguments: { tasks: [{ task: 'Inline' }, { task: 'Background' }] },
        structured_content: {
          agents: [{ agentId: 'inline', name: 'Inline' }, { agentId: 'background', name: 'Background', jobId: 'job-background' }],
          subagentProgressByTask: {
            inline: { status: 'completed', preview: 'Inline final result' },
            background: { status: 'failed', preview: 'Background failed with evidence' },
          },
        },
      }],
    }]
    const reloaded = JSON.parse(JSON.stringify(saved)) as ChatMessage[]
    const { container, rerender } = render(<MessageList conversationId="subagent-history" messages={reloaded} />)
    expect(screen.getByText('汇总结论')).toBeVisible()
    expect(screen.queryByRole('button', { name: 'Inline 已完成' })).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: /^Worked/ }))
    const completed = screen.getByRole('button', { name: 'Inline 已完成' })
    expect(completed).not.toHaveAttribute('aria-haspopup')
    expect(screen.getByRole('button', { name: 'Background 失败' })).toBeVisible()
    expect(screen.queryByRole('button', { name: /^子代理 / })).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '查看派工回执' }))
    const dialog = screen.getByRole('dialog', { name: '派工回执' })
    expect(container.contains(dialog)).toBe(false)
    expect(within(dialog).getByText('Inline final result')).toBeVisible()
    expect(within(dialog).getByText('Background failed with evidence')).toBeVisible()
    expect(within(dialog).getByText('已完成')).toBeVisible()
    expect(within(dialog).getByText('失败')).toBeVisible()
    expect(within(dialog).getByText('任务: job-background')).toBeVisible()

    const stale = mergeSubagentProgress(reloaded[0].tool_calls![0], {
      parentConversationId: 'subagent-history', parentRunId: 'old-run', parentToolCallId: 'task-call',
      taskId: 'inline', name: 'Inline', depth: 1, status: 'running', preview: 'stale snapshot',
    })
    rerender(<MessageList conversationId="subagent-history" messages={[{ ...reloaded[0], tool_calls: [stale] }]} />)
    expect(within(dialog).getByText('Inline final result')).toBeVisible()
    expect(within(dialog).getByText('Background failed with evidence')).toBeVisible()
    expect(screen.queryByText('stale snapshot')).toBeNull()
  })

  it('keeps a late completion under the dispatch reply’s Worked control after reopening', () => {
    const lifecycle = { agentId: 'delay', name: 'DelayedProbe', parentToolCallId: 'task',
      afterSegmentId: null, textChars: null }
    const messages: ChatMessage[] = [
      { id: 'user', role: 'user', timestamp: 1, content: 'Delegate the check' },
      { id: 'dispatch', role: 'assistant', timestamp: 2, content: 'I handed off the check.', stream_outcome: 'completed',
        tool_calls: [{ id: 'task', name: 'task', source: 'native', status: 'success' }],
        segments: [
          { id: 'task', kind: 'tool', phase: 'tool_loop', order: 1001, tool_call_id: 'task' },
          { id: 'handoff', kind: 'text', phase: 'plain', order: 1002, text: 'I handed off the check.' },
          { id: 'start', kind: { subagent: { ...lifecycle, afterOrder: null, status: 'started' } }, phase: 'auxiliary', order: 3 },
          { id: 'finish', kind: { subagent: { ...lifecycle, afterOrder: 0xffff_ffff, status: 'completed' } }, phase: 'auxiliary', order: 0 },
        ],
      },
      { id: 'subagent-result-delay', role: 'assistant', timestamp: 3, content: 'Raw worker output' },
      { id: 'answer', role: 'assistant', timestamp: 4, content: 'Here is the result.' },
    ]
    render(<MessageList conversationId="subagent-late-finish" messages={messages} />)
    const worked = screen.getByRole('button', { name: /^Worked/ })
    const answer = screen.getByText('Here is the result.')
    expect(screen.queryByRole('button', { name: /^DelayedProbe/ })).not.toBeInTheDocument()
    expect(screen.queryByText('I handed off the check.')).not.toBeInTheDocument()
    expect(answer).toBeVisible()
    fireEvent.click(worked)
    const ordered = [
      worked, screen.getByRole('button', { name: 'DelayedProbe 开始工作' }), screen.getByText('I handed off the check.'),
      screen.getByRole('button', { name: 'DelayedProbe 已完成' }), answer,
    ]
    for (let index = 1; index < ordered.length; index += 1) {
      expect(ordered[index - 1].compareDocumentPosition(ordered[index]) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    }
  })

  it('folds all automatic continuations of one user request and leaves the next request independent', () => {
    const event = { agentId: 'scout', name: 'Scout', parentToolCallId: 'task', afterSegmentId: null,
      afterOrder: null, textChars: null }
    const messages: ChatMessage[] = [
      { id: 'user-one', role: 'user', content: 'Run the audit', timestamp: 1 },
      { id: 'dispatch', role: 'assistant', content: 'Audit handed off.', timestamp: 2, stream_outcome: 'completed',
        segments: [{ id: 'start', kind: { subagent: { ...event, status: 'started' } }, phase: 'auxiliary', order: 1 },
          { id: 'handoff', kind: 'text', phase: 'plain', order: 2, text: 'Audit handed off.' }] },
      { id: 'subagent-result-execution-scout', role: 'assistant', content: 'Raw audit result', timestamp: 3 },
      { id: 'follow-up', role: 'assistant', content: 'One check came back.', timestamp: 4, stream_outcome: 'completed',
        segments: [{ id: 'finish', kind: { subagent: { ...event, status: 'completed' } }, phase: 'auxiliary', order: 1 },
          { id: 'update', kind: 'text', phase: 'plain', order: 2, text: 'One check came back.' }] },
      { id: 'final', role: 'assistant', content: 'Final audit result.', reasoning: 'Reconcile the checks.', timestamp: 5, stream_outcome: 'completed' },
      { id: 'user-two', role: 'user', content: 'Run another check', timestamp: 6 },
      { id: 'next', role: 'assistant', content: 'Second result.', reasoning: 'Independent reasoning.', timestamp: 7, stream_outcome: 'completed' },
    ]
    const { rerender } = render(<MessageList conversationId="subagent-whole-turn"
      messages={messages.slice(0, 2)} waitingForBackgroundTasks />)
    expect(screen.getByRole('button', { name: 'Working' })).toHaveAttribute('aria-expanded', 'true')
    expect(screen.getByText('Audit handed off.')).toBeVisible()
    rerender(<MessageList conversationId="subagent-whole-turn" messages={messages} />)
    const [first, second] = screen.getAllByRole('button', { name: /^Worked/ })
    expect(screen.getAllByRole('button', { name: /^Worked/ })).toHaveLength(2)
    expect(screen.getByText('Final audit result.')).toBeVisible()
    expect(screen.getByText('Second result.')).toBeVisible()
    expect(screen.queryByText('Audit handed off.')).not.toBeInTheDocument()
    expect(screen.queryByText('One check came back.')).not.toBeInTheDocument()
    fireEvent.click(first)
    expect(screen.getByRole('button', { name: 'Scout 开始工作' })).toBeVisible()
    expect(screen.getByRole('button', { name: 'Scout 已完成' })).toBeVisible()
    expect(screen.getByText('Audit handed off.')).toBeVisible()
    expect(screen.getByText('One check came back.')).toBeVisible()
    expect(second).toHaveAttribute('aria-expanded', 'false')
    fireEvent.click(first)
    expect(screen.queryByRole('button', { name: 'Scout 已完成' })).not.toBeInTheDocument()
    expect(screen.queryByText('One check came back.')).not.toBeInTheDocument()
    expect(screen.getByText('Final audit result.')).toBeVisible()
  })

  it('keeps an explicit answer visible across automatic continuations', () => {
    render(<MessageList conversationId="answer-continuation" messages={[
      { id: 'user', role: 'user', content: 'Inspect', timestamp: 1 },
      { id: 'answer', role: 'assistant', content: '', timestamp: 2, segments: [
        { id: 'body', kind: 'text', phase: 'answer', order: 1, text: 'Complete findings' },
      ] },
      { id: 'subagent-result-worker', role: 'assistant', content: 'Internal receipt', timestamp: 3 },
      { id: 'supplement', role: 'assistant', content: 'Follow-up note', timestamp: 4 },
    ]} />)
    expect(screen.getByText('Complete findings')).toBeVisible()
    expect(screen.getByText('Follow-up note')).toBeVisible()
    expect(screen.queryByText('Internal receipt')).not.toBeInTheDocument()
  })

  it('does not hide user or assistant text just because it quotes a worker header', () => {
    render(<MessageList conversationId="subagent-quote" messages={[
      { id: 'question', role: 'user', content: '[Sub-agent: A · Completed] 是什么？', timestamp: 1 },
      { id: 'answer', role: 'assistant', content: '[Sub-agent: A · Completed] 是内部回执标题。', timestamp: 2 },
    ]} />)
    expect(screen.getByText(/是什么？/)).toBeInTheDocument()
    expect(screen.getByText(/是内部回执标题/)).toBeInTheDocument()
  })
})
