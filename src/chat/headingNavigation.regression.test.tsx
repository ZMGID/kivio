import { act, fireEvent, render, renderHook, screen, waitFor } from '@testing-library/react'
import { expect, it, vi } from 'vitest'
import { ChatMarkdown } from './ChatMarkdown'
import { MessageList } from './MessageList'
import { useMultiAnswerViewMode } from './multiAnswerViewMode'

it.each(['> ## Quoted heading', '- ## List heading', '#'])('keeps root anchors aligned after excluded Markdown: %s', async (prefix) => {
  const { container } = render(<ChatMarkdown
    content={`${prefix}\n\n# Actual first\n\n## Actual second`}
    outlineSource={{ ownerMessageId: 'answer', sourceId: 'answer', onChange: vi.fn() }} />)
  await waitFor(() => expect(container.querySelector('[id="user-content-chat-heading-answer-0"]')).toHaveTextContent('Actual first'))
  expect(container.querySelector('[id="user-content-chat-heading-answer-1"]')).toHaveTextContent('Actual second')
})

it('follows the focused answer and its inner scroll in side-by-side mode', async () => {
  const mode = renderHook(() => useMultiAnswerViewMode())
  act(() => mode.result.current[1]('columns'))
  const view = render(<MessageList conversationId="columns-outline-review" messages={[
    { id: 'user', role: 'user', content: 'Question', timestamp: 1 },
    { id: 'a', role: 'assistant', group_id: 'group', content: '# A first\n\n## A second', timestamp: 2 },
    { id: 'b', role: 'assistant', group_id: 'group', content: '# B first\n\n## B second', timestamp: 2 },
  ]} />)
  try {
    await act(async () => { await Promise.resolve() })
    // Consume the initial outline measurement before installing synthetic geometry.
    // CI may reach this frame sooner than local runs; changing a DOM mock does not
    // itself notify the navigator that layout has changed.
    await act(async () => { await new Promise<void>((resolve) => requestAnimationFrame(() => resolve())) })
    const viewport = view.container.querySelector<HTMLElement>('.chat-scroll-viewport')!
    Object.defineProperty(viewport, 'clientHeight', { configurable: true, value: 800 })
    viewport.getBoundingClientRect = () => ({ top: 0, bottom: 800, height: 800 } as DOMRect)
    for (const root of view.container.querySelectorAll<HTMLElement>('[data-chat-outline-owner]')) {
      const body = root.closest<HTMLElement>('.chat-message-group-col-body')!
      Object.defineProperty(body, 'clientHeight', { configurable: true, value: 560 })
      body.getBoundingClientRect = () => ({ top: 100, bottom: 660, height: 560 } as DOMRect)
      root.closest<HTMLElement>('[data-chat-row-index]')!.getBoundingClientRect = () => ({ top: 100, bottom: 660, height: 560 } as DOMRect)
      root.getBoundingClientRect = () => ({ top: 100 - body.scrollTop, bottom: 2100 - body.scrollTop, height: 2000 } as DOMRect)
      root.querySelector<HTMLElement>('h1')!.getBoundingClientRect = () => ({ top: 120 - body.scrollTop } as DOMRect)
      root.querySelector<HTMLElement>('h2')!.getBoundingClientRect = () => ({ top: 900 - body.scrollTop } as DOMRect)
    }
    fireEvent.scroll(viewport)
    await waitFor(() => expect(screen.getByRole('button', { name: '跳转到：A first' })).toHaveAttribute('aria-current', 'location'))
    const second = view.container.querySelectorAll<HTMLElement>('.chat-message-group-col')[1]
    fireEvent.mouseOver(second)
    await waitFor(() => expect(screen.getByRole('button', { name: '跳转到：B first' })).toHaveAttribute('aria-current', 'location'))
    const body = second.querySelector<HTMLElement>('.chat-message-group-col-body')!
    body.scrollTop = 700
    fireEvent.scroll(body)
    await waitFor(() => expect(screen.getByRole('button', { name: '跳转到：B second' })).toHaveAttribute('aria-current', 'location'))
    body.scrollTop = 0
    fireEvent.click(screen.getByRole('button', { name: '跳转到：B second' }))
    await waitFor(() => expect(body.scrollTop).toBeGreaterThan(700))
  } finally {
    view.unmount()
    act(() => mode.result.current[1]('tabs'))
    mode.unmount()
  }
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
  fireEvent.scroll(viewport)
  await waitFor(() => expect(screen.getByRole('button', {name: '跳转到：First'})).toHaveAttribute('aria-current', 'location'))
  await act(async () => { await new Promise((resolve) => setTimeout(resolve, 80)) })
  offset = 500
  viewport.scrollTop = 500
  fireEvent.scroll(viewport)
  await waitFor(() => expect(screen.getByRole('button', {name: '跳转到：Second'})).toHaveAttribute('aria-current', 'location'))
})
