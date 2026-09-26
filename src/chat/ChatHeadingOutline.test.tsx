import { fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { LangContext } from '../components/i18n'
import { ChatHeadingOutline } from './ChatHeadingOutline'
import { clearRememberedChatHeadingOutlineState } from './chatHeadingOutlinePersistence'
import { outlineItemsForSource } from './markdownHeadingOutline'

const items = outlineItemsForSource('answer-1', '# First\n## Second\n### Third')

describe('ChatHeadingOutline', () => {
  afterEach(() => {
    clearRememberedChatHeadingOutlineState()
  })

  it('renders one row per heading that expands titles in place on hover', () => {
    render(
      <ChatHeadingOutline
        conversationId="conversation-1"
        items={items}
        activeAnchorId={items[0]!.anchorId}
        onNavigate={() => {}}
      />,
    )

    const outline = screen.getByLabelText('回答标题目录')
    const list = outline.querySelector('.chat-heading-navigator-list')!
    expect(list.querySelectorAll('button')).toHaveLength(2)
    expect(outline).not.toHaveClass('is-expanded')

    fireEvent.pointerEnter(list)
    expect(outline).toHaveClass('is-expanded')
    expect(list.querySelectorAll('button')).toHaveLength(3)

    fireEvent.pointerLeave(list)
    expect(outline).not.toHaveClass('is-expanded')
  })

  it('navigates directly from a collapsed tick and exposes the current location', () => {
    const onNavigate = vi.fn()
    render(
      <ChatHeadingOutline
        conversationId="conversation-1"
        items={items}
        activeAnchorId={items[1]!.anchorId}
        onNavigate={onNavigate}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: '跳转到：Second' }))
    expect(onNavigate).toHaveBeenCalledWith(items[1])
    expect(screen.getByRole('button', { name: '跳转到：Second' })).toHaveAttribute('aria-current', 'location')
  })

  it('expands all heading levels and remembers the choice per conversation', () => {
    const { rerender } = render(
      <ChatHeadingOutline
        conversationId="conversation-1"
        items={items}
        activeAnchorId={items[0]!.anchorId}
        onNavigate={() => {}}
      />,
    )
    const list = screen.getByLabelText('回答标题目录').querySelector('.chat-heading-navigator-list')!
    fireEvent.pointerEnter(list)
    fireEvent.click(screen.getByRole('button', { name: '显示所有标题层级' }))
    expect(screen.getByRole('button', { name: '跳转到：Third' })).toBeInTheDocument()

    rerender(
      <ChatHeadingOutline
        conversationId="conversation-2"
        items={items}
        activeAnchorId={items[0]!.anchorId}
        onNavigate={() => {}}
      />,
    )
    fireEvent.pointerEnter(screen.getByLabelText('回答标题目录').querySelector('.chat-heading-navigator-list')!)
    expect(screen.queryByRole('button', { name: '跳转到：Third' })).not.toBeInTheDocument()

    rerender(
      <ChatHeadingOutline
        conversationId="conversation-1"
        items={items}
        activeAnchorId={items[0]!.anchorId}
        onNavigate={() => {}}
      />,
    )
    fireEvent.pointerEnter(screen.getByLabelText('回答标题目录').querySelector('.chat-heading-navigator-list')!)
    expect(screen.getByRole('button', { name: '跳转到：Third' })).toBeInTheDocument()
  })

  it.each([
    ['zh', '显示所有标题层级', '仅显示主级和次级标题', '跳转到：Third', '跳转到：Second'],
    ['en', 'Show all heading levels', 'Show primary and secondary headings', 'Jump to: Third', 'Jump to: Second'],
  ] as const)('labels the visible levels accurately in %s and preserves heading navigation', (lang, expand, collapse, third, second) => {
    const onNavigate = vi.fn()
    render(
      <LangContext.Provider value={lang}>
        <ChatHeadingOutline
          conversationId="conversation-labels"
          items={items}
          activeAnchorId={items[0]!.anchorId}
          onNavigate={onNavigate}
        />
      </LangContext.Provider>,
    )
    const list = document.querySelector('.chat-heading-navigator-list')!
    fireEvent.pointerEnter(list)
    fireEvent.click(screen.getByRole('button', { name: expand }))
    fireEvent.click(screen.getByRole('button', { name: third }))
    expect(onNavigate).toHaveBeenCalledWith(items[2])
    fireEvent.click(screen.getByRole('button', { name: collapse }))
    expect(screen.queryByRole('button', { name: third })).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: second }))
    expect(onNavigate).toHaveBeenLastCalledWith(items[1])
  })
})
