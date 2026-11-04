import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { MessageList } from './MessageList'
import { mergeSubagentProgress } from './streamApply'
import type { ChatMessage } from './types'

describe('sub-agent result receipts in the parent timeline', () => {
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
    const { rerender } = render(<MessageList conversationId="subagent-history" messages={reloaded} />)
    expect(screen.getByText('汇总结论')).toBeVisible()
    fireEvent.click(screen.getByRole('button', { name: /^Worked/ }))
    fireEvent.click(screen.getByRole('button', { name: /子代理/ }))
    expect(screen.getByText('Inline final result')).toBeVisible()
    expect(screen.getByText('Background failed with evidence')).toBeVisible()
    expect(screen.getByText('已完成')).toBeVisible()
    expect(screen.getByText('失败')).toBeVisible()
    expect(screen.getByText('任务: job-background')).toBeVisible()

    const stale = mergeSubagentProgress(reloaded[0].tool_calls![0], {
      parentConversationId: 'subagent-history', parentRunId: 'old-run', parentToolCallId: 'task-call',
      taskId: 'inline', name: 'Inline', depth: 1, status: 'running', preview: 'stale snapshot',
    })
    rerender(<MessageList conversationId="subagent-history" messages={[{ ...reloaded[0], tool_calls: [stale] }]} />)
    expect(screen.getByText('Inline final result')).toBeVisible()
    expect(screen.getByText('Background failed with evidence')).toBeVisible()
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
