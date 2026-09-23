import { act, fireEvent, render } from '@testing-library/react'
import type { Virtualizer } from '@tanstack/react-virtual'
import { afterEach, expect, it, vi } from 'vitest'
import { MessageList } from './MessageList'
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

afterEach(() => { act(() => reset()); vi.restoreAllMocks() })

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
