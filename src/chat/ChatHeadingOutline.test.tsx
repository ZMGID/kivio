import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ChatHeadingOutline } from './ChatHeadingOutline'
import { outlineItemsForSource } from './markdownHeadingOutline'

const items = outlineItemsForSource('answer-1', '# First\n## Second\n### Third')

describe('ChatHeadingOutline', () => {
  it('keeps the content-sized panel mounted for animation but inert until the rail expands', () => {
    render(
      <ChatHeadingOutline
        ownerMessageId="answer-1"
        items={items}
        activeAnchorId={items[0]!.anchorId}
        onNavigate={() => {}}
      />,
    )

    const outline = screen.getByLabelText('回答标题目录')
    const panel = outline.querySelector('.chat-heading-navigator-panel')
    const firstHeading = panel?.querySelector<HTMLButtonElement>('[title="First"]')

    expect(panel).toBeInTheDocument()
    expect(panel).toHaveAttribute('aria-hidden', 'true')
    expect(firstHeading).toHaveAttribute('tabindex', '-1')
    expect(outline).not.toHaveClass('is-expanded')

    fireEvent.pointerEnter(outline)

    expect(outline).toHaveClass('is-expanded')
    expect(panel).toHaveAttribute('aria-hidden', 'false')
    expect(firstHeading).toHaveAttribute('tabindex', '0')
  })

  it('shows the first heading directly and keeps deeper headings behind the top-right toggle', () => {
    render(
      <ChatHeadingOutline
        ownerMessageId="answer-1"
        items={items}
        activeAnchorId={items[0]!.anchorId}
        onNavigate={() => {}}
      />,
    )

    fireEvent.pointerEnter(screen.getByLabelText('回答标题目录'))
    expect(screen.getByRole('button', { name: 'First' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Second' })).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '展开二、三级标题' }))
    expect(screen.getByRole('button', { name: 'Second' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Third' })).toBeInTheDocument()
  })

  it('navigates from a visible heading and exposes the current location', () => {
    const onNavigate = vi.fn()
    render(
      <ChatHeadingOutline
        ownerMessageId="answer-1"
        items={items}
        activeAnchorId={items[1]!.anchorId}
        onNavigate={onNavigate}
      />,
    )
    fireEvent.pointerEnter(screen.getByLabelText('回答标题目录'))
    fireEvent.click(screen.getByRole('button', { name: 'First' }))
    expect(onNavigate).toHaveBeenCalledWith(items[0])
    expect(screen.getAllByRole('button', { name: '跳转到：Second' })[0]).toHaveAttribute('aria-current', 'location')
  })
})
