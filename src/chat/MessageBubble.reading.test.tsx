import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { MessageBubble } from './MessageBubble'
import type { ChatMessage } from './types'

const timeline: ChatMessage = {
  id: 'reading-safe-answer', role: 'assistant', timestamp: 1,
  content: 'A useful answer with ![remote](https://example.com/tracker.png) and [preview](artifact:unsafe-file).',
  artifacts: [{ id: 'unsafe-file', name: 'unsafe.html', mime_type: 'text/html', path: '/tmp/unsafe.html' }],
  tool_calls: [{ id: 'reading-call', name: 'read_local_file', status: 'completed' }],
  segments: [
    { id: 'reading-reasoning', kind: 'reasoning', phase: 'tool_loop', order: 0, text: 'Process detail remains available in normal read-only views' },
    { id: 'reading-tool', kind: 'tool', phase: 'tool_loop', order: 1, tool_call_id: 'reading-call' },
    { id: 'reading-text', kind: 'text', phase: 'synthesis', order: 2, text: 'A useful answer with ![remote](https://example.com/tracker.png) and [preview](artifact:unsafe-file).' },
  ],
}

describe('shared MessageBubble reading presentation', () => {
  it('renders safe text from the existing answer body without media, previews, or tool actions', () => {
    const { container } = render(<MessageBubble message={timeline} presentation="reading" />)
    expect(screen.getByText(/A useful answer/)).toBeVisible()
    expect(container.querySelector('img, iframe, video, audio, object, embed')).toBeNull()
    expect(container.querySelector('a[href]')).toBeNull()
    expect(screen.queryByRole('button', { name: /^Worked/ })).toBeNull()
    expect(screen.queryByRole('button', { name: '重新生成' })).toBeNull()
    expect(screen.queryByRole('button', { name: '存为笔记' })).toBeNull()
    expect(screen.queryByText('read_local_file')).toBeNull()
    expect(screen.queryByText('unsafe.html')).toBeNull()
  })

  it('keeps pre-existing readOnly subagent timelines and reasoning available', () => {
    const safeTimeline = { ...timeline, content: 'Final text', artifacts: [], segments: [timeline.segments![0], { id: 'final', kind: 'text' as const, phase: 'synthesis' as const, order: 2, text: 'Final text' }] }
    render(<MessageBubble message={safeTimeline} readOnly />)
    fireEvent.click(screen.getByRole('button', { name: /^Worked/ }))
    expect(screen.getByText('Final text')).toBeVisible()
    fireEvent.click(screen.getByRole('button', { name: 'Thought' }))
    expect(screen.getByText('Process detail remains available in normal read-only views')).toBeVisible()
    expect(screen.queryByRole('button', { name: '重新生成' })).toBeNull()
  })

  it('uses the shared disclosure for an explicit full solution, including partial streaming', () => {
    const view = render(<MessageBubble presentation="reading" messageStreaming
      answerDisclosureLabel="Reveal full solution" message={{ id: 'solution', role: 'assistant', timestamp: 1, content: 'First part' }} />)
    expect(screen.queryByText('First part')).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Reveal full solution' }))
    expect(screen.getByText('First part')).toBeVisible()
    view.rerender(<MessageBubble presentation="reading" answerDisclosureLabel="Reveal full solution"
      message={{ id: 'solution', role: 'assistant', timestamp: 1, content: 'Complete solution', stream_outcome: 'cancelled' }} />)
    expect(screen.getByText('Complete solution')).toBeVisible()
    fireEvent.click(screen.getByRole('button', { name: 'Reveal full solution' }))
    expect(screen.queryByText('Complete solution')).toBeNull()
  })

  it.each(['', 'Partial imported answer'])('retains assistant-side imported-history notices with body %j', content => {
    render(<MessageBubble presentation="reading" annotation={<p role="status">This imported answer was interrupted</p>}
      message={{ id: 'imported', role: 'assistant', timestamp: 1, content }} />)
    expect(screen.getByRole('status')).toHaveTextContent('This imported answer was interrupted')
    if (content) expect(screen.getByText(content)).toBeVisible()
  })

  it('keeps a source annotation interactive while suppressing user attachment previews', () => {
    const jump = vi.fn()
    const { container } = render(<MessageBubble presentation="reading" annotation={<button onClick={jump}>Page 2 · selected region</button>}
      message={{ id: 'question', role: 'user', timestamp: 1, content: 'Explain this', attachments: [{ id: 'att', type: 'image', name: 'remote.png', path: 'https://example.com/private.png' }] }} />)
    fireEvent.click(screen.getByRole('button', { name: 'Page 2 · selected region' }))
    expect(jump).toHaveBeenCalledOnce()
    expect(screen.getByText('Explain this')).toBeVisible()
    expect(container.querySelector('img')).toBeNull()
  })
})
