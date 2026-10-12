import { act, render, screen } from '@testing-library/react'
import { afterEach, expect, it } from 'vitest'
import { MessageList } from './MessageList'
import { reset, setCoarse } from './streamingStore'

afterEach(() => act(() => reset()))

it.each([
  ['Goal mode does not support Orchestrate in this version', 'unknown', '调用失败'],
  ['Failed to generate response', 'unknown', '调用失败'],
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
