import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ContextIndicator } from './ContextIndicator'
import type { ConversationContextState } from './types'

const breakdownState: ConversationContextState = {
  reported_context_tokens: 53_000,
  token_count_source: 'provider_context_reported',
  context_window_tokens: 1_000_000,
  segments: [
    { id: 'system_prompt', label: '', chars: 200, estimated_tokens: 1_000 },
    { id: 'tools', label: '', chars: 500, estimated_tokens: 4_000 },
    { id: 'conversation', label: '', chars: 300, estimated_tokens: 2_000 },
  ],
}

function openPanel(props: Partial<Parameters<typeof ContextIndicator>[0]>) {
  const onStopCompression = vi.fn()
  render(
    <ContextIndicator
      contextState={null}
      messageCount={4}
      compressing
      onCompress={vi.fn()}
      onStopCompression={onStopCompression}
      lang="en"
      {...props}
    />,
  )
  fireEvent.click(screen.getByLabelText('Context'))
  return onStopCompression
}

describe('ContextIndicator character share', () => {
  it('labels UTF-16 shares beside three category estimates and the k capacity', () => {
    render(
      <ContextIndicator
        contextState={breakdownState}
        messageCount={4}
        lang="en"
        onRefresh={vi.fn()}
        onCompress={vi.fn()}
      />,
    )
    fireEvent.click(screen.getByLabelText('Context'))
    expect(screen.getByText('53.0k/1000.0k (5.3%)')).toBeTruthy()
    expect(screen.getByText('Character share')).toBeTruthy()
    expect(screen.getByText('System prompt')).toBeTruthy()
    expect(screen.getByText('Tools')).toBeTruthy()
    expect(screen.getByText('Conversation')).toBeTruthy()
    expect(screen.getByText('≈ 4.0k')).toBeTruthy()
    expect(screen.getByText('≈ 2.0k')).toBeTruthy()
    expect(screen.getByText('≈ 1.0k')).toBeTruthy()
    expect(screen.getByText('50%')).toBeTruthy()
    expect(screen.getByText('30%')).toBeTruthy()
    expect(screen.getByText('20%')).toBeTruthy()
    expect(screen.getByTitle(/UTF-16 character shares, not token shares/)).toBeTruthy()
  })

  it('uses the Chinese character-share label without changing the three rows', () => {
    render(
      <ContextIndicator
        contextState={breakdownState}
        messageCount={4}
        lang="zh"
        onRefresh={vi.fn()}
        onCompress={vi.fn()}
      />,
    )
    fireEvent.click(screen.getByLabelText('上下文'))
    expect(screen.getByText('字符占比')).toBeTruthy()
    expect(screen.getByText('系统提示词')).toBeTruthy()
    expect(screen.getByText('工具')).toBeTruthy()
    expect(screen.getByText('对话')).toBeTruthy()
    expect(screen.getByText('≈ 4.0k')).toBeTruthy()
  })
})

describe('ContextIndicator stop action', () => {
  it('offers to stop a manual compaction', () => {
    const onStop = openPanel({ generating: false })
    fireEvent.click(screen.getByRole('button', { name: 'Stop compaction' }))
    expect(onStop).toHaveBeenCalledTimes(1)
  })

  it('says it stops the generation during automatic compaction', () => {
    // Automatic compaction runs inside a generation; stopping it stops that generation.
    openPanel({ generating: true })
    expect(screen.getByRole('button', { name: 'Stop generating' })).toBeTruthy()
    expect(screen.queryByRole('button', { name: 'Stop compaction' })).toBeNull()
  })
})
