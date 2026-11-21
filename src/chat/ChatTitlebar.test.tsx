import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { LangContext } from '../components/i18n'
import { ChatTitlebar } from './ChatTitlebar'

describe('ChatTitlebar dock toggle', () => {
  it('keeps the same toggle beside the window controls when opening, resizing and closing the dock', () => {
    const onToggleDock = vi.fn()
    const props = { sidebarExpanded: true, onToggleSidebar: vi.fn(), onNewConversation: vi.fn(), onToggleDock }
    const header = (open: boolean, width = 360) => <LangContext.Provider value="zh">
      <ChatTitlebar {...props} dockVisible={open} dockWidth={width}>聊天控件</ChatTitlebar>
    </LangContext.Provider>
    const view = render(header(false))
    const toggle = screen.getByRole('button', { name: '面板' })
    const caption = screen.getByRole('button', { name: '最小化' }).parentElement
    expect(toggle.nextElementSibling).toBe(caption)
    expect(toggle).toHaveAttribute('aria-expanded', 'false')
    fireEvent.click(toggle)
    expect(onToggleDock).toHaveBeenCalledTimes(1)
    for (const [open, width] of [[true, 360], [true, 480], [false, 480]] as const) {
      view.rerender(header(open, width))
      expect(screen.getByRole('button', { name: '面板' })).toBe(toggle)
      expect(toggle.nextElementSibling).toBe(caption)
      expect(toggle).toHaveAttribute('aria-expanded', String(open))
    }
    fireEvent.click(toggle)
    expect(onToggleDock).toHaveBeenCalledTimes(2)
  })

  it('does not offer a dock toggle on pages without a dock', () => {
    render(<LangContext.Provider value="zh"><ChatTitlebar sidebarExpanded onToggleSidebar={() => {}} onNewConversation={() => {}} /></LangContext.Provider>)
    expect(screen.queryByRole('button', { name: '面板' })).not.toBeInTheDocument()
  })
})
