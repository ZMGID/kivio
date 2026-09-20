import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { MessageList } from './MessageList'
import type { ChatMessage } from './types'

describe('artifact references across turns', () => {
  it('opens a file created before a stopped reply when a later reply cites it', () => {
    const messages: ChatMessage[] = [
      { id: 'first-question', role: 'user', content: 'Collect sources', timestamp: 1 },
      {
        id: 'stopped-answer', role: 'assistant', content: 'Generation stopped', timestamp: 2,
        stream_outcome: 'cancelled',
        tool_calls: [{
          id: 'write-file', name: 'write', status: 'success',
          artifacts: [{ id: 'art_source_map', name: 'source-map.md', path: '/workspace/source-map.md' }],
        }],
      },
      { id: 'next-question', role: 'user', content: 'Just collect them', timestamp: 3 },
      {
        id: 'next-answer', role: 'assistant', timestamp: 4,
        content: 'I collected [source-map.md](artifact:art_source_map).',
      },
    ]

    render(<MessageList conversationId="same-conversation" messages={messages} />)

    expect(screen.getByRole('button', { name: '打开文件 source-map.md' })).toBeVisible()
    expect(screen.queryByText(/文件不可用/)).not.toBeInTheDocument()
  })

  it('does not resolve an artifact ID from a different conversation', () => {
    const previous: ChatMessage[] = [{
      id: 'file', role: 'assistant', content: 'Created it', timestamp: 1,
      artifacts: [{ id: 'art_source_map', name: 'source-map.md', path: '/workspace/source-map.md' }],
    }]
    const { rerender } = render(<MessageList conversationId="previous" messages={previous} />)

    rerender(<MessageList conversationId="next" messages={[{
      id: 'answer', role: 'assistant', timestamp: 2,
      content: '[source-map.md](artifact:art_source_map)',
    }]} />)

    expect(screen.getByRole('status')).toHaveTextContent('文件不可用')
    expect(screen.queryByRole('button', { name: '打开文件 source-map.md' })).not.toBeInTheDocument()
  })
})
