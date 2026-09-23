import { act, fireEvent, render, screen, within } from '@testing-library/react'
import { renderToString } from 'react-dom/server'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { ReasoningBlock } from './ReasoningBlock'

afterEach(() => { vi.unstubAllGlobals(); vi.useRealTimers() })

describe('ReasoningBlock', () => {
  it('renders only the latest nonempty line in the single-line preview', () => {
    const { rerender } = render(<ReasoningBlock reasoning={'older line\r\nlatest line\r\n  \t'} streaming />)
    const preview = screen.getByTestId('reasoning-preview')
    expect(preview).toHaveTextContent('latest line')
    expect(screen.queryByText('older line')).not.toBeInTheDocument()
    expect(screen.queryByTestId('reasoning-scroll')).not.toBeInTheDocument()
    expect(screen.getByRole('button')).toHaveAttribute('aria-expanded', 'false')
    rerender(<ReasoningBlock reasoning={'older line\r\nlatest line\nnext line'} streaming />)
    expect(preview).toHaveTextContent('next line')
    rerender(<ReasoningBlock reasoning={'older line\r\nlatest line\nnext line'} />)
    expect(screen.getByTestId('reasoning-preview')).toBe(preview)
    expect(preview).toHaveTextContent('next line')
  })

  it('opens the full thought with one click and does not close it on completion', () => {
    const { rerender } = render(<ReasoningBlock reasoning={'first line\nlatest line'} streaming />)
    fireEvent.click(screen.getByRole('button'))
    expect(screen.getByRole('button')).toHaveAttribute('aria-expanded', 'true')
    expect(screen.getByTestId('reasoning-text')).toHaveTextContent('first line')
    expect(screen.queryByTestId('reasoning-preview')).not.toBeInTheDocument()
    rerender(<ReasoningBlock reasoning={'first line\nlatest line'} />)
    expect(screen.getByRole('button')).toHaveAttribute('aria-expanded', 'true')
    fireEvent.click(screen.getByRole('button'))
    expect(screen.getByTestId('reasoning-preview')).toHaveTextContent('latest line')
    expect(screen.queryByTestId('reasoning-scroll')).not.toBeInTheDocument()
  })

  it('follows the horizontal end on content and viewport resizing without mounting history', () => {
    let resize: ResizeObserverCallback = () => {}
    const disconnect = vi.fn()
    vi.stubGlobal('ResizeObserver', class {
      constructor(callback: ResizeObserverCallback) { resize = callback }
      observe() {}
      disconnect = disconnect
    })
    const { rerender, unmount } = render(<ReasoningBlock reasoning={'history\nlatest'} streaming />)
    const preview = screen.getByTestId('reasoning-preview')
    let width = 500
    let viewportWidth = 100
    Object.defineProperties(preview, {
      scrollWidth: { get: () => width },
      clientWidth: { get: () => viewportWidth },
    })
    const notify = () => act(() => resize([], {} as ResizeObserver))
    notify()
    expect(preview.scrollLeft).toBe(400)
    rerender(<ReasoningBlock reasoning={'history\nlatest grows'} streaming />)
    width = 550
    notify()
    expect(preview.scrollLeft).toBe(450)
    viewportWidth = 200
    notify()
    expect(preview.scrollLeft).toBe(350)
    rerender(<ReasoningBlock reasoning={'history\nlatest grows\nnext'} />)
    width = 80
    notify()
    expect(preview.scrollLeft).toBe(0)
    expect(screen.queryByTestId('reasoning-text')).not.toBeInTheDocument()
    unmount()
    expect(disconnect).toHaveBeenCalled()
  })

  it('renders a completed collapsed body at zero height on the first paint', () => {
    const html = renderToString(<ReasoningBlock reasoning="hidden until expanded" />)
    expect(html).toContain('aria-hidden="true"')
    expect(html).toContain('height:0')
  })

  it('renders section shell without a scroll body for empty reasoning', () => {
    render(<ReasoningBlock reasoning="" />)
    const section = screen.getByLabelText('Thinking')
    expect(within(section).queryByTestId('reasoning-scroll')).not.toBeInTheDocument()
  })

  it('shows the thinking title and the newest line without mounting full text', () => {
    render(<ReasoningBlock reasoning={'line one\nline two\nline three\nline four'} streaming />)
    const section = screen.getByLabelText('Thinking')
    expect(within(section).getByRole('button', { name: /Thinking/i })).toBeInTheDocument()
    expect(within(section).getByTestId('reasoning-preview')).toHaveTextContent('line four')
    expect(within(section).queryByTestId('reasoning-text')).not.toBeInTheDocument()
  })

  it('leaves max-height unset while streaming so growth never animates', () => {
    // 流式中若给 body 写 max-height，CSS 过渡会让内容高度逐帧变化（实测一次收起 18 帧），
    // 消息区的 ResizeObserver 钉底会逐帧重写 scrollTop —— 表现为整屏抖动。
    const { rerender, container } = render(<ReasoningBlock reasoning="alpha" streaming />)
    const body = container.querySelector('[data-chat-disclosure-body]') as HTMLElement
    expect(body.style.maxHeight).toBe('')

    rerender(<ReasoningBlock reasoning={'alpha\nbeta\ngamma'} streaming />)
    expect(body.style.maxHeight).toBe('')
    expect(container.querySelector('.reasoning-stream-tail')).toBeNull()
  })

  it('renders markdown and code fences as plain thinking text', () => {
    render(<ReasoningBlock reasoning={'Before\n```ts\nconst x = 1\n```\nAfter'} streaming />)
    fireEvent.click(screen.getByRole('button'))
    const section = screen.getByLabelText('Thinking')
    expect(section.querySelector('pre')).toBeNull()
    expect(section.querySelector('code')).toBeNull()
    expect(within(section).getByTestId('reasoning-text')).toHaveTextContent('```ts')
  })

  it('shows thinking duration beside the title when provided', () => {
    render(<ReasoningBlock reasoning="alpha" durationMs={65000} />)
    const section = screen.getByLabelText('Thinking')
    expect(within(section).getByRole('button', { name: /Thought/i })).toHaveTextContent('1m 5s')
  })

  it('expands full reasoning after toggle', async () => {
    const user = userEvent.setup()
    const reasoning = 'alpha\nbeta\ngamma'
    render(<ReasoningBlock reasoning={reasoning} />)
    const section = screen.getByLabelText('Thinking')
    await user.click(within(section).getByRole('button', { name: /Thought/i }))
    const text = within(section).getByTestId('reasoning-text')
    expect(within(section).getByTestId('reasoning-scroll')).toBeVisible()
    expect(text.textContent).toContain('alpha')
    expect(text.textContent).toContain('beta')
    expect(text.textContent).toContain('gamma')
  })

  it('keeps the same preview mounted and visible when streaming completes', () => {
    const { rerender } = render(<ReasoningBlock reasoning="done thinking" streaming />)
    const preview = screen.getByTestId('reasoning-preview')
    rerender(<ReasoningBlock reasoning="done thinking" streaming={false} />)
    expect(screen.getByTestId('reasoning-preview')).toBe(preview)
    expect(preview).toBeVisible()
    expect(screen.queryByTestId('reasoning-scroll')).not.toBeInTheDocument()
  })

  it('honors a manual collapse during streaming and after completion', () => {
    const { rerender } = render(<ReasoningBlock reasoning="alpha" streaming />)
    fireEvent.click(screen.getByRole('button'))
    fireEvent.click(screen.getByRole('button'))
    expect(screen.getByRole('button')).toHaveAttribute('aria-expanded', 'false')
    rerender(<ReasoningBlock reasoning="alpha beta" streaming />)
    expect(screen.getByRole('button')).toHaveAttribute('aria-expanded', 'false')
    rerender(<ReasoningBlock reasoning="alpha beta" />)
    expect(screen.getByRole('button')).toHaveAttribute('aria-expanded', 'false')
    fireEvent.click(screen.getByRole('button'))
    expect(screen.getByTestId('reasoning-scroll')).toBeVisible()
  })

  it('keeps measured duration after stopping when no server duration is available', () => {
    vi.useFakeTimers()
    const { rerender } = render(<ReasoningBlock reasoning="alpha" streaming />)
    act(() => vi.advanceTimersByTime(3000))
    rerender(<ReasoningBlock reasoning="alpha" />)
    expect(screen.getByRole('button')).toHaveTextContent('3s')
    act(() => vi.advanceTimersByTime(3000))
    expect(screen.getByRole('button')).toHaveTextContent('3s')
  })

  it('follows content growth, pauses for reading, and resumes only at the bottom', () => {
    let resize: ResizeObserverCallback = () => {}
    let target: Element
    const disconnect = vi.fn()
    vi.stubGlobal('ResizeObserver', class {
      constructor(callback: ResizeObserverCallback) { resize = callback }
      observe(element: Element) { target = element }
      disconnect = disconnect
    })
    const { rerender, unmount } = render(<ReasoningBlock reasoning="alpha" streaming />)
    fireEvent.click(screen.getByRole('button'))
    const box = screen.getByTestId('reasoning-scroll')
    let height = 300
    let top = 0
    let writes = 0
    Object.defineProperties(box, {
      clientHeight: { configurable: true, value: 100 },
      scrollHeight: { configurable: true, get: () => height },
      scrollTop: { configurable: true, get: () => top, set: (value: number) => { top = Math.min(value, height - 100); writes++ } },
    })
    const grow = () => act(() => resize([{ target, borderBoxSize: [{ blockSize: height }] } as unknown as ResizeObserverEntry], {} as ResizeObserver))
    grow()
    expect(top).toBe(200)
    const initialWrites = writes
    rerender(<ReasoningBlock reasoning="alpha beta" streaming />)
    grow() // A token / width-only notification without a new line needs no scroll write.
    expect(writes).toBe(initialWrites)
    top = 40
    fireEvent.scroll(box)
    height = 400
    rerender(<ReasoningBlock reasoning={'alpha beta\ngamma'} streaming />)
    grow()
    expect(top).toBe(40)
    rerender(<ReasoningBlock reasoning={'alpha beta\ngamma'} />)
    height = 500
    grow()
    expect(top).toBe(40)
    top = 400
    fireEvent.scroll(box)
    height = 600
    grow()
    expect(top).toBe(500)
    unmount()
    expect(disconnect).toHaveBeenCalled()
  })
})
