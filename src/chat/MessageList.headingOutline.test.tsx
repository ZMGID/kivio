import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { MessageList } from './MessageList'

describe('MessageList heading outline', () => {
  it('shows the settled current assistant outline but not its process text', async () => {
    const { container } = render(
      <MessageList
        conversationId="outline-current-message"
        messages={[
          { id: 'question', role: 'user', content: 'Question', timestamp: 1 },
          {
            id: 'answer', role: 'assistant', content: '# Final one\n## Final two', timestamp: 2,
            segments: [
              { id: 'process', kind: 'text', phase: 'tool_loop', order: 0, text: '# Process heading' },
              { id: 'final', kind: 'text', phase: 'synthesis', order: 1, text: '# Final one\n## Final two' },
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
    expect(screen.queryByText('Process heading')).not.toBeInTheDocument()
  })
})
