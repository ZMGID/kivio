import { render } from '../test/renderWithNotices'
import { act, fireEvent } from '@testing-library/react'
import type { Virtualizer } from '@tanstack/react-virtual'
import { afterEach, expect, it, vi } from 'vitest'
import { MessageList } from './MessageList'
import { chatApi } from './api'
import { clearChatReadingPositions, recallChatReadingPosition, rememberChatReadingPosition } from './chatReadingPosition'
import { reset, setCoarse, setSnapshot } from './streamingStore'
import { createEmptyStreamSnapshot } from './conversationRuns'
import type { ChatMessage } from './types'

let list: Virtualizer<Element, Element>
vi.mock('@tanstack/react-virtual', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@tanstack/react-virtual')>()
  return {
    ...actual,
    useVirtualizer: (...args: Parameters<typeof actual.useVirtualizer>) => {
      const instance = actual.useVirtualizer(...args)
      if (args[0].getScrollElement()?.classList.contains('chat-scroll-viewport')) list = instance
      return instance
    },
  }
})

afterEach(() => { act(() => reset()); clearChatReadingPositions(); vi.restoreAllMocks() })

it('restores a streamed answer by its message identity after switching away', async () => {
  const answer: ChatMessage = { id: 'reading-answer', role: 'assistant', content: 'Finished answer', timestamp: 2 }
  const history: ChatMessage[] = [{ id: 'reading-question', role: 'user', content: 'Question', timestamp: 1 }, answer,
    ...Array.from({ length: 30 }, (_, index): ChatMessage => ({ id: `later-${index}`, role: 'user', content: 'Later', timestamp: index + 3 }))]
  act(() => {
    setSnapshot({ ...createEmptyStreamSnapshot(), messageId: answer.id, content: answer.content, streaming: true })
    setCoarse({ streaming: true })
  })
  const { container, rerender } = render(<MessageList conversationId="reading-a" messages={history} />)
  await act(async () => {})
  act(() => reset())
  await act(async () => {})
  const viewport = container.querySelector<HTMLElement>('.chat-scroll-viewport')!
  Object.defineProperty(viewport, 'scrollHeight', { configurable: true, value: 5000 })
  Object.defineProperty(viewport, 'clientHeight', { configurable: true, value: 800 })
  fireEvent.wheel(viewport, { deltaY: -100 })
  viewport.scrollTop = list.getVirtualItems().find(item => item.index === 2)!.start + 10
  list.scrollOffset = viewport.scrollTop
  const before = viewport.scrollTop
  rerender(<MessageList conversationId="reading-b" messages={[]} />)
  await act(async () => {})
  expect(recallChatReadingPosition('reading-a')?.following).toBe(false)
  rerender(<MessageList conversationId="reading-a" messages={history} />)
  await act(async () => {})
  expect(viewport.scrollTop).toBe(before)
})

it.each(['wheel', 'pointerdown', 'touchstart', 'keydown'])(
  'holds a restored row through late measurements and yields to %s', async gesture => {
    const messages: ChatMessage[] = Array.from({ length: 40 }, (_, index) => ({
      id: `restore-${index}`, role: 'user', content: 'Short', timestamp: index,
    }))
    rememberChatReadingPosition('settling-restore', {
      following: false, rowKey: 'restore-10', rowOffset: 10, scrollTop: 0, layoutKey: 'old-width',
    })
    const { container } = render(<MessageList conversationId="settling-restore" messages={messages} />)
    await act(async () => {})
    const viewport = container.querySelector<HTMLElement>('.chat-scroll-viewport')!
    // Earlier offscreen rows change size after the estimated initial jump.
    act(() => { for (let index = 1; index < 11; index++) list.resizeItem(index, 250) })
    await act(async () => { await new Promise(resolve => requestAnimationFrame(resolve)) })
    expect(viewport.scrollTop).toBe(list.measurementsCache[11].start + 10)
    if (gesture === 'wheel') fireEvent.wheel(viewport, { deltaY: -100 })
    else if (gesture === 'keydown') fireEvent.keyDown(viewport, { key: 'PageUp' })
    else if (gesture === 'touchstart') fireEvent.touchStart(viewport, { touches: [{ clientY: 100 }] })
    else fireEvent.pointerDown(viewport)
    viewport.scrollTop = 50
    await act(async () => { await new Promise(resolve => requestAnimationFrame(resolve)) })
    expect(viewport.scrollTop).toBe(50)
  },
)

it('renders a cross-window image from supplied history dependencies', async () => {
  const { container } = render(<MessageList conversationId="window-image" historyStart={70}
    messages={[{ id: 'tail-reference', role: 'assistant', timestamp: 1, content: '![earlier](artifact:art_early)' }]}
    historyArtifacts={[{ id: 'art_early', name: 'early.png', mime_type: 'image/png', data_url: 'data:image/png;base64,AAAA' }]} />)
  await act(async () => { await Promise.resolve() })
  expect(container.textContent).not.toContain('文件不可用')
  expect(container.querySelector('img')).toHaveAttribute('src', 'data:image/png;base64,AAAA')
})

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

it('keeps the reading position when a saved inline tool result reopens on demand', async () => {
  const rows: ChatMessage[] = Array.from({ length: 35 }, (_, i) => ({
    id: `tool-history-${i}`, role: 'user', content: 'Question', timestamp: i,
  }))
  const tool = { id: 'read', name: 'Read', source: 'native', status: 'success' as const, arguments: '{"path":"file.txt"}' }
  const answer: ChatMessage = {
    ...rows[2], role: 'assistant', content: 'Result', tool_calls: [{ ...tool, result_preview: 'x'.repeat(5000) }],
  }
  rows[2] = answer
  const conversationId = 'tool-reading'
  const { container, rerender } = render(<MessageList conversationId={conversationId} messages={rows} />)
  await act(async () => {})
  const viewport = container.querySelector<HTMLElement>('.chat-scroll-viewport')!
  Object.defineProperty(viewport, 'scrollHeight', { configurable: true, value: 5000 })
  Object.defineProperty(viewport, 'clientHeight', { configurable: true, value: 800 })
  fireEvent.wheel(viewport, { deltaY: -100 })
  const row = () => list.measurementsCache.find(item => String(item.key).endsWith(`:${answer.id}`))!
  viewport.scrollTop = row().start + 12
  list.scrollOffset = viewport.scrollTop
  rerender(<MessageList conversationId="other" messages={[]} />)
  await act(async () => {})
  expect(recallChatReadingPosition(conversationId)?.rowKey).toBe(answer.id)
  const deferredToolResult = { conversationId, messageId: answer.id, toolCallId: tool.id, expectedHash: 'digest' }
  rerender(<MessageList conversationId={conversationId} messages={rows.map(message =>
    message === answer ? { ...message, tool_calls: [{ ...tool, deferredToolResult }] } : message)} />)
  await act(async () => {})
  expect(viewport.scrollTop).toBe(row().start + 12)
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

it('shows a compaction divider only when its display anchor has been loaded', async () => {
  const messages: ChatMessage[] = [
    { id: 'split', role: 'user', content: 'First question', timestamp: 1 },
    { id: 'tail', role: 'user', content: 'Latest question', timestamp: 3 },
  ]
  const contextState = { compaction_boundaries: [{
    id: 'boundary', source_until_message_id: 'split', display_after_message_id: 'anchor',
    token_estimate_before: 100, token_estimate_after: 10, summary_content: 'summary',
    trigger: 'manual' as const, created_at: 2,
  }] }
  const { container, rerender } = render(<MessageList conversationId="compaction-gap"
    messages={messages} contextState={contextState} historyGap={{ start: 1, end: 2 }} />)
  expect(container.querySelector('[data-compaction-divider-id="boundary"]')).toBeNull()
  rerender(<MessageList conversationId="compaction-gap" contextState={contextState}
    messages={[messages[0], { id: 'anchor', role: 'assistant', content: 'Reply', timestamp: 2 }, messages[1]]} />)
  expect(container.querySelector('[data-compaction-divider-id="boundary"]')).toBeInTheDocument()
})

it('uses the lightweight directory to open a turn outside the first window', async () => {
  const focus = vi.fn()
  const fullDirectory = vi.spyOn(chatApi, 'getConversationDirectory').mockResolvedValue({
    revision: 7, history_directory: [
      { kind: 'turn', id: 'turn-old', message_id: 'old', message_index: 0, title: 'Old question' },
      { kind: 'compaction', id: 'compaction-middle', message_id: 'middle', message_index: 1, title: '已压缩此前上下文' },
      { kind: 'turn', id: 'turn-recent', message_id: 'recent', message_index: 2, title: 'Recent question' },
    ],
  })
  const { container } = render(<MessageList
    conversationId="directory"
    messages={[{ id: 'recent', role: 'user', content: 'Recent', timestamp: 3 }]}
    historyStart={2} historyRevision={7} historyTurnCount={2}
    onFocusHistoryMessage={focus} onLoadHistory={vi.fn()}
  />)
  expect(fullDirectory).not.toHaveBeenCalled()
  await act(async () => {
    fireEvent.pointerOver(container.querySelector('.chat-message-navigator')!)
  })
  expect(fullDirectory).toHaveBeenCalledWith('directory')
  fireEvent.click(container.querySelector('[data-message-navigator-id="turn-old"]')!)
  expect(focus).toHaveBeenCalledWith('directory', 'old', expect.any(AbortSignal), 'old')
  const firstSignal = focus.mock.calls[0][2] as AbortSignal
  fireEvent.click(container.querySelector('[data-message-navigator-id="compaction-middle"]')!)
  expect(firstSignal.aborted).toBe(true)
  expect(focus).toHaveBeenCalledWith('directory', 'compaction-summary-middle', expect.any(AbortSignal), 'middle')
  const secondSignal = focus.mock.calls[1][2] as AbortSignal
  fireEvent.wheel(container.querySelector('.chat-scroll-viewport')!, { deltaY: -100 })
  expect(secondSignal.aborted).toBe(true)
})

it('loads the adjacent side of a restored gap as the reader approaches from either direction', async () => {
  const load = vi.fn()
  const messages = Array.from({ length: 48 }, (_, i) => ({
    id: `gap-${i}`, role: 'user' as const, content: 'Question', timestamp: i,
  }))
  const { container } = render(<MessageList conversationId="restored-gap" messages={messages}
    historyStart={0} historyGap={{ start: 24, end: 120 }} onLoadHistory={load} />)
  await act(async () => {})
  const viewport = container.querySelector<HTMLElement>('.chat-scroll-viewport')!
  Object.defineProperty(viewport, 'clientHeight', { configurable: true, value: 400 })
  Object.defineProperty(viewport, 'scrollHeight', { configurable: true, value: 5000 })
  const firstTailRow = list.measurementsCache.find(row => String(list.options.getItemKey(row.index)).endsWith(':gap-24'))!
  expect(list.options.count).toBe(messages.length + 1) // Only the top padding and actual messages.
  // A navigator jump holds the target in place until the reader takes over.
  fireEvent.click(container.querySelector('[data-message-navigator-id="turn-gap-0"]')!)
  for (const [deltaY, edge] of [[320, 'gap-start'], [-320, 'gap-end']] as const) {
    // The same final viewport can straddle the gap after scrolling from either side.
    viewport.scrollTop = firstTailRow.start - 200
    fireEvent.wheel(viewport, { deltaY })
    fireEvent.scroll(viewport)
    await act(async () => {})
    expect(load).toHaveBeenLastCalledWith(edge)
  }
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
    messages={recent} historyStart={2} onLoadHistory={load} />)
  await act(async () => { await Promise.resolve() })
  const viewport = container.querySelector<HTMLElement>('.chat-scroll-viewport')!
  const originalStart = list.measurementsCache.find((item) => String(list.options.getItemKey(item.index)).endsWith(':recent-0'))!.start
  fireEvent.wheel(viewport, { deltaY: -100 })
  fireEvent.scroll(viewport)
  expect(load).toHaveBeenCalledOnce()
  rerender(<MessageList conversationId="page-anchor" messages={[...older, ...recent]}
    historyStart={0} onLoadHistory={load} />)
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
    messages={recent} historyStart={2} onLoadHistory={vi.fn()} />)
  await act(async () => { await Promise.resolve() })
  const viewport = container.querySelector<HTMLElement>('.chat-scroll-viewport')!
  const original = list.measurementsCache.find((item) => String(list.options.getItemKey(item.index)).endsWith(':recent-3'))!
  act(() => {
    fireEvent.wheel(viewport, { deltaY: -100 })
    viewport.scrollTop = original.start + 12
    fireEvent.scroll(viewport)
  })
  rerender(<MessageList conversationId="background-anchor" messages={[...older, ...recent]}
    historyStart={0} onLoadHistory={vi.fn()} />)
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
    onLoadHistory: load,
  }
  const { container, rerender } = render(<MessageList {...props} />)
  await act(async () => { await Promise.resolve() })
  const viewport = container.querySelector<HTMLElement>('.chat-scroll-viewport')!
  fireEvent.wheel(viewport, { deltaY: -100 })
  fireEvent.scroll(viewport)
  expect(load).toHaveBeenCalledOnce()
  rerender(<MessageList {...props} historyLoadError="加载更早消息失败，请重试。" />)
  expect(document.querySelector('[role="alert"]')).toHaveTextContent('加载更早消息失败，请重试。')
  await act(async () => { await Promise.resolve() })
  // At scrollTop=0 another upward wheel must retry even without a scroll event.
  fireEvent.wheel(viewport, { deltaY: -100 })
  expect(load).toHaveBeenCalledTimes(2)
})

it('keeps following after returning to the bottom while history is still loading', async () => {
  const recent = Array.from({ length: 8 }, (_, index) => ({
    id: `follow-${index}`, role: 'user' as const, content: 'Recent', timestamp: index + 2,
  }))
  const { container, rerender } = render(<MessageList conversationId="hydrate-follow"
    messages={recent} historyStart={2} onLoadHistory={vi.fn()} />)
  await act(async () => { await Promise.resolve() })
  const viewport = container.querySelector<HTMLElement>('.chat-scroll-viewport')!
  Object.defineProperty(viewport, 'scrollHeight', { configurable: true, value: 3000 })
  Object.defineProperty(viewport, 'clientHeight', { configurable: true, value: 600 })
  fireEvent.wheel(viewport, { deltaY: -100 })
  viewport.scrollTop = 100
  fireEvent.scroll(viewport)
  fireEvent.click(container.querySelector('[aria-label="回到底部"]')!)
  expect(list.options.anchorTo).toBe('end')
  // Let the temporary bottom hold finish before the delayed read resolves.
  for (let frame = 0; frame < 45; frame += 1) {
    await act(async () => { await new Promise((resolve) => requestAnimationFrame(resolve)) })
  }
  rerender(<MessageList conversationId="hydrate-follow" historyStart={0}
    messages={[{ id: 'older', role: 'user', content: 'Old', timestamp: 1 }, ...recent]} />)
  await act(async () => { await Promise.resolve() })
  expect(list.options.anchorTo).toBe('end')
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
      id: `mixed-${index}`, role: 'user' as const,
      content: index === 29 ? 'Historical detail\n'.repeat(400) : 'Short', timestamp: index,
    }))
  } />)
  await act(async () => { await Promise.resolve() })
  expect(list.options.rangeExtractor(range)).toEqual(Array.from({ length: 13 }, (_, index) => index + 14))
  // The leading spacer places the large message at row 30. A short neighbor
  // near either edge shouldn't bring the large offscreen message into the DOM.
  for (const index of [28, 32]) {
    expect(list.options.rangeExtractor({ ...range, startIndex: index, endIndex: index })).not.toContain(30)
  }
})

it('keeps nearby rows mounted when a hidden answer tab contains a long report', async () => {
  const messages: ChatMessage[] = Array.from({ length: 40 }, (_, i) => ({
    id: 'question-' + i, role: 'user', content: 'Short', timestamp: i,
  }))
  messages.splice(19, 1,
    { id: 'short-answer', role: 'assistant', group_id: 'compare', content: 'Summary', timestamp: 19 },
    { id: 'long-answer', role: 'assistant', group_id: 'compare', content: 'Report detail\n'.repeat(400), timestamp: 19 },
  )
  const props = { conversationId: 'group-overscan', messages }
  const { rerender } = render(<MessageList {...props} groupSelections={{ compare: 'short-answer' }} />)
  await act(async () => {})
  const range = { startIndex: 17, endIndex: 17, overscan: 6, count: list.options.count }
  expect(list.options.rangeExtractor(range)).toContain(21)
  rerender(<MessageList {...props} groupSelections={{ compare: 'long-answer' }} />)
  await act(async () => {})
  expect(list.options.rangeExtractor(range)).not.toContain(21)
})

it('corrects a short cache when content grows without a ResizeObserver entry', async () => {
  const { container } = render(<MessageList conversationId="stale-row-height" messages={[
    { id: 'question', role: 'user', content: 'Question', timestamp: 1 },
    { id: 'answer', role: 'assistant', content: '```ts\nconst table = true\n```\n| a | b |\n| --- | --- |\n| 1 | 2 |', timestamp: 2 },
    { id: 'next', role: 'user', content: 'Next question', timestamp: 3 },
  ]} />)
  await act(async () => { await Promise.resolve() })
  const row = container.querySelector<HTMLElement>('[data-chat-row-index="2"]')!
  const index = 2
  Object.defineProperty(row, 'offsetHeight', { configurable: true, get: () => 480 })
  // The stale 120 must not survive its own commit: the per-commit pass reads
  // offsetHeight before paint, without waiting for a ResizeObserver entry.
  act(() => { list.resizeItem(index, 120) })
  expect(list.itemSizeCache.get(list.options.getItemKey(index))).toBe(480)
  const items = list.getVirtualItems()
  const start = items.find((item) => item.index === index)!.start
  expect(items.find((item) => item.index === index + 1)!.start).toBe(start + 480)
  let measured = 0
  act(() => {
    measured = list.options.measureElement(row as HTMLDivElement, undefined, list)
  })
  expect(measured).toBe(480)
})

it('does not recurse before paint when a row height never settles', async () => {
  const { container, rerender } = render(<MessageList conversationId="unsettled-row" messages={[
    { id: 'question', role: 'user', content: 'Question', timestamp: 1 },
    { id: 'answer', role: 'assistant', content: 'Answer', timestamp: 2 },
  ]} />)
  await act(async () => { await Promise.resolve() })
  const row = container.querySelector<HTMLElement>('[data-chat-row-index="2"]')!
  // Layout feedback (e.g. a scrollbar toggling the width): every read disagrees with the cache.
  Object.defineProperty(row, 'offsetHeight', { configurable: true,
    get: () => list.itemSizeCache.get(list.options.getItemKey(2)) === 200 ? 300 : 200 })
  expect(() => rerender(<MessageList conversationId="unsettled-row" messages={[
    { id: 'question', role: 'user', content: 'Question', timestamp: 1 },
    { id: 'answer', role: 'assistant', content: 'Answer', timestamp: 2 },
  ]} />)).not.toThrow()
})

it('keeps a measured row when a ref read happens before layout', async () => {
  const { container } = render(<MessageList conversationId="unlaid-row-height" messages={[
    { id: 'question', role: 'user', content: 'Question', timestamp: 1 },
    { id: 'answer', role: 'assistant', content: 'Answer', timestamp: 2 },
  ]} />)
  await act(async () => { await Promise.resolve() })
  const row = container.querySelector<HTMLElement>('[data-chat-row-index="1"]')!
  Object.defineProperty(row, 'offsetHeight', { configurable: true, get: () => 0 })
  act(() => { list.resizeItem(1, 300) })
  let measured = 0
  act(() => {
    measured = list.options.measureElement(row as HTMLDivElement, undefined, list)
  })
  expect(measured).toBe(300)
  expect(list.itemSizeCache.get(list.options.getItemKey(1))).toBe(300)
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
    expect(row.style.top).toBe(`${start + 150}px`)
    measure(180)
    expect(viewport.scrollTop).toBe(580)
    expect(row.style.top).toBe(`${start - 20}px`)
  })
})

it.each(['auto-prefetch', undefined])('prefetches within two viewports, coalesces pending reads, and stops at the oldest page (%s)', async (conversationId) => {
  let complete!: () => void
  const load = vi.fn(() => new Promise<void>((resolve) => { complete = resolve }))
  const props = {
    conversationId, historyStart: 60, onLoadHistory: load,
    messages: Array.from({ length: 40 }, (_, index) => ({
      id: `prefetch-${index}`, role: 'user' as const, content: 'Message', timestamp: index,
    })),
  }
  const { container, rerender } = render(<MessageList {...props} />)
  await act(async () => { await Promise.resolve() })
  const viewport = container.querySelector<HTMLElement>('.chat-scroll-viewport')!
  Object.defineProperty(viewport, 'clientHeight', { configurable: true, value: 600 })
  Object.defineProperty(viewport, 'scrollHeight', { configurable: true, value: 6000 })
  expect(container.textContent).not.toContain('加载更早消息')
  expect(load).not.toHaveBeenCalled()
  viewport.scrollTop = 1201
  fireEvent.wheel(viewport, { deltaY: -100 })
  fireEvent.scroll(viewport)
  expect(load).not.toHaveBeenCalled()
  viewport.scrollTop = 1200
  fireEvent.scroll(viewport)
  expect(load).toHaveBeenCalledOnce()
  viewport.scrollTop = 900
  fireEvent.scroll(viewport)
  fireEvent.wheel(viewport, { deltaY: -100 })
  expect(load).toHaveBeenCalledOnce()
  await act(async () => { complete(); await Promise.resolve() })
  viewport.scrollTop = 600
  fireEvent.scroll(viewport)
  expect(load).toHaveBeenCalledTimes(2)
  await act(async () => { complete(); await Promise.resolve() })
  rerender(<MessageList {...props} historyStart={0} />)
  viewport.scrollTop = 0
  fireEvent.scroll(viewport)
  fireEvent.wheel(viewport, { deltaY: -100 })
  expect(load).toHaveBeenCalledTimes(2)
})


it.each([0, 40])('keeps reserve independent of mounted history and clears it on rewind (%i older rows)', async (olderCount) => {
  const earlier = [
    ...Array.from({ length: olderCount }, (_, index) => ({
      id: `older-${index}`, role: 'assistant' as const, content: 'Older answer', timestamp: index,
    })),
    { id: 'old-question', role: 'user' as const, content: 'Earlier question', timestamp: 1 },
    { id: 'old-answer', role: 'assistant' as const, content: 'Earlier answer', timestamp: 2 },
  ]
  const question = { id: 'new-question', role: 'user' as const, content: 'New question', timestamp: 3 }
  const { container, rerender } = render(<MessageList conversationId="reserve-rewind" messages={earlier} />)
  await act(async () => { await Promise.resolve() })
  const viewport = container.querySelector<HTMLElement>('.chat-scroll-viewport')!
  Object.defineProperty(viewport, 'clientHeight', { configurable: true, value: 800 })
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    const top = this.dataset.messageId ? 0 : 200
    return { top, bottom: top + 100, height: 100, width: 280, left: 0, right: 280, x: 0, y: top, toJSON() {} }
  })
  act(() => setCoarse({ streaming: true }))
  rerender(<MessageList conversationId="reserve-rewind" messages={[...earlier, question]} />)
  act(() => setCoarse({ streaming: false }))
  rerender(<MessageList conversationId="reserve-rewind" messages={[...earlier, question,
    { id: 'new-answer', role: 'assistant', content: 'Short answer', timestamp: 4 },
  ]} />)
  const spacer = container.querySelector<HTMLElement>('[data-chat-message-list-item="tail"] > div > div:last-child')!
  expect(parseFloat(spacer.style.height)).toBeGreaterThan(16)
  rerender(<MessageList conversationId="reserve-rewind" messages={earlier} />)
  expect(spacer.style.height).toBe('16px')
})

it('reuses the list across conversations and restores each one\'s own reading position', async () => {
  const [rowsA, rowsB] = ['a', 'b'].map((prefix) => Array.from({ length: 40 }, (_, index) => ({
    id: `${prefix}-${index}`, role: 'user' as const, content: 'Short', timestamp: index,
  })))
  const startOf = (key: string) => {
    const index = Array.from({ length: list.options.count }, (_, value) => value)
      .find((value) => String(list.options.getItemKey(value)).endsWith(`:${key}`))!
    return (list.measurementsCache[index]?.start ?? list.getOffsetForIndex(index, 'start')?.[0])!
  }
  rememberChatReadingPosition('reuse-b', { following: false, rowKey: 'b-10', rowOffset: 5, scrollTop: 0, layoutKey: 'x' })
  const { container, rerender } = render(<MessageList conversationId="reuse-a" messages={rowsA} />)
  await act(async () => { await Promise.resolve() })
  const viewport = container.querySelector<HTMLElement>('.chat-scroll-viewport')!
  rerender(<MessageList conversationId="reuse-b" messages={rowsB} />)
  await act(async () => { await Promise.resolve() })
  // Same DOM node: no unmount/remount flash between conversations.
  expect(container.querySelector('.chat-scroll-viewport')).toBe(viewport)
  expect(container.querySelector('[data-message-id^="a-"]')).toBeNull()
  expect(viewport.scrollTop).toBe(startOf('b-10') + 5)
})

it('saves the outgoing conversation with its measured rows and restores them on return', async () => {
  const [rowsA, rowsB] = ['a', 'b'].map((prefix) => Array.from({ length: 40 }, (_, index) => ({
    id: `${prefix}-${index}`, role: 'user' as const, content: 'Short', timestamp: index,
  })))
  const { container, rerender } = render(<MessageList conversationId="measured-b" messages={rowsB} />)
  await act(async () => { await Promise.resolve() })
  const viewport = container.querySelector<HTMLElement>('.chat-scroll-viewport')!
  // Real heights differ from estimates; B's restore must use them, not a fresh estimate.
  act(() => { for (let index = 0; index < rowsB.length; index += 1) list.resizeItem(index, 300) })
  // A follows the bottom, which drives TanStack's end-anchor path during the switch render.
  rerender(<MessageList conversationId="measured-a" messages={rowsA} />)
  await act(async () => { await Promise.resolve() })
  rememberChatReadingPosition('measured-b', { following: false, rowKey: 'b-10', rowOffset: 5, scrollTop: 0, layoutKey: 'x' })
  rerender(<MessageList conversationId="measured-b" messages={rowsB} />)
  await act(async () => { await Promise.resolve() })
  const index = Array.from({ length: list.options.count }, (_, value) => value)
    .find((value) => String(list.options.getItemKey(value)).endsWith(':b-10'))!
  // Measured 300px rows, not ~88px estimates: the snapshot survived the switch away and back.
  expect(list.measurementsCache[index].size).toBe(300)
  expect(viewport.scrollTop).toBe(list.measurementsCache[index].start + 5)
})

it('does not pull a detached conversation to the bottom when leaving a streaming one', async () => {
  const [rowsA, rowsB] = ['a', 'b'].map((prefix) => Array.from({ length: 40 }, (_, index) => ({
    id: `${prefix}-${index}`, role: 'user' as const, content: 'Short', timestamp: index,
  })))
  rememberChatReadingPosition('detached-b', { following: false, rowKey: 'b-10', rowOffset: 5, scrollTop: 0, layoutKey: 'x' })
  act(() => setCoarse({ streaming: true }))
  const { container, rerender } = render(<MessageList conversationId="live-a" messages={rowsA} />)
  await act(async () => { await Promise.resolve() })
  const viewport = container.querySelector<HTMLElement>('.chat-scroll-viewport')!
  act(() => {
    setCoarse({ streaming: false })
    rerender(<MessageList conversationId="detached-b" messages={rowsB} />)
  })
  await act(async () => { await Promise.resolve() })
  const index = Array.from({ length: list.options.count }, (_, value) => value)
    .find((value) => String(list.options.getItemKey(value)).endsWith(':b-10'))!
  expect(viewport.scrollTop).toBe((list.measurementsCache[index]?.start ?? list.getOffsetForIndex(index, 'start')?.[0])! + 5)
})
