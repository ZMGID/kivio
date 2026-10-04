import { act, fireEvent, render, screen, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { MessageBubble } from './MessageBubble'
import { ChatMarkdown } from './ChatMarkdown'
import { MessageList } from './MessageList'
import { createLongRunningChatFixture } from './performanceFixtures'
import { createStreamPreviewOwner } from './streamPreviewOwner'
import type { CitationView } from './citations'
import * as attachmentPreview from './attachmentPreview'
import type { ChatMessage, ToolCallRecord } from './types'

const markdownRender = vi.hoisted(() => vi.fn())
const reasoningRender = vi.hoisted(() => vi.fn())
vi.mock('./ReasoningBlock', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./ReasoningBlock')>()
  return { ...actual, ReasoningBlock: (props: React.ComponentProps<typeof actual.ReasoningBlock>) => {
    reasoningRender(props)
    return <actual.ReasoningBlock {...props} />
  } }
})
vi.mock('streamdown', async (importOriginal) => {
  const actual = await importOriginal<typeof import('streamdown')>()
  return {
    ...actual,
    Streamdown: (props: React.ComponentProps<typeof actual.Streamdown>) => {
      markdownRender(props.children)
      return <actual.Streamdown {...props} />
    },
  }
})

function longRun(): ChatMessage {
  return {
    id: 'long-run', role: 'assistant', timestamp: 1, content: 'Current',
    segments: [
      ...Array.from({ length: 30 }, (_, index) => ({
        id: `text-${index}`, kind: 'text' as const, phase: 'tool_loop' as const,
        order: index, text: `Completed step ${index}.`,
      })),
      { id: 'active', kind: 'text', phase: 'tool_loop', order: 30, text: 'Current' },
    ],
  }
}

const source: ToolCallRecord = {
  id: 'search', name: 'web_search', status: 'completed',
  structured_content: { citations: [{ title: 'Original source', url: 'https://example.com/old' }] },
}

describe('live message Markdown render boundary', () => {
  beforeEach(() => markdownRender.mockClear())

  it('keeps settled reasoning out of unrelated text and tool updates while retaining live state changes', () => {
    const tool: ToolCallRecord = { id: 'read', name: 'read', status: 'running', result_preview: 'before' }
    const message: ChatMessage = { ...longRun(), tool_calls: [tool], segments: [
      { id: 'thought', kind: 'reasoning', order: 29, phase: 'tool_loop', text: 'Completed thought' },
      ...longRun().segments!,
    ] }
    const { rerender } = render(<MessageBubble message={message} messageStreaming reasoningDurationMs={2000} />)
    reasoningRender.mockClear()
    const next = { ...message, segments: message.segments!.map(segment => segment.id === 'active'
      ? { ...segment, text: 'Current next' } : segment) }
    rerender(<MessageBubble message={next} messageStreaming reasoningDurationMs={2000} />)
    expect(screen.getByText('Current next')).toBeVisible()
    expect(reasoningRender).not.toHaveBeenCalled()
    rerender(<MessageBubble message={{ ...next, tool_calls: [{ ...tool, result_preview: 'after' }] }} messageStreaming reasoningDurationMs={2000} />)
    expect(reasoningRender).not.toHaveBeenCalled()
    rerender(<MessageBubble message={next} messageStreaming reasoningDurationMs={3000} />)
    expect(reasoningRender).toHaveBeenCalledTimes(1)
    expect(reasoningRender.mock.calls[0][0].durationMs).toBe(3000)
  })

  it('ends the active thought when a new segment arrives and preserves an expanded thought through completion', () => {
    const thought = { id: 'thought', kind: 'reasoning' as const, order: 0, phase: 'tool_loop' as const, text: 'First line\nLatest thought' }
    const message: ChatMessage = { id: 'thinking', role: 'assistant', timestamp: 1, content: '', segments: [thought] }
    const { rerender } = render(<MessageBubble message={message} messageStreaming reasoningStreaming />)
    expect(screen.getByText('Thinking…')).toBeVisible()
    fireEvent.click(screen.getByTitle('展开完整思考'))
    const fullThought = screen.getByTestId('reasoning-text')
    const next: ChatMessage = { ...message, segments: [thought,
      { id: 'note', kind: 'text', phase: 'tool_loop', order: 1, text: 'Starting work' },
      { id: 'tool', kind: 'tool', phase: 'tool_loop', order: 2, tool_call_id: 'read' },
    ] }
    rerender(<MessageBubble message={next} messageStreaming reasoningStreaming />)
    expect(screen.queryByText('Thinking…')).toBeNull()
    expect(screen.getByText('Thought')).toBeVisible()
    rerender(<MessageBubble message={next} />)
    expect(screen.getByTestId('reasoning-text')).toBe(fullThought)
    expect(fullThought).toBeVisible()
  })

  it('replaces missing tool records and updates the same tool ID without closing its details', () => {
    const message: ChatMessage = { id: 'tools', role: 'assistant', timestamp: 1, content: '', segments: [
      { id: 'tool', kind: 'tool', phase: 'tool_loop', order: 0, tool_call_id: 'call' },
    ] }
    const { rerender } = render(<MessageBubble message={message} messageStreaming />)
    expect(screen.getByText('工具记录缺失 · call')).toBeVisible()
    const tool: ToolCallRecord = { id: 'call', name: 'fixture_check', status: 'running', result_preview: 'First output' }
    rerender(<MessageBubble message={{ ...message, tool_calls: [tool] }} messageStreaming />)
    expect(screen.queryByText('工具记录缺失 · call')).toBeNull()
    fireEvent.click(screen.getByText('fixture_check'))
    expect(screen.getByText('First output')).toBeVisible()
    rerender(<MessageBubble message={{ ...message, tool_calls: [{ ...tool, status: 'completed', result_preview: 'Updated output' }] }} messageStreaming />)
    expect(screen.getByText('Updated output')).toBeVisible()
    expect(screen.queryByText('First output')).toBeNull()
  })

  it('only renders the changed text segment when the active message grows', () => {
    const message = longRun()
    const { rerender } = render(<MessageBubble message={message} messageStreaming />)
    markdownRender.mockClear()
    rerender(<MessageBubble message={{
      ...message, content: 'Current next',
      segments: message.segments!.map(segment => segment.id === 'active'
        ? { ...segment, text: 'Current next' } : segment),
    }} messageStreaming />)
    expect(screen.getByText('Current next')).toBeVisible()
    expect(markdownRender.mock.calls.map(([text]) => text)).toEqual(['Current next'])
  })

  it.each([false, true])('skips old Markdown when ordinary tool output changes (with citations=%s)', (withCitations) => {
    const tool: ToolCallRecord = { id: 'read', name: 'read', status: 'running', result_preview: 'before' }
    const message = { ...longRun(), tool_calls: [...(withCitations ? [source] : []), tool] }
    const { rerender } = render(<MessageBubble message={message} messageStreaming />)
    markdownRender.mockClear()
    rerender(<MessageBubble message={{ ...message, tool_calls: [
      ...(withCitations ? [{ ...source }] : []), { ...tool, result_preview: 'after', status: 'completed' },
    ] }} messageStreaming />)
    expect(markdownRender).not.toHaveBeenCalled()
    expect(screen.getByText('Completed step 20.')).toBeVisible()
  })

  it('refreshes old text when a citation arrives and when its source is replaced', () => {
    const message: ChatMessage = { id: 'cited', role: 'assistant', timestamp: 1, content: 'See [1].' }
    const { rerender } = render(<MessageBubble message={message} messageStreaming />)
    expect(screen.queryByRole('button', { name: '来源 1' })).toBeNull()
    markdownRender.mockClear()
    rerender(<MessageBubble message={{ ...message, tool_calls: [source] }} messageStreaming />)
    expect(markdownRender).toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: '来源 1' }))
    expect(within(screen.getByRole('dialog')).getByText('Original source')).toBeVisible()
    markdownRender.mockClear()
    rerender(<MessageBubble message={{ ...message, tool_calls: [{ ...source,
      structured_content: { citations: [{ title: 'Replacement source', url: 'https://example.com/new' }] },
    }] }} messageStreaming />)
    if (!screen.queryByRole('dialog')) fireEvent.click(screen.getByRole('button', { name: '来源 1' }))
    expect(within(screen.getByRole('dialog')).getByText('Replacement source')).toBeVisible()
    expect(within(screen.getByRole('dialog')).queryByText('Original source')).toBeNull()
    rerender(<MessageBubble message={message} messageStreaming />)
    expect(screen.queryByRole('button', { name: '来源 1' })).toBeNull()
  })

  it('skips equivalent artifact lists but refreshes a replacement with the same ID', async () => {
    const artifact = { id: 'chart', name: 'chart.png', mime_type: 'image/png', data_url: 'data:image/png;base64,AAAA' }
    const { rerender } = render(<ChatMarkdown content="![chart](chart.png)" artifacts={[artifact]} />)
    await act(async () => { await Promise.resolve() })
    expect(screen.getByRole('img')).toHaveAttribute('src', artifact.data_url)
    markdownRender.mockClear()
    rerender(<ChatMarkdown content="![chart](chart.png)" artifacts={[artifact]} />)
    expect(markdownRender).not.toHaveBeenCalled()
    const replacement = { ...artifact, data_url: 'data:image/png;base64,BBBB' }
    rerender(<ChatMarkdown content="![chart](chart.png)" artifacts={[replacement]} />)
    await act(async () => { await Promise.resolve() })
    expect(screen.getByRole('img')).toHaveAttribute('src', replacement.data_url)
  })

  it('updates late citation candidates inside a cached block without changing literal or code references', () => {
    const content = 'See [1], unknown [002], and `[1]`.'
    const { container, rerender } = render(<ChatMarkdown content={content} />)
    const paragraph = container.querySelector('p')
    const citations = new Map<number, CitationView>([[1, { n: 1, docName: 'Document', score: 1, text: 'First excerpt' }]])
    rerender(<ChatMarkdown content={content} citations={citations} />)
    expect(container.querySelector('p')).toBe(paragraph)
    expect(screen.getAllByRole('button', { name: '来源 1' })).toHaveLength(1)
    fireEvent.click(screen.getByRole('button', { name: '来源 1' }))
    expect(screen.getByText('First excerpt')).toBeVisible()
    rerender(<ChatMarkdown content={content} citations={new Map([[1, { n: 1, docName: 'Document', score: 1, text: 'Revised excerpt' }]])} />)
    expect(screen.getByText('Revised excerpt')).toBeVisible()
    expect(screen.queryByText('First excerpt')).toBeNull()
    rerender(<ChatMarkdown content={content} />)
    expect(screen.queryByRole('button', { name: '来源 1' })).toBeNull()
    expect(container.querySelector('p')).toBe(paragraph)
    expect(container).toHaveTextContent('See [1], unknown [002], and [1].')
  })

  it('does not retain old pixels when a replacement file cannot be loaded', async () => {
    const loader = vi.spyOn(attachmentPreview, 'loadArtifactDataUrl').mockResolvedValue(null)
    try {
      const original = { id: 'chart', name: 'chart.png', mime_type: 'image/png', data_url: 'data:image/png;base64,AAAA' }
      const { rerender } = render(<ChatMarkdown content="![chart](chart.png)" artifacts={[original]} />)
      expect(screen.getByRole('img')).toHaveAttribute('src', original.data_url)
      const replacement = { id: 'chart', name: 'chart.png', mime_type: 'image/png', path: '/new/chart.png' }
      rerender(<ChatMarkdown content="![chart](chart.png)" artifacts={[replacement]} />)
      await act(async () => { await Promise.resolve() })
      expect(loader).toHaveBeenCalled()
      expect(screen.queryByRole('img')).toBeNull()
    } finally {
      loader.mockRestore()
    }
  })

  it('updates outline registration when its owner or callback changes', () => {
    const onChange = vi.fn()
    const nextOnChange = vi.fn()
    const outline = { ownerMessageId: 'first', sourceId: 'first-source', onChange }
    const { container, rerender } = render(<ChatMarkdown content="# Heading" outlineSource={outline} />)
    markdownRender.mockClear()
    rerender(<ChatMarkdown content="# Heading" outlineSource={{ ...outline }} />)
    expect(markdownRender).not.toHaveBeenCalled()
    rerender(<ChatMarkdown content="# Heading" outlineSource={{
      ownerMessageId: 'next', sourceId: 'next-source', onChange: nextOnChange,
    }} />)
    expect(container.querySelector('[data-chat-outline-source-id="next-source"]')).not.toBeNull()
    expect(nextOnChange).toHaveBeenCalledWith(expect.objectContaining({ ownerMessageId: 'next', sourceId: 'next-source' }))
  })

  it('preserves the render boundary through the real stream and tool event owner', async () => {
    const owner = createStreamPreviewOwner()
    const fixture = createLongRunningChatFixture(20)
    try {
      act(() => {
        owner.activate('F5')
        owner.begin('F5')
        for (const turn of fixture.turns) {
          owner.receive(turn.text)
          owner.projectDisplay({ kind: 'tool', payload: turn.tool })
        }
        owner.receive(fixture.active)
      })
      render(<MessageList conversationId="F5" messages={[]} />)
      await screen.findByText('Current output')
      markdownRender.mockClear()
      act(() => { owner.receive(fixture.append) })
      await screen.findByText('Current output next token')
      expect(markdownRender.mock.calls.map(([text]) => text)).toEqual(['Current output next token'])
      markdownRender.mockClear()
      act(() => {
        owner.projectDisplay({ kind: 'tool', payload: { ...fixture.turns[0].tool, status: 'running', resultPreview: 'more output' } })
      })
      // Publish the pending tool update before checking absence of work.
      act(() => { owner.activate('F5') })
      expect(markdownRender).not.toHaveBeenCalled()
      act(() => { owner.freeze('F5') })
      expect(screen.getByText('Current output next token')).toBeVisible()
    } finally {
      act(() => { owner.dispose() })
    }
  })
})
