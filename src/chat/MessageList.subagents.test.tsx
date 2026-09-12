import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { MessageList } from './MessageList'
import type { ChatMessage } from './types'

describe('sub-agent result receipts in the parent timeline', () => {
  it('keeps persisted worker reports out of the timeline while showing the final answer', () => {
    const messages: ChatMessage[] = [
      { id: 'question', role: 'user', content: '调查这个项目', timestamp: 1 },
      { id: 'subagent-result-execution-a', role: 'assistant', content: '[Sub-agent: A · Completed]\nRaw worker report', timestamp: 2 },
      { id: 'answer', role: 'assistant', content: '主代理整理后的结论', timestamp: 3 },
    ]
    render(<MessageList conversationId="subagent-receipt" messages={messages} />)
    expect(screen.getByText('主代理整理后的结论')).toBeInTheDocument()
    expect(screen.queryByText(/Raw worker report/)).not.toBeInTheDocument()
    expect(messages).toHaveLength(3)
  })

  it('does not hide user or assistant text just because it quotes a worker header', () => {
    render(<MessageList conversationId="subagent-quote" messages={[
      { id: 'question', role: 'user', content: '[Sub-agent: A · Completed] 是什么？', timestamp: 1 },
      { id: 'answer', role: 'assistant', content: '[Sub-agent: A · Completed] 是内部回执标题。', timestamp: 2 },
    ]} />)
    expect(screen.getByText(/是什么？/)).toBeInTheDocument()
    expect(screen.getByText(/是内部回执标题/)).toBeInTheDocument()
  })
})
