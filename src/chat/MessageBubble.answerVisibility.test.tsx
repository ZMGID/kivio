import { fireEvent, render, screen } from '@testing-library/react'
import { expect, it } from 'vitest'
import { MessageBubble } from './MessageBubble'
import type { ChatMessage } from './types'

it('keeps live text in normal body typography through subsequent tools', () => {
  const message: ChatMessage = { id: 'reply', role: 'assistant', timestamp: 1, content: '', segments: [
    { id: 'text', kind: 'text', phase: 'tool_loop', order: 0, text: '正在核对结果。' },
    { id: 'tool', kind: 'tool', phase: 'tool_loop', order: 1, tool_call_id: 'todo' },
  ] }
  const { rerender } = render(<MessageBubble message={message} messageStreaming />)
  const body = screen.getByText('正在核对结果。').closest('.chat-markdown')
  expect(body).toHaveClass('text-[15px]')
  fireEvent.click(screen.getByRole('button', { name: /^Working/ }))
  expect(screen.getByText('正在核对结果。')).toBeVisible()
  rerender(<MessageBubble message={{ ...message, stream_outcome: 'completed' }} />)
  expect(screen.getByText('正在核对结果。')).toBeVisible()
})

it.each(['answer', 'plain', 'synthesis'] as const)('preserves %s body before a trailing tool and supplement after reopening', phase => {
  const message: ChatMessage = { id: 'reply', role: 'assistant', timestamp: 1, content: '', segments: [
    { id: 'answer', kind: 'text', phase, order: 0, text: '完整分析结果。' },
    { id: 'tool', kind: 'tool', phase: 'tool_loop', order: 1, tool_call_id: 'todo' },
    { id: 'tail', kind: 'text', phase: 'synthesis', order: 2, text: '全部完成。' },
  ] }
  const { unmount } = render(<MessageBubble message={message} />)
  expect(screen.getByText('完整分析结果。')).toBeVisible()
  expect(screen.getByText('全部完成。')).toBeVisible()
  expect(screen.getByRole('button', { name: /^Worked/ })).toHaveAttribute('aria-expanded', 'false')
  unmount()
  render(<MessageBubble message={message} />)
  expect(screen.getByText('完整分析结果。')).toBeVisible()
})

it('folds progress around explicit delivered answers together with tools after completion and reopening', () => {
  const notes = ['先检查源码。', '正在执行验证。', '测试已经通过，正在整理。']
  const message: ChatMessage = {
    id: 'codex-progress', role: 'assistant', timestamp: 1, content: '',
    tool_calls: [
      { id: 'read', name: 'Read', source: 'external_cli', status: 'success', arguments: '{"path":"README.md"}' },
      { id: 'test', name: 'Bash', source: 'external_cli', status: 'success', arguments: '{"command":"npm test"}' },
    ],
    segments: [
      { id: 'intro', kind: 'text', phase: 'plain', order: 0, text: notes[0] },
      { id: 'read', kind: 'tool', phase: 'tool_loop', order: 1, tool_call_id: 'read' },
      { id: 'partial-answer', kind: 'text', phase: 'answer', order: 2, text: '已确认问题的位置。' },
      { id: 'progress', kind: 'text', phase: 'synthesis', order: 3, text: notes[1] },
      { id: 'test', kind: 'tool', phase: 'tool_loop', order: 4, tool_call_id: 'test' },
      { id: 'last-progress', kind: 'text', phase: 'synthesis', order: 5, text: notes[2] },
    ],
  }
  const { rerender, unmount, container } = render(<MessageBubble message={message} messageStreaming />)
  for (const text of notes) expect(screen.getByText(text)).toBeVisible()
  const completed: ChatMessage = { ...message, stream_outcome: 'completed', segments: [
    ...message.segments!,
    { id: 'final-answer', kind: 'text', phase: 'answer', order: 6, text: '修复已完成。' },
  ] }
  rerender(<MessageBubble message={completed} />)
  const worked = screen.getByRole('button', { name: /^Worked/ })
  expect(worked).toHaveAttribute('aria-expanded', 'false')
  for (const text of notes) expect(screen.queryByText(text)).not.toBeInTheDocument()
  expect(screen.getByText('已确认问题的位置。')).toBeVisible()
  expect(screen.getByText('修复已完成。')).toBeVisible()
  fireEvent.click(worked)
  for (const text of notes) expect(screen.getByText(text)).toBeVisible()
  expect(container.querySelector('[data-tool-call-cluster]')).not.toBeNull()
  fireEvent.click(worked)
  for (const text of notes) expect(screen.queryByText(text)).not.toBeInTheDocument()
  unmount()
  render(<MessageBubble message={completed} />)
  for (const text of notes) expect(screen.queryByText(text)).not.toBeInTheDocument()
  expect(screen.getByText('修复已完成。')).toBeVisible()
})
