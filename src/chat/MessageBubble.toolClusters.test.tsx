import { fireEvent, render, screen, within } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { MessageBubble } from './MessageBubble'
import type { ChatMessage, ChatMessageSegment, ToolCallRecord } from './types'

const read: ToolCallRecord = { id: 'read', name: 'read', status: 'completed', arguments: { path: 'first.ts' }, result_preview: 'First contents' }
const command: ToolCallRecord = { id: 'run', name: 'bash', status: 'running', arguments: { command: 'npm test' }, result_preview: 'Tests running' }

function message(tools: ToolCallRecord[], segments?: ChatMessageSegment[]): ChatMessage {
  return {
    id: 'cluster-message', role: 'assistant', timestamp: 1, content: '', tool_calls: tools,
    segments: segments ?? tools.map((tool, order) => ({
      id: `seg-${tool.id}`, kind: 'tool', phase: 'tool_loop', order, tool_call_id: tool.id,
    })),
  }
}

function activityButton(container: HTMLElement, index = 0): HTMLButtonElement {
  return container.querySelectorAll<HTMLButtonElement>('[data-tool-call-cluster] > button')[index]
}

describe('tool activity in the message timeline', () => {
  it('keeps the active call in view and preserves inspected output through completion', () => {
    const pending = message([command, read])
    const { container, rerender } = render(<MessageBubble message={pending} messageStreaming />)
    const row = activityButton(container)
    expect(row).toHaveTextContent('npm test')
    expect(screen.queryByText('First contents')).not.toBeInTheDocument()
    fireEvent.click(row)
    fireEvent.click(screen.getByRole('button', { name: 'Run npm test' }))
    expect(screen.getByText('Tests running')).toBeVisible()
    rerender(<MessageBubble message={{ ...pending, content: 'Done', stream_outcome: 'completed',
      tool_calls: [{ ...command, status: 'completed', result_preview: 'Tests passed' }, read],
      segments: [...pending.segments!, { id: 'final', kind: 'text', phase: 'synthesis', order: 2, text: 'Done' }],
    }} />)
    expect(activityButton(container)).toBe(row)
    expect(row).toHaveAttribute('aria-expanded', 'true')
    expect(screen.getByText('Tests passed')).toBeVisible()
    expect(screen.getByRole('button', { name: /^Worked/ })).toHaveAttribute('aria-expanded', 'true')
    expect(screen.getByText('Done').closest('[aria-label="过程分组"]')).toBeNull()
  })

  it('hides failures but keeps stopped counts after the finished turn folds', () => {
    const pending = message([{ ...read, status: 'error', error: 'Permission denied' }, command])
    const { rerender } = render(<MessageBubble message={pending} messageStreaming />)
    rerender(<MessageBubble message={{ ...pending, content: 'Unable to finish', stream_outcome: 'completed',
      tool_calls: [pending.tool_calls![0], { ...command, status: 'cancelled' }],
      segments: [...pending.segments!, { id: 'final', kind: 'text', phase: 'synthesis', order: 2, text: 'Unable to finish' }],
    }} />)
    const work = screen.getByRole('button', { name: /^Worked/ })
    expect(work).toHaveAttribute('aria-expanded', 'false')
    expect(within(work).queryByText(/失败/)).toBeNull()
    expect(within(work).getByText('1 项已停止或跳过')).toBeVisible()
    expect(screen.getByText('Unable to finish')).toBeVisible()
  })

  it('summarizes the complete large cluster without crossing a commentary boundary', () => {
    const reads = Array.from({ length: 128 }, (_, index): ToolCallRecord => ({
      id: `read-${index}`, name: 'read', status: index === 0 ? 'error' : index === 1 ? 'cancelled' : 'completed',
      arguments: { path: `file-${index % 8}.ts` },
    }))
    const last: ToolCallRecord = { id: 'last', name: 'bash', status: 'completed', arguments: { command: 'echo done' } }
    const segments = message([...reads, last]).segments!
    segments.splice(128, 0, { id: 'boundary', kind: 'text', phase: 'tool_loop', order: 127.5, text: 'Moving on' })
    const { container } = render(<MessageBubble message={message([...reads, last], segments)} />)
    fireEvent.click(screen.getByRole('button', { name: /^Worked/ }))
    const first = activityButton(container)
    const second = activityButton(container, 1)
    expect(first).toHaveTextContent('读取 8 个文件，1 项已停止')
    expect(first).not.toHaveTextContent('失败')
    expect(second).toHaveTextContent('执行 1 条命令')
    expect(second).not.toHaveTextContent('失败')
    expect(second).not.toHaveTextContent('已停止')
  })

  it('keeps unanswered questions and permissions visible when Work is closed', () => {
    const question: ToolCallRecord = { id: 'ask', name: 'AskUserQuestion', status: 'running', structured_content: {
      askUser: { phase: 'awaiting', questions: [{ id: 'q', prompt: '继续吗？', options: [{ id: 'yes', label: '继续' }, { id: 'no', label: '停止' }] }] },
    } }
    const permission: ToolCallRecord = { id: 'permit', name: 'deploy', status: 'pending', requires_confirmation: true }
    render(<MessageBubble message={message([read, question, permission, command])} messageStreaming />)
    fireEvent.click(screen.getByRole('button', { name: /^Working/ }))
    expect(screen.getByText('等待你回答（在下方面板里选）')).toBeVisible()
    expect(screen.getByText('deploy')).toBeVisible()
    expect(screen.getByText('deploy').closest('[data-tool-call-cluster]')).toBeNull()
  })

  it('hides built-in dispatch placeholders and wait while preserving external tools', () => {
    const task: ToolCallRecord = { id: 'task', source: 'native', name: 'task', status: 'success',
      arguments: { context: 'Shared', tasks: [{ name: 'Research', task: 'Inspect', solutionSpace: 'Read' }] } }
    const wait: ToolCallRecord = { id: 'wait', source: 'native', name: 'wait', status: 'running', arguments: {} }
    const external: ToolCallRecord = { id: 'external', source: 'external_cli', name: 'Task', status: 'running', arguments: { description: 'CLI research' } }
    const externalMessage: ToolCallRecord = { id: 'external-message', source: 'external_cli', name: 'SendMessage', status: 'completed' }
    const mcpTask: ToolCallRecord = { id: 'mcp', source: 'mcp', name: 'task', status: 'completed' }
    const { container } = render(<MessageBubble message={message([read, task, wait, external, externalMessage, mcpTask, command])} messageStreaming />)
    expect(screen.queryByRole('button', { name: 'Research 已派工' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /等待子代理/ })).not.toBeInTheDocument()
    expect(screen.getByText('SUBAGENT').closest('[data-tool-call-cluster]')).toBeNull()
    expect(screen.getByText('SendMessage').closest('[data-tool-call-cluster]')).toBeNull()
    expect(container.querySelectorAll('[data-tool-call-cluster]')).toHaveLength(2)
  })

  it('keeps only the answer in a history entry containing built-in waits', () => {
    const receipt: ToolCallRecord = { id: 'poll', source: 'native', name: 'native__wait', status: 'success', result_preview: 'Poll result' }
    const legacy: ToolCallRecord = { ...receipt, id: 'unlinked', name: 'wait', status: 'error', error: 'Poll error' }
    const history = message([receipt])
    const { rerender } = render(<MessageBubble message={{ ...history, tool_calls: [receipt, legacy], content: 'Final answer' }} />)
    expect(screen.getByText('Final answer')).toBeVisible()
    expect(screen.queryByRole('button', { name: /^Work/ })).not.toBeInTheDocument()
    expect(screen.queryByText(/Poll (result|error)/)).not.toBeInTheDocument()
    const external: ToolCallRecord = { ...receipt, source: 'mcp', name: 'wait', status: 'running' }
    rerender(<MessageBubble message={message([external])} messageStreaming />)
    expect(screen.getByRole('button', { name: /wait/i })).toBeVisible()
  })

  it('keeps a single activity anchor as thinking and tools append', () => {
    const thought: ChatMessageSegment = { id: 'thought', kind: 'reasoning', phase: 'tool_loop', order: 0, text: 'Plan\nInspecting source' }
    const initial = message([], [thought])
    const { container, rerender } = render(<MessageBubble message={initial} messageStreaming reasoningStreaming />)
    const row = activityButton(container)
    fireEvent.click(row)
    const fullThought = screen.getByTestId('reasoning-text')
    const next = message([command], [thought, { id: 'command', kind: 'tool', phase: 'tool_loop', order: 1, tool_call_id: command.id }])
    rerender(<MessageBubble message={next} messageStreaming />)
    expect(activityButton(container)).toBe(row)
    expect(screen.getByTestId('reasoning-text')).toBe(fullThought)
    expect(fullThought).toBeVisible()
    expect(screen.getByRole('button', { name: 'Run npm test' })).toBeVisible()
  })
})
