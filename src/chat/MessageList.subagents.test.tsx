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
  it('shows the saved agent message directly below its sender and recipient', () => {
    render(<MessageList conversationId="root" messages={[{ id: 'irc-record', role: 'assistant', content: 'Raw message', timestamp: 1,
      irc_message: { id: 'irc', conversationId: 'root', fromAgent: 'reviewer', toAgent: 'main', content: 'Peer evidence', replyTo: null, progress: false, broadcast: false, observation: false, timestamp: 1 },
    }]} />)
    const message = within(screen.getByRole('article', { name: '代理消息 · reviewer → main' }))
    expect(message.getByText('reviewer → main')).toBeVisible()
    expect(message.getByText('Peer evidence')).toBeVisible()
    expect(message.queryByRole('button')).not.toBeInTheDocument()
    expect(screen.queryByText('Raw message')).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /重新生成|复制/ })).not.toBeInTheDocument()
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
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
    expect(screen.queryByRole('button', { name: /^Worked/ })).not.toBeInTheDocument()
    const completed = screen.getByRole('button', { name: 'Inline 已完成' })
    expect(completed).toHaveAttribute('aria-haspopup', 'dialog')
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

  it('does not hide user or assistant text just because it quotes a worker header', () => {
    render(<MessageList conversationId="subagent-quote" messages={[
      { id: 'question', role: 'user', content: '[Sub-agent: A · Completed] 是什么？', timestamp: 1 },
      { id: 'answer', role: 'assistant', content: '[Sub-agent: A · Completed] 是内部回执标题。', timestamp: 2 },
    ]} />)
    expect(screen.getByText(/是什么？/)).toBeInTheDocument()
    expect(screen.getByText(/是内部回执标题/)).toBeInTheDocument()
  })
})
