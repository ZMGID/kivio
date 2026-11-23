import { act, render, screen } from '@testing-library/react'
import { afterEach, expect, it } from 'vitest'
import { MessageList } from './MessageList'
import { reset, setCoarse } from './streamingStore'

afterEach(() => act(() => {
  reset()
  setCoarse({ streamError: '' })
}))

it.each([
  ['Goal mode does not support Orchestrate in this version', 'unknown', '调用失败'],
  ['Failed to generate response', 'unknown', '调用失败'],
  ['context deadline exceeded', 'timeout', '超时 / 连接中断'],
  ['DEADLINE_EXCEEDED', 'timeout', '超时 / 连接中断'],
  ['request timed out', 'timeout', '超时 / 连接中断'],
  ['maximum context length exceeded', 'context_overflow', '上下文超长'],
  ['context_length_exceeded', 'context_overflow', '上下文超长'],
  ['Rate limit exceeded', 'rate_limited', '限流 / 配额'],
  ['rate_limit_exceeded', 'rate_limited', '限流 / 配额'],
  ['rate-limited', 'rate_limited', '限流 / 配额'],
  ['HTTP 429: Too Many Requests', 'rate_limited', '限流 / 配额'],
  ['quota exceeded', 'rate_limited', '限流 / 配额'],
  ['请求被限流', 'rate_limited', '限流 / 配额'],
])('shows the correct error category for %s', (error, kind, label) => {
  act(() => setCoarse({ streamError: error }))
  render(<MessageList conversationId="error-category" messages={[]} />)
  expect(screen.getByRole('status')).toHaveAttribute('data-degraded-kind', kind)
  expect(screen.getByText(label)).toBeVisible()
  expect(screen.getByText(error)).toBeVisible()
})

const failedReply = {
  id: 'failed', role: 'assistant' as const, content: '', timestamp: 2,
  degraded: { kind: 'rate_limited', reason: '模型服务限流', detail: '429 Too Many Requests', text: 'raw 429', totalToolCalls: null },
}

it('shows a single current error even when the conversation also records a failure', () => {
  act(() => setCoarse({ streamError: 'HTTP 429: Too Many Requests' }))
  render(<MessageList conversationId="failed-once" messages={[failedReply]} />)
  expect(screen.getAllByRole('status')).toHaveLength(1)
  expect(screen.getByText('HTTP 429: Too Many Requests')).toBeVisible()
})

it('keeps the partial answer and removes its failure card on the next send', () => {
  const partial = { ...failedReply, content: '已经生成的回答', segments: [
    { id: 'partial', kind: 'text' as const, phase: 'partial_answer' as const, order: 0, text: '已经生成的回答' },
    { id: 'error-copy', kind: 'text' as const, phase: 'plain' as const, order: 1, text: failedReply.degraded.text },
  ] }
  const next = { id: 'u2', role: 'user' as const, content: '继续', timestamp: 3 }
  const { rerender } = render(<MessageList conversationId="partial-continue" messages={[partial]} />)
  expect(screen.getByRole('status')).toHaveTextContent(partial.degraded.reason)
  rerender(<MessageList conversationId="partial-continue" messages={[partial, next]} />)
  expect(screen.getByText(partial.content)).toBeVisible()
  expect(screen.queryByRole('status')).toBeNull()
  expect(screen.queryByText(failedReply.degraded.text)).toBeNull()
})

it('shows queued input immediately in the transcript and reconciles a delivered record once', () => {
  const pending = [{ id: 'q1', content: '就是 codex 占用什么的', attachments: [], steering: false }]
  const { container, rerender } = render(<MessageList conversationId="steering" messages={[]} pendingInjections={pending} />)
  const text = screen.getByText(pending[0].content)
  expect(text).toBeVisible()
  expect(text.closest('.chat-scroll-viewport')).not.toBeNull()
  expect(text.closest('.chat-user-bubble')).not.toBeNull()
  expect(container.querySelector('.animate-spin')).toBeNull()
  const delivered = [{
    id: 'assistant', role: 'assistant' as const, content: '收到', timestamp: 1,
    tool_calls: [{ id: 'steer-q1', name: 'user_steer', source: 'native' as const, status: 'completed' as const,
      structured_content: { type: 'user_steer', steer_id: 'q1', text: pending[0].content } }],
    segments: [
      { id: 's1', kind: 'tool' as const, phase: 'tool_loop' as const, order: 0, tool_call_id: 'steer-q1' },
      { id: 's2', kind: 'text' as const, phase: 'synthesis' as const, order: 1, text: '收到' },
    ],
  }]
  // The display event may arrive before the queue confirmation.
  rerender(<MessageList conversationId="steering" messages={delivered} pendingInjections={pending} />)
  expect(screen.getAllByText(pending[0].content)).toHaveLength(1)
  rerender(<MessageList conversationId="steering" messages={delivered} pendingInjections={[]} />)
  expect(screen.getAllByText(pending[0].content)).toHaveLength(1)
})
