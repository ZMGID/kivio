import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ChatHeadingOutline } from './ChatHeadingOutline'
import { outlineItemsForSource } from './markdownHeadingOutline'

const items = outlineItemsForSource('answer-1', '# First\n## Second\n### Third')

describe('ChatHeadingOutline', () => {
  it('renders one row per heading that expands titles in place on hover', () => {
    render(
      <ChatHeadingOutline
        items={items}
        activeAnchorId={items[0]!.anchorId}
        onNavigate={() => {}}
      />,
    )

    const outline = screen.getByLabelText('回答标题目录')
    const list = outline.querySelector('.chat-heading-navigator-list')!
    expect(list.querySelectorAll('button')).toHaveLength(3)
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
        items={items}
        activeAnchorId={items[1]!.anchorId}
        onNavigate={onNavigate}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: '跳转到：Third' }))
    expect(onNavigate).toHaveBeenCalledWith(items[2])
    expect(screen.getByRole('button', { name: '跳转到：Second' })).toHaveAttribute('aria-current', 'location')
  })
})
