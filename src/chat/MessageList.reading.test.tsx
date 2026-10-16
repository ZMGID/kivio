import { act, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { MessageList } from './MessageList'
import { patchSnapshot, reset, setCoarse } from './streamingStore'
import type { ChatMessage } from './types'

afterEach(() => act(() => reset()))
const question: ChatMessage = { id: 'source-question', role: 'user', timestamp: 1, content: 'Explain the marked sentence' }

describe('shared MessageList reading presentation', () => {
  it('keeps the normal history list with source navigation and a safe answer profile', async () => {
    const navigate = vi.fn()
    const { container } = render(<MessageList presentation="reading" conversationId="reading-list"
      renderMessageAnnotation={message => message.role === 'user' ? <button onClick={() => navigate(message.id)}>Page 7</button> : null}
      messages={[question, { id: 'source-answer', role: 'assistant', timestamp: 2, content: 'Explanation ![remote](https://example.com/image.png)' }]} />)
    await act(async () => { await Promise.resolve() })
    fireEvent.click(screen.getByRole('button', { name: 'Page 7' }))
    expect(navigate).toHaveBeenCalledWith(question.id)
    expect(screen.getByText(question.content)).toBeVisible()
    expect(screen.getByText(/Explanation/)).toBeVisible()
    expect(container.querySelector('.chat-message-list-inner--reading')).not.toBeNull()
    expect(container.querySelector('img')).toBeNull()
  })

  it('also applies safe rendering to stored context summaries before and after disclosure', async () => {
    const { container } = render(<MessageList presentation="reading" conversationId="reading-compaction"
      messages={[question]} contextState={{ compaction_boundaries: [{ id: 'summary', source_until_message_id: question.id,
        summary_content: 'Saved context ![remote](https://example.com/summary.png)', trigger: 'auto',
      }] }} />)
    await act(async () => { await Promise.resolve() })
    expect(container.querySelector('img')).toBeNull()
    fireEvent.click(container.querySelector('.chat-compaction-summary-toggle')!)
    expect(screen.getByText(/Saved context/)).toBeInTheDocument()
    expect(container.querySelector('img, iframe, video, audio, object, embed')).toBeNull()
  })

  it('uses the same live stream then stored twin without duplicate answers', async () => {
    act(() => {
      setCoarse({ streaming: true })
      patchSnapshot({ runId: 'reading-run', messageId: 'reading-live', streaming: true, content: 'Partial explanation' })
    })
    const view = render(<MessageList presentation="reading" conversationId="reading-stream" messages={[question]} />)
    await act(async () => { await Promise.resolve() })
    expect(screen.getByText('Partial explanation')).toBeVisible()
    act(() => patchSnapshot({ content: 'Completed explanation' }))
    act(() => { setCoarse({ streamFrozen: true }); patchSnapshot({ streaming: false }) })
    view.rerender(<MessageList presentation="reading" conversationId="reading-stream" messages={[question,
      { id: 'reading-live', role: 'assistant', timestamp: 2, content: 'Completed explanation', stream_outcome: 'completed' },
    ]} />)
    act(() => reset())
    await act(async () => { await Promise.resolve() })
    expect(screen.getAllByText('Completed explanation')).toHaveLength(1)
  })

  it('preserves the existing failed-send retry action', async () => {
    const retry = vi.fn()
    act(() => setCoarse({ streamError: 'network timeout' }))
    render(<MessageList presentation="reading" conversationId="reading-retry" messages={[question]} onRetryLastUser={retry} />)
    await act(async () => { await Promise.resolve() })
    fireEvent.click(screen.getByRole('button', { name: '重试' }))
    expect(retry).toHaveBeenCalledWith(question.id)
  })

  it.each(['cancelled', 'error', 'interrupted'] as const)('keeps shared regeneration for a persisted %s assistant reply', async outcome => {
    const regenerate = vi.fn().mockResolvedValue(undefined)
    render(<MessageList presentation="reading" conversationId={`reading-retry-${outcome}`}
      onRegenerateMessage={regenerate} messages={[question, {
        id: 'partial-assistant', role: 'assistant', timestamp: 2, content: 'Useful partial explanation', stream_outcome: outcome,
      }]} />)
    await act(async () => { await Promise.resolve() })
    expect(screen.getByText('Useful partial explanation')).toBeVisible()
    fireEvent.click(screen.getByRole('button', { name: '重新生成' }))
    expect(regenerate).toHaveBeenCalledWith('partial-assistant')
    expect(screen.queryByRole('button', { name: '存为笔记' })).toBeNull()
    expect(screen.queryByRole('button', { name: '建分支' })).toBeNull()
  })

  it('also offers shared regeneration for a persisted empty failed assistant', async () => {
    const regenerate = vi.fn().mockResolvedValue(undefined)
    render(<MessageList presentation="reading" conversationId="reading-empty-retry" onRegenerateMessage={regenerate}
      messages={[question, { id: 'empty-assistant', role: 'assistant', timestamp: 2, content: '', stream_outcome: 'error' }]} />)
    await act(async () => { await Promise.resolve() })
    fireEvent.click(screen.getByRole('button', { name: '重新生成' }))
    expect(regenerate).toHaveBeenCalledWith('empty-assistant')
  })

  it('applies the safe presentation and explicit solution disclosure to grouped history too', async () => {
    const { container } = render(<MessageList presentation="reading" conversationId="reading-group"
      answerDisclosureLabel={() => 'Reveal solution'} messages={[question,
        { id: 'group-a', group_id: 'group', role: 'assistant', timestamp: 2, content: 'Hidden answer ![remote](https://example.com/image.png)', model: 'model-a' },
        { id: 'group-b', group_id: 'group', role: 'assistant', timestamp: 3, content: 'Other answer', model: 'model-b' },
      ]} />)
    await act(async () => { await Promise.resolve() })
    expect(screen.queryByText(/Hidden answer/)).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Reveal solution' }))
    expect(screen.getByText(/Hidden answer/)).toBeVisible()
    expect(container.querySelector('img')).toBeNull()
  })
})
