import { describe, expect, it, vi } from 'vitest'
import {
  beginConversationTransition,
  awaitCurrentConversationNavigation,
  cancelConversationTransition,
  captureConversationNavigation,
  completeConversationTransition,
  getConversationTransitionSnapshot,
  invalidateConversationTransition,
  isCurrentConversationNavigation,
  isCurrentConversationTransition,
} from './conversationTransitionStore'

describe('conversationTransitionStore', () => {
  it('keeps sidebar selection on the newest request while older loads finish harmlessly', () => {
    const first = beginConversationTransition('conversation-a')
    const second = beginConversationTransition('conversation-b')

    expect(isCurrentConversationTransition(first, 'conversation-a')).toBe(false)
    expect(isCurrentConversationTransition(second, 'conversation-b')).toBe(true)

    completeConversationTransition('conversation-a', first)
    expect(getConversationTransitionSnapshot()).toMatchObject({
      targetConversationId: 'conversation-b',
      loading: true,
    })

    completeConversationTransition('conversation-b', second)
    expect(getConversationTransitionSnapshot()).toMatchObject({
      targetConversationId: 'conversation-b',
      loading: false,
    })
  })

  it('can invalidate a pending load when starting a new conversation', () => {
    const requestId = beginConversationTransition('conversation-a')
    invalidateConversationTransition()
    cancelConversationTransition(requestId)

    expect(getConversationTransitionSnapshot()).toMatchObject({
      targetConversationId: null,
      loading: false,
    })
  })

  it('only shows the loading shell for larger conversations', () => {
    // threshold is exclusive: ≤12 messages skip the logo shell so small opens feel instant
    beginConversationTransition('small', { messageCount: 12 })
    expect(getConversationTransitionSnapshot().showLoading).toBe(false)

    beginConversationTransition('large', { messageCount: 13 })
    expect(getConversationTransitionSnapshot().showLoading).toBe(true)

    // unknown size stays conservative
    beginConversationTransition('unknown')
    expect(getConversationTransitionSnapshot().showLoading).toBe(true)
  })

  it('invalidates a captured navigation lease without cancelling background work', async () => {
    beginConversationTransition('conversation-a')
    const lease = captureConversationNavigation()
    let finish: ((value: string) => void) | undefined
    const pending = new Promise<string>((resolve) => { finish = resolve })
    const committed = vi.fn()
    const backgroundCancelled = vi.fn()
    const result = pending.then((value) => {
      if (isCurrentConversationNavigation(lease)) committed(value)
    })

    invalidateConversationTransition()
    finish?.('late value')
    await result

    expect(committed).not.toHaveBeenCalled()
    expect(backgroundCancelled).not.toHaveBeenCalled()
  })

  it('also suppresses an error arriving after its navigation lease was invalidated', async () => {
    beginConversationTransition('missing-a')
    const lease = captureConversationNavigation()
    let fail: ((error: Error) => void) | undefined
    const pending = new Promise<void>((_resolve, reject) => { fail = reject })
    const publishError = vi.fn()
    const result = pending.catch((error: Error) => {
      if (isCurrentConversationNavigation(lease)) publishError(error.message)
    })

    invalidateConversationTransition()
    fail?.(new Error('not found'))
    await result

    expect(publishError).not.toHaveBeenCalled()
  })

  it('marks an ownership lookup stale when navigation changes while its promise is pending', async () => {
    const requestId = beginConversationTransition('conversation-a')
    let finish: ((ids: ReadonlySet<string>) => void) | undefined
    const ownership = new Promise<ReadonlySet<string>>((resolve) => { finish = resolve })
    const result = awaitCurrentConversationNavigation(
      ownership,
      () => isCurrentConversationTransition(requestId, 'conversation-a'),
    )

    invalidateConversationTransition()
    finish?.(new Set(['conversation-a']))

    await expect(result).resolves.toEqual({ status: 'stale' })
  })
})
