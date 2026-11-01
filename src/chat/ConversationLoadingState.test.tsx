import { act, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ConversationLoadingState } from './ConversationLoadingState'

describe('ConversationLoadingState', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })
  afterEach(() => {
    vi.useRealTimers()
  })

  it('keeps the previous conversation visible for fast loads, then covers it with the logo', () => {
    const { container } = render(<ConversationLoadingState />)
    act(() => { vi.advanceTimersByTime(399) })
    expect(container.querySelector('.chat-conversation-loading')).toBeNull()
    act(() => { vi.advanceTimersByTime(1) })
    expect(container.querySelector('.chat-conversation-loading .kv-stream-dot-logo')).not.toBeNull()
  })

  it('never covers the list when the load finishes before the delay', () => {
    const { container, unmount } = render(<ConversationLoadingState />)
    unmount()
    act(() => { vi.advanceTimersByTime(1000) })
    expect(container.querySelector('.chat-conversation-loading')).toBeNull()
  })
})
