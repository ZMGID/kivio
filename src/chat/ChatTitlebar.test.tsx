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

  it('fades the sidebar and dock columns with their panels instead of switching them instantly', () => {
    const props = { sidebarExpanded: true, onToggleSidebar: vi.fn(), onNewConversation: vi.fn(), onToggleDock: vi.fn(), dockWidth: 420 }
    const header = (sidebarVisible: boolean, dockVisible: boolean, settingsMode = false) => <LangContext.Provider value="zh">
      <ChatTitlebar {...props} sidebarVisible={sidebarVisible} dockVisible={dockVisible} settingsMode={settingsMode} />
    </LangContext.Provider>
    const view = render(header(true, true))
    const sidebarBg = document.querySelector('.chat-titlebar-panel-bg--sidebar')!
    const dockBg = document.querySelector('.chat-titlebar-panel-bg--dock')!
    expect(sidebarBg).toHaveClass('is-visible')
    expect(dockBg).toHaveClass('is-visible')

    // Collapsing keeps the same layers (so CSS can fade them) and the dock width they fade out at.
    view.rerender(header(false, false))
    expect(document.querySelector('.chat-titlebar-panel-bg--sidebar')).toBe(sidebarBg)
    expect(document.querySelector('.chat-titlebar-panel-bg--dock')).toBe(dockBg)
    expect(sidebarBg).not.toHaveClass('is-visible')
    expect(dockBg).not.toHaveClass('is-visible')
    expect((dockBg.parentElement as HTMLElement).style.getPropertyValue('--chat-dock-width')).toBe('420px')

    // The settings page covers the sidebar at once; its column does not fade over the page.
    view.rerender(header(false, false, true))
    expect(document.querySelector('.chat-titlebar-panel-bg--sidebar')).toHaveClass('is-instant')
  })
})
