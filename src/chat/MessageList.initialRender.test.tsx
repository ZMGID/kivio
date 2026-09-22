import { act, fireEvent, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { MessageList, type MessageListProps } from './MessageList'
import { ConversationLoadingState } from './ConversationLoadingState'
import { beginConversationTransition, completeConversationTransition, invalidateConversationTransition, useConversationTransition } from './conversationTransitionStore'
import { reset, setCoarse } from './streamingStore'

beforeEach(() => {
  vi.useFakeTimers()
  vi.spyOn(performance, 'now').mockImplementation(() => Date.now())
})
afterEach(() => {
  act(() => reset())
  act(() => invalidateConversationTransition())
  vi.useRealTimers()
  vi.restoreAllMocks()
})

async function frame() {
  await act(async () => { await vi.advanceTimersByTimeAsync(16) })
}

async function settle() {
  for (let index = 0; index < 12; index += 1) await frame()
}

async function mount() {
  const ready = vi.fn()
  const props = {
    conversationId: 'opening-layout', renderRequestId: 1, onInitialRender: ready,
    messages: [{ id: 'answer', role: 'assistant', content: 'Answer', timestamp: 1 }],
  } satisfies MessageListProps
  const { container, unmount, rerender } = render(<MessageList {...props} />)
  await act(async () => { await Promise.resolve() })
  const viewport = container.querySelector<HTMLElement>('.chat-scroll-viewport')!
  const content = container.querySelector<HTMLElement>('.chat-message-list-inner')!
  const row = container.querySelector<HTMLElement>('[data-message-id="answer"]')!
  // Virtualized rows can move/resize while the list's estimated total stays fixed.
  Object.defineProperty(content, 'scrollHeight', { configurable: true, get: () => 1600 })
  Object.defineProperty(viewport, 'scrollHeight', { configurable: true, get: () => 1600 })
  let top = 0
  vi.spyOn(row, 'getBoundingClientRect').mockImplementation(() => ({
    top, bottom: top + 200, height: 200, width: 600, left: 0, right: 600,
  }) as DOMRect)
  return { ready, row, viewport, unmount, rerender, props, move: () => { top += 40 } }
}

describe('MessageList opening mask readiness', () => {
  it('waits for visible row positions, not just total scrollHeight', async () => {
    const { ready, move } = await mount()
    for (let index = 0; index < 8; index += 1) {
      move()
      await frame()
    }
    expect(ready).not.toHaveBeenCalled()
    await settle()
    expect(ready).toHaveBeenCalledOnce()
    expect(ready).toHaveBeenCalledWith('opening-layout', 1)
  })

  it('waits for image loading and the resulting layout to settle', async () => {
    const { ready, row, move } = await mount()
    const image = document.createElement('img')
    let complete = false
    Object.defineProperty(image, 'complete', { get: () => complete })
    row.append(image)
    for (let index = 0; index < 60; index += 1) await frame()
    expect(ready).not.toHaveBeenCalled()
    complete = true
    move()
    fireEvent.load(image)
    await settle()
    expect(ready).toHaveBeenCalledOnce()
  })

  it('reveals moving rows at the deadline even while async content remains pending', async () => {
    const { ready, row, move } = await mount()
    row.setAttribute('data-chat-async-pending', 'true')
    for (let index = 0; index < 140; index += 1) {
      move()
      await frame()
    }
    expect(ready).toHaveBeenCalledOnce()
    row.removeAttribute('data-chat-async-pending')
    await settle()
    expect(ready).toHaveBeenCalledOnce()
  })

  it('waits for scroll compensation even when the content height stays fixed', async () => {
    const { ready, viewport } = await mount()
    for (let index = 0; index < 8; index += 1) {
      viewport.scrollTop += 40
      await frame()
    }
    expect(ready).not.toHaveBeenCalled()
    await settle()
    expect(ready).toHaveBeenCalledOnce()
  })

  it('releases a stalled async placeholder at the deadline', async () => {
    const { ready, row } = await mount()
    row.setAttribute('data-chat-async-pending', 'true')
    await settle()
    expect(ready).not.toHaveBeenCalled()
    for (let index = 0; index < 140; index += 1) await frame()
    expect(ready).toHaveBeenCalledOnce()
    row.removeAttribute('data-chat-async-pending')
    await settle()
    expect(ready).toHaveBeenCalledOnce()
  })

  it('cancels the old readiness callback when switching away before settling', async () => {
    const { ready, unmount } = await mount()
    await frame()
    unmount()
    await act(async () => { await vi.advanceTimersByTimeAsync(3000) })
    expect(ready).not.toHaveBeenCalled()
  })

  it('does not hide an actively generating conversation indefinitely', async () => {
    act(() => setCoarse({ streaming: true }))
    const { ready, row } = await mount()
    for (let index = 0; index < 140; index += 1) {
      // Live tokens can keep mutating DOM even after history has mounted.
      const token = document.createElement('span')
      row.append(token)
      await frame()
      token.remove()
    }
    expect(ready).toHaveBeenCalledOnce()
  })

  it('bounds waiting for fonts that never finish loading', async () => {
    const previous = Object.getOwnPropertyDescriptor(document, 'fonts')
    Object.defineProperty(document, 'fonts', { configurable: true, value: { status: 'loading' } })
    try {
      const { ready } = await mount()
      await act(async () => { await vi.advanceTimersByTimeAsync(1900) })
      expect(ready).not.toHaveBeenCalled()
      await act(async () => { await vi.advanceTimersByTimeAsync(200) })
      expect(ready).toHaveBeenCalledOnce()
    } finally {
      if (previous) Object.defineProperty(document, 'fonts', previous)
      else Reflect.deleteProperty(document, 'fonts')
    }
  })

  it('bounds waiting for repeated history DOM updates', async () => {
    const { ready, row } = await mount()
    for (let index = 0; index < 140; index += 1) {
      const token = document.createElement('span')
      row.append(token)
      await frame()
      token.remove()
    }
    expect(ready).toHaveBeenCalledOnce()
  })

  it('keeps the original deadline when the callback and streaming state change', async () => {
    const { ready, row, props, rerender } = await mount()
    row.setAttribute('data-chat-async-pending', 'true')
    await act(async () => { await vi.advanceTimersByTimeAsync(1500) })
    act(() => setCoarse({ streaming: true }))
    rerender(<MessageList {...props} onInitialRender={(...args) => ready(...args)} />)
    await act(async () => { await vi.advanceTimersByTimeAsync(600) })
    expect(ready).toHaveBeenCalledOnce()
    expect(ready).toHaveBeenCalledWith('opening-layout', 1)
  })

  it('gives a new navigation its own deadline and cancels the previous callback', async () => {
    const { ready, row, props, rerender } = await mount()
    row.setAttribute('data-chat-async-pending', 'true')
    await act(async () => { await vi.advanceTimersByTimeAsync(1500) })
    rerender(<MessageList {...props} renderRequestId={2} />)
    await act(async () => { await vi.advanceTimersByTimeAsync(600) })
    expect(ready).not.toHaveBeenCalled()
    await act(async () => { await vi.advanceTimersByTimeAsync(1500) })
    expect(ready).toHaveBeenCalledOnce()
    expect(ready).toHaveBeenCalledWith('opening-layout', 2)
  })

  it('reveals pending content even if animation frames are suspended', async () => {
    vi.spyOn(window, 'requestAnimationFrame').mockImplementation(() => 0)
    const { ready, row } = await mount()
    row.setAttribute('data-chat-async-pending', 'true')
    await act(async () => { await vi.advanceTimersByTimeAsync(2100) })
    expect(ready).toHaveBeenCalledOnce()
  })

  it('does not emit readiness again after an early reveal or its deadline', async () => {
    const { ready, row } = await mount()
    await settle()
    expect(ready).toHaveBeenCalledOnce()
    row.append(document.createElement('span'))
    await act(async () => { await vi.advanceTimersByTimeAsync(3000) })
    expect(ready).toHaveBeenCalledOnce()
  })

  it('removes the actual loading mask while a mounted image is still pending', async () => {
    function LoadedConversation() {
      const transition = useConversationTransition()
      return <>
        <MessageList conversationId="slow-image" renderRequestId={transition.requestId}
          onInitialRender={completeConversationTransition}
          messages={[{ id: 'answer', role: 'assistant', content: 'History already loaded', timestamp: 1 }]} />
        {transition.loading && <ConversationLoadingState showAnimation />}
      </>
    }
    beginConversationTransition('slow-image')
    const { container } = render(<LoadedConversation />)
    await act(async () => { await Promise.resolve() })
    const row = container.querySelector('[data-message-id="answer"]')!
    expect(row.textContent).toContain('History already loaded')
    const image = document.createElement('img')
    let complete = false
    Object.defineProperty(image, 'complete', { get: () => complete })
    row.append(image)
    await act(async () => { await vi.advanceTimersByTimeAsync(1900) })
    expect(container.querySelector('.chat-conversation-loading')).not.toBeNull()
    await act(async () => { await vi.advanceTimersByTimeAsync(200) })
    expect(container.querySelector('.chat-conversation-loading')).toBeNull()
    await act(async () => { await vi.advanceTimersByTimeAsync(30_000) })
    complete = true
    fireEvent.load(image)
    await settle()
    expect(container.querySelector('.chat-conversation-loading')).toBeNull()
    expect(row.textContent).toContain('History already loaded')
  })
})
