import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { MessageList } from './MessageList'

describe('MessageList heading outline', () => {
  it('keeps outlines and heading targets separate when answers reuse segment ids', async () => {
    const { container } = render(
      <MessageList
        conversationId="outline-reused-segments"
        messages={[
          { id: 'question-a', role: 'user', content: '111', timestamp: 1 },
          {
            id: 'answer-a', role: 'assistant', content: '# aaa\n## aaa detail', timestamp: 2,
            segments: [{ id: 'final', kind: 'text', phase: 'synthesis', order: 0, text: '# aaa\n## aaa detail' }],
          },
          { id: 'question-b', role: 'user', content: '222', timestamp: 3 },
          {
            id: 'answer-b', role: 'assistant', content: '# bbb\n## bbb detail', timestamp: 4,
            segments: [{ id: 'final', kind: 'text', phase: 'synthesis', order: 0, text: '# bbb\n## bbb detail' }],
          },
        ]}
      />,
    )
    await act(async () => { await Promise.resolve() })
    const viewport = container.querySelector<HTMLElement>('.chat-scroll-viewport')!
    const first = container.querySelector<HTMLElement>('[data-chat-outline-owner="answer-a"]')!
    const second = container.querySelector<HTMLElement>('[data-chat-outline-owner="answer-b"]')!
    Object.defineProperty(viewport, 'clientHeight', { configurable: true, value: 800 })
    Object.defineProperty(viewport, 'scrollTop', { configurable: true, writable: true, value: 0 })
    Object.defineProperty(viewport, 'scrollHeight', { configurable: true, value: 3000 })
    viewport.getBoundingClientRect = () => ({ top: 0, bottom: 800, height: 800 } as DOMRect)
    let readingSecond = false
    first.getBoundingClientRect = () => ({ top: readingSecond ? -1000 : 0, bottom: readingSecond ? 0 : 1000, height: 1000 } as DOMRect)
    second.getBoundingClientRect = () => ({ top: readingSecond ? 0 : 1000, bottom: readingSecond ? 1000 : 2000, height: 1000 } as DOMRect)

    fireEvent.scroll(viewport)
    await waitFor(() => expect(screen.getByLabelText('回答标题目录')).toBeInTheDocument())
    fireEvent.pointerEnter(screen.getByLabelText('回答标题目录'))
    await waitFor(() => expect(screen.getByRole('button', { name: 'aaa' })).toBeInTheDocument())

    readingSecond = true
    fireEvent.scroll(viewport)
    await waitFor(() => expect(screen.getByRole('button', { name: 'bbb' })).toBeInTheDocument())
    expect(screen.queryByRole('button', { name: 'aaa' })).not.toBeInTheDocument()
    const firstHeading = first.querySelector('h1')!
    const secondHeading = second.querySelector('h1')!
    expect(firstHeading.id).not.toBe(secondHeading.id)
    expect(container.querySelector(`#${CSS.escape(secondHeading.id)}`)).toBe(secondHeading)
    firstHeading.getBoundingClientRect = vi.fn(() => ({ top: -1000, bottom: -960, height: 40 } as DOMRect))
    secondHeading.getBoundingClientRect = vi.fn(() => ({ top: 420, bottom: 460, height: 40 } as DOMRect))
    second.closest<HTMLElement>('[data-chat-row-index]')!.getBoundingClientRect = () => (
      { top: 0, bottom: 1000, height: 1000 } as DOMRect
    )
    fireEvent.click(screen.getByRole('button', { name: 'bbb' }))
    await waitFor(() => expect(secondHeading.getBoundingClientRect).toHaveBeenCalled())
    expect(firstHeading.getBoundingClientRect).not.toHaveBeenCalled()

    // Returning to history must restore that answer's outline, too.
    fireEvent.wheel(viewport, { deltaY: -800 })
    readingSecond = false
    fireEvent.scroll(viewport)
    await waitFor(() => expect(screen.getByRole('button', { name: 'aaa' })).toBeInTheDocument())
    expect(screen.queryByRole('button', { name: 'bbb' })).not.toBeInTheDocument()
  })

  it('includes tool-loop body headings but excludes reasoning from the settled outline', async () => {
    const { container } = render(
      <MessageList
        conversationId="outline-current-message"
        messages={[
          { id: 'question', role: 'user', content: 'Question', timestamp: 1 },
          {
            id: 'answer', role: 'assistant', content: '# Final one\n## Final two', timestamp: 2,
            segments: [
              { id: 'process', kind: 'reasoning', phase: 'tool_loop', order: 0, text: '# Process heading' },
              { id: 'questions', kind: 'text', phase: 'tool_loop', order: 1, text: '# Questions' },
              { id: 'final', kind: 'text', phase: 'synthesis', order: 2, text: '# Final one\n## Final two' },
            ],
          },
        ]}
      />,
    )
    await act(async () => { await Promise.resolve() })
    const viewport = container.querySelector<HTMLElement>('.chat-scroll-viewport')!
    const answer = container.querySelector<HTMLElement>('[data-chat-outline-owner="answer"]')!
    Object.defineProperty(viewport, 'clientHeight', { configurable: true, value: 800 })
    Object.defineProperty(viewport, 'scrollTop', { configurable: true, writable: true, value: 0 })
    Object.defineProperty(viewport, 'scrollHeight', { configurable: true, value: 2000 })
    viewport.getBoundingClientRect = () => ({ top: 0, bottom: 800, height: 800 } as DOMRect)
    answer.getBoundingClientRect = () => ({ top: 0, bottom: 1000, height: 1000 } as DOMRect)
    fireEvent.scroll(viewport)
    await waitFor(() => expect(screen.getByLabelText('回答标题目录')).toBeInTheDocument())
    fireEvent.pointerEnter(screen.getByLabelText('回答标题目录'))
    expect(screen.getByRole('button', { name: 'Final one' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Questions' })).toBeInTheDocument()
    expect(screen.queryByText('Process heading')).not.toBeInTheDocument()

    // With only one turn, scrolling away must still clear the answer outline.
    answer.getBoundingClientRect = () => ({ top: -1200, bottom: -200, height: 1000 } as DOMRect)
    fireEvent.scroll(viewport)
    await waitFor(() => expect(screen.queryByLabelText('回答标题目录')).not.toBeInTheDocument())
  })
})
