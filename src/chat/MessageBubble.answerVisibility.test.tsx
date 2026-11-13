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
