/**
 * @vitest-environment jsdom
 */
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { LangContext } from '../../components/i18n'
import { PopoutOccupiedPlaceholder } from './PopoutOccupiedPlaceholder'

describe('PopoutOccupiedPlaceholder', () => {
  it('docks the conversation back without focusing the popout', async () => {
    const user = userEvent.setup()
    const onFocus = vi.fn()
    const onDock = vi.fn()
    render(
      <LangContext.Provider value="zh">
        <PopoutOccupiedPlaceholder
          lang="zh"
          onFocus={onFocus}
          onDock={onDock}
          sidebarCollapsed={false}
          titlebarControls={null}
          onToggleSidebar={() => {}}
          onNewConversation={() => {}}
        />
      </LangContext.Provider>,
    )

    const dock = screen.getByRole('button', { name: '回归窗口' })
    await user.click(dock)
    expect(onDock).toHaveBeenCalledOnce()
    expect(onFocus).not.toHaveBeenCalled()
  })
})
