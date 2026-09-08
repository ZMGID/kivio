import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { expect, it, vi } from 'vitest'
import { ChatMarkdown } from './ChatMarkdown'
import { MessageList } from './MessageList'

it.each(['> ## Quoted heading', '- ## List heading', '#'])('keeps root anchors aligned after excluded Markdown: %s', async (prefix) => {
  const { container } = render(<ChatMarkdown
    content={`${prefix}\n\n# Actual first\n\n## Actual second`}
    outlineSource={{ ownerMessageId: 'answer', sourceId: 'answer', onChange: vi.fn() }} />)
  await waitFor(() => expect(container.querySelector('[id="user-content-chat-heading-answer-0"]')).toHaveTextContent('Actual first'))
  expect(container.querySelector('[id="user-content-chat-heading-answer-1"]')).toHaveTextContent('Actual second')
})

it('updates the current heading on scroll in a single-turn conversation', async () => {
  const { container } = render(<MessageList conversationId="single-turn-review" messages={[
    { id: 'user', role: 'user', content: 'Question', timestamp: 1 },
    { id: 'answer', role: 'assistant', content: '# First\n\nBody\n\n## Second\n\nBody', timestamp: 2 },
  ]} />)
  await act(async () => { await Promise.resolve() })
  const viewport = container.querySelector<HTMLElement>('.chat-scroll-viewport')!
  const answer = container.querySelector<HTMLElement>('[data-chat-outline-owner="answer"]')!
  let offset = 0
  Object.defineProperty(viewport, 'clientHeight', { configurable: true, value: 800 })
  Object.defineProperty(viewport, 'scrollTop', { configurable: true, writable: true, value: 0 })
  Object.defineProperty(viewport, 'scrollHeight', { configurable: true, value: 2000 })
  viewport.getBoundingClientRect = () => ({ top: 0, bottom: 800, height: 800 } as DOMRect)
  answer.getBoundingClientRect = () => ({ top: -offset, bottom: 2000-offset, height: 2000 } as DOMRect)
  answer.querySelector<HTMLElement>('h1')!.getBoundingClientRect = () => ({ top: 100-offset } as DOMRect)
  answer.querySelector<HTMLElement>('h2')!.getBoundingClientRect = () => ({ top: 600-offset } as DOMRect)
  await waitFor(() => expect(screen.getByRole('button', {name: '跳转到：First'})).toHaveAttribute('aria-current', 'location'))
  await act(async () => { await new Promise((resolve) => setTimeout(resolve, 80)) })
  offset = 500
  viewport.scrollTop = 500
  fireEvent.scroll(viewport)
  await waitFor(() => expect(screen.getByRole('button', {name: '跳转到：Second'})).toHaveAttribute('aria-current', 'location'))
})
