import { afterEach, describe, expect, it, vi } from 'vitest'
import { act, render } from '@testing-library/react'
import { ChatInlineImage } from './ChatInlineImage'
import { beginConversationTransition, completeConversationTransition, invalidateConversationTransition } from './conversationTransitionStore'

afterEach(() => { act(() => invalidateConversationTransition()) })

const PNG = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUg'

/** 触发 <img> 的 onLoad，并伪造自然尺寸（jsdom 不解码图片）。 */
function fireLoad(img: HTMLImageElement, width: number, height: number) {
  Object.defineProperty(img, 'naturalWidth', { value: width, configurable: true })
  Object.defineProperty(img, 'naturalHeight', { value: height, configurable: true })
  Object.defineProperty(img, 'currentSrc', { value: img.getAttribute('src'), configurable: true })
  act(() => {
    img.dispatchEvent(new Event('load'))
  })
}

describe('ChatInlineImage', () => {
  it('loads mounted images eagerly under the conversation mask and restores lazy loading afterwards', () => {
    const request = beginConversationTransition('image-opening')
    const { container } = render(<ChatInlineImage src="https://example.com/opening.png" alt="opening" />)
    const image = container.querySelector('img')!
    expect(image).toHaveAttribute('loading', 'eager')
    act(() => completeConversationTransition('image-opening', request))
    expect(container.querySelector('img')).toBe(image)
    expect(image).toHaveAttribute('loading', 'lazy')
  })

  it('caps the long edge at 128px so tiles can sit in a row', () => {
    const { container } = render(<ChatInlineImage src={`${PNG}A`} alt="x" />)
    const button = container.querySelector('button')!
    // 未知比例先按 1:1 占位，避免解码前撑满整行。
    expect(button.style.aspectRatio).toBe('1')
    expect(button.style.width).toBe('128px')
    expect(button.style.maxWidth).toBe('100%')

    fireLoad(container.querySelector('img')!, 1000, 500)
    expect(button.style.aspectRatio).toBe('2')
    // 横图：宽封顶 128，高 = 64。
    expect(button.style.width).toBe('128px')
  })

  it('caps portrait tiles by height so they stay 240 tall', () => {
    const { container } = render(<ChatInlineImage src={`${PNG}P`} alt="x" />)
    fireLoad(container.querySelector('img')!, 500, 1000)
    expect(container.querySelector('button')!.style.width).toBe('64px')
  })

  it('remembers the ratio across unmount so scrolling back does not re-measure', () => {
    const src = `${PNG}B`
    const first = render(<ChatInlineImage src={src} alt="x" />)
    fireLoad(first.container.querySelector('img')!, 800, 400)
    first.unmount()

    const second = render(<ChatInlineImage src={src} alt="x" />)
    expect(second.container.querySelector('button')!.style.aspectRatio).toBe('2')
  })

  it('resets ratio and loading state when the source changes', () => {
    const first = `${PNG}source-a`
    const second = `${PNG}source-b`
    const view = render(<ChatInlineImage src={first} alt="x" />)
    fireLoad(view.container.querySelector('img')!, 800, 400)
    expect(view.container.querySelector('button')!.style.aspectRatio).toBe('2')
    view.rerender(<ChatInlineImage src={second} alt="x" />)
    expect(view.container.querySelector('button')!.style.aspectRatio).toBe('1')
    act(() => { view.container.querySelector('img')!.dispatchEvent(new Event('error')) })
    expect(view.container.querySelector('button')).toHaveAttribute('aria-label', '重试加载图片')
    view.rerender(<ChatInlineImage src={first} alt="x" />)
    expect(view.container.querySelector('button')!.style.aspectRatio).toBe('2')
    expect(view.container.querySelector('button')).toHaveAttribute('aria-label', '预览图片')
  })

  it('lets the user retry a failed source', () => {
    const onOpenViewer = vi.fn()
    const view = render(<ChatInlineImage src="https://example.com/missing.png" alt="missing" onOpenViewer={onOpenViewer} />)
    const original = view.container.querySelector('img')!
    act(() => { original.dispatchEvent(new Event('error')) })
    expect(view.container.querySelector('button')).toHaveAttribute('aria-label', '重试加载图片')
    act(() => { view.container.querySelector('button')!.click() })
    expect(view.container.querySelector('img')).not.toBe(original)
    expect(onOpenViewer).not.toHaveBeenCalled()
    expect(view.container.querySelector('button')).toHaveAttribute('aria-label', '预览图片')
  })

  it('does not lazy-load data URLs (bytes are already in memory)', () => {
    const inline = render(<ChatInlineImage src={`${PNG}C`} alt="x" />)
    expect(inline.container.querySelector('img')!.getAttribute('loading')).toBeNull()

    const remote = render(<ChatInlineImage src="https://example.com/a.png" alt="x" />)
    expect(remote.container.querySelector('img')!.getAttribute('loading')).toBe('lazy')
  })

  it('stops the contextmenu from reaching the message-level menu', () => {
    const onParentContextMenu = vi.fn()
    const { container } = render(
      <div onContextMenu={onParentContextMenu}>
        <ChatInlineImage src={`${PNG}D`} alt="x" />
      </div>,
    )
    act(() => {
      container
        .querySelector('button')!
        .dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true }))
    })
    expect(onParentContextMenu).not.toHaveBeenCalled()
    expect(document.body.textContent).toContain('复制图片')
  })
})
