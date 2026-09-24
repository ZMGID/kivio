import { act, fireEvent, render } from '@testing-library/react'
import type { Virtualizer } from '@tanstack/react-virtual'
import { afterEach, expect, it, vi } from 'vitest'
import { MessageList } from './MessageList'
import { clearChatReadingPositions, rememberChatReadingPosition } from './chatReadingPosition'
import { reset } from './streamingStore'

let list: Virtualizer<Element, Element>
vi.mock('@tanstack/react-virtual', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@tanstack/react-virtual')>()
  return {
    ...actual,
    useVirtualizer: (...args: Parameters<typeof actual.useVirtualizer>) => {
      list = actual.useVirtualizer(...args)
      return list
    },
  }
})

afterEach(() => { act(() => reset()); clearChatReadingPositions(); vi.restoreAllMocks() })

it('restores an earlier row when reopening a conversation that ends in a user message', async () => {
  rememberChatReadingPosition('restore-reading', {
    following: false, rowKey: 'short-20', rowOffset: 18,
    scrollTop: 200, layoutKey: 'older-layout',
  })
  const { container } = render(<MessageList conversationId="restore-reading" messages={
    Array.from({ length: 40 }, (_, index) => ({
      id: `short-${index}`, role: 'user' as const, content: 'Short', timestamp: index,
    }))
  } />)
  await act(async () => { await Promise.resolve() })
  const viewport = container.querySelector<HTMLElement>('.chat-scroll-viewport')!
  const index = Array.from({ length: list.options.count }, (_, value) => value)
    .find((value) => String(list.options.getItemKey(value)).endsWith(':short-20'))!
  expect(viewport.scrollTop).toBe((list.measurementsCache[index]?.start ?? list.getOffsetForIndex(index, 'start')?.[0])! + 18)
})

it('falls back to the latest position when the saved row content changed', async () => {
  rememberChatReadingPosition('edited-reading', {
    following: false, rowKey: 'old', rowRevision: 'obsolete',
    rowOffset: 18, scrollTop: 200, layoutKey: 'same-layout',
  })
  const { container } = render(<MessageList conversationId="edited-reading" messages={[
    { id: 'old', role: 'user', content: 'edited', timestamp: 1 },
  ]} />)
  await act(async () => { await Promise.resolve() })
  expect(container.querySelector<HTMLElement>('.chat-scroll-viewport')!.scrollTop).toBe(0)
})

it('uses the lightweight directory to open a turn outside the first window', async () => {
  const focus = vi.fn()
  const { container } = render(<MessageList
    conversationId="directory"
    messages={[{ id: 'recent', role: 'user', content: 'Recent', timestamp: 3 }]}
    historyStart={2}
    historyDirectory={[
      { kind: 'turn', id: 'turn-old', message_id: 'old', message_index: 0, title: 'Old question' },
      { kind: 'compaction', id: 'compaction-middle', message_id: 'middle', message_index: 1, title: '已压缩此前上下文' },
      { kind: 'turn', id: 'turn-recent', message_id: 'recent', message_index: 2, title: 'Recent question' },
    ]}
    onFocusHistoryMessage={focus}
    onLoadOlder={vi.fn()}
  />)
  await act(async () => { await Promise.resolve() })
  fireEvent.click(container.querySelector('[data-message-navigator-id="turn-old"]')!)
  expect(focus).toHaveBeenCalledWith('directory', 'old')
  fireEvent.click(container.querySelector('[data-message-navigator-id="compaction-middle"]')!)
  expect(focus).toHaveBeenCalledWith('directory', 'compaction-summary-middle')
})

it('keeps the visible row anchored when an older page is prepended', async () => {
  const older = Array.from({ length: 2 }, (_, index) => ({
    id: `old-${index}`, role: 'user' as const, content: 'Old', timestamp: index,
  }))
  const recent = Array.from({ length: 8 }, (_, index) => ({
    id: `recent-${index}`, role: 'user' as const, content: 'Recent', timestamp: index + 2,
  }))
  const load = vi.fn()
  const { container, rerender } = render(<MessageList conversationId="page-anchor"
    messages={recent} historyStart={2} onLoadOlder={load} />)
  await act(async () => { await Promise.resolve() })
  const viewport = container.querySelector<HTMLElement>('.chat-scroll-viewport')!
  const originalStart = list.measurementsCache.find((item) => String(list.options.getItemKey(item.index)).endsWith(':recent-0'))!.start
  fireEvent.click([...container.querySelectorAll('button')].find((button) => button.textContent === '加载更早消息')!)
  expect(load).toHaveBeenCalledOnce()
  rerender(<MessageList conversationId="page-anchor" messages={[...older, ...recent]}
    historyStart={0} onLoadOlder={load} />)
  await act(async () => { await Promise.resolve() })
  const index = Array.from({ length: list.options.count }, (_, value) => value)
    .find((value) => String(list.options.getItemKey(value)).endsWith(':recent-0'))!
  expect(viewport.scrollTop).toBe(list.measurementsCache[index].start - originalStart)
})

it('keeps a detached reader on the same row when background history hydration prepends messages', async () => {
  const recent = Array.from({ length: 8 }, (_, index) => ({
    id: `recent-${index}`, role: 'user' as const, content: 'Recent', timestamp: index + 2,
  }))
  const older = Array.from({ length: 2 }, (_, index) => ({
    id: `old-${index}`, role: 'user' as const, content: 'Old', timestamp: index,
  }))
  const { container, rerender } = render(<MessageList conversationId="background-anchor"
    messages={recent} historyStart={2} onLoadOlder={vi.fn()} />)
  await act(async () => { await Promise.resolve() })
  const viewport = container.querySelector<HTMLElement>('.chat-scroll-viewport')!
  const original = list.measurementsCache.find((item) => String(list.options.getItemKey(item.index)).endsWith(':recent-3'))!
  act(() => {
    fireEvent.wheel(viewport, { deltaY: -100 })
    viewport.scrollTop = original.start + 12
    fireEvent.scroll(viewport)
  })
  rerender(<MessageList conversationId="background-anchor" messages={[...older, ...recent]}
    historyStart={0} onLoadOlder={vi.fn()} />)
  await act(async () => { await Promise.resolve() })
  const index = Array.from({ length: list.options.count }, (_, value) => value)
    .find((value) => String(list.options.getItemKey(value)).endsWith(':recent-3'))!
  expect(viewport.scrollTop).toBe(list.measurementsCache[index].start + 12)
})

it('shows a failed history page request and lets the reader retry', async () => {
  const load = vi.fn()
  const props = {
    conversationId: 'page-error',
    messages: [{ id: 'recent', role: 'user' as const, content: 'Recent', timestamp: 2 }],
    historyStart: 2,
    onLoadOlder: load,
  }
  const { rerender } = render(<MessageList {...props} />)
  fireEvent.click([...document.querySelectorAll('button')].find((button) => button.textContent === '加载更早消息')!)
  expect(load).toHaveBeenCalledOnce()
  rerender(<MessageList {...props} historyLoadError="加载更早消息失败，请重试。" />)
  expect(document.querySelector('[role="alert"]')).toHaveTextContent('加载更早消息失败，请重试。')
  fireEvent.click([...document.querySelectorAll('button')].find((button) => button.textContent === '加载更早消息')!)
  expect(load).toHaveBeenCalledTimes(2)
})

it('mounts a distant scroll range before the scroll delivery can paint', async () => {
  const { container } = render(<MessageList conversationId="fast-scroll-range" messages={
    Array.from({ length: 60 }, (_, index) => ({
      id: `short-${index}`, role: 'user' as const, content: 'Short', timestamp: index,
    }))
  } />)
  await act(async () => { await Promise.resolve() })
  const viewport = container.querySelector<HTMLElement>('.chat-scroll-viewport')!
  fireEvent.wheel(viewport, { deltaY: -100 })
  const target = 30
  expect(container.querySelector(`[data-chat-row-index="${target}"]`)).toBeNull()
  act(() => {
    list.scrollOffset = list.measurementsCache[target].start
    viewport.scrollTop = list.scrollOffset
    // TanStack's scroll observer requests a synchronous range commit.
    list.options.onChange(list, true)
    expect(container.querySelector(`[data-chat-row-index="${target}"]`)).not.toBeNull()
  })
})

it('bounds offscreen rendering by viewport height while keeping short-message overscan', async () => {
  const { unmount } = render(<MessageList conversationId="long-row-budget" messages={
    Array.from({ length: 40 }, (_, index) => ({
      id: `answer-${index}`, role: 'assistant' as const,
      content: 'A long line of historical content\n'.repeat(400), timestamp: index,
    }))
  } />)
  await act(async () => { await Promise.resolve() })
  const range = { startIndex: 20, endIndex: 20, overscan: 6, count: 41 }
  expect(list.options.rangeExtractor(range)).toEqual([19, 20, 21])
  unmount()
  render(<MessageList conversationId="short-row-budget" messages={
    Array.from({ length: 40 }, (_, index) => ({
      id: `short-${index}`, role: 'user' as const, content: 'Short', timestamp: index,
    }))
  } />)
  await act(async () => { await Promise.resolve() })
  expect(list.options.rangeExtractor(range)).toEqual(Array.from({ length: 13 }, (_, index) => index + 14))
})

it('commits row positions in the same delivery as scroll compensation above the reader', async () => {
  const { container } = render(<MessageList conversationId="scroll-compensation" messages={[
    { id: 'question', role: 'user', content: 'Question', timestamp: 1 },
    { id: 'answer', role: 'assistant', content: 'Long answer', timestamp: 2 },
    { id: 'next', role: 'user', content: 'Next question', timestamp: 3 },
  ]} />)
  await act(async () => { await Promise.resolve() })
  const viewport = container.querySelector<HTMLElement>('.chat-scroll-viewport')!
  Object.defineProperty(viewport, 'scrollHeight', { configurable: true, value: 3000 })
  Object.defineProperty(viewport, 'clientHeight', { configurable: true, value: 800 })
  fireEvent.wheel(viewport, { deltaY: -100 })
  act(() => {
    list.resizeItem(1, 200)
    list.resizeItem(2, 1000)
  })
  viewport.scrollTop = 600
  list.scrollOffset = 600
  list.scrollAdjustments = 0
  const row = container.querySelector<HTMLElement>('[data-chat-row-index="2"]')!
  const previousRow = container.querySelector<HTMLElement>('[data-chat-row-index="1"]')!
  const measure = (height: number) => {
    const entry = { target: previousRow, borderBoxSize: [{ blockSize: height, inlineSize: 280 }] } as unknown as ResizeObserverEntry
    list.resizeItem(1, list.options.measureElement(previousRow, entry, list))
  }
  const start = list.getVirtualItems().find(item => item.index === 2)!.start
  act(() => {
    // The real ResizeObserver path calls resizeItem. Inspect before act's
    // end-of-delivery flush, just as a browser paint can observe the DOM.
    measure(350)
    expect(viewport.scrollTop).toBe(750)
    expect(row.style.transform).toBe(`translateY(${start + 150}px)`)
    measure(180)
    expect(viewport.scrollTop).toBe(580)
    expect(row.style.transform).toBe(`translateY(${start - 20}px)`)
  })
})
