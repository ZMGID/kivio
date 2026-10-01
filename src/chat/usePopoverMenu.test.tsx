import { act, fireEvent, render, screen } from '@testing-library/react'
import { useRef, useState } from 'react'
import { describe, expect, it } from 'vitest'
import { usePopoverMenu } from './usePopoverMenu'

function Harness({ withInput = false }: { withInput?: boolean }) {
  const [open, setOpen] = useState(false)
  const menuRef = useRef<HTMLDivElement>(null)
  usePopoverMenu(open, () => setOpen(false), menuRef)
  return (
    <div>
      <button type="button" aria-expanded={open} onClick={() => setOpen((value) => !value)}>
        Trigger
      </button>
      {open && (
        <div ref={menuRef} role="menu">
          {withInput && <input aria-label="Filter" />}
          <button type="button" role="menuitem">One</button>
          <button type="button" role="menuitem" disabled>Disabled</button>
          <button type="button" role="menuitem">Two</button>
          <button type="button" role="menuitem">Three</button>
        </div>
      )}
    </div>
  )
}

function openMenu() {
  const trigger = screen.getByRole('button', { name: 'Trigger' })
  trigger.focus()
  act(() => {
    fireEvent.click(trigger)
  })
  return trigger
}

function press(key: string) {
  act(() => {
    fireEvent.keyDown(document.activeElement ?? window, { key })
  })
}

describe('usePopoverMenu', () => {
  // jsdom 里程序聚焦后 :focus-visible 为真，所以这里的打开方式等同键盘打开。
  it('focuses the first enabled item on keyboard open and cycles with arrow keys, skipping disabled items', () => {
    render(<Harness />)
    openMenu()
    expect(screen.getByRole('menuitem', { name: 'One' })).toHaveFocus()
    press('ArrowDown')
    expect(screen.getByRole('menuitem', { name: 'Two' })).toHaveFocus()
    press('End')
    expect(screen.getByRole('menuitem', { name: 'Three' })).toHaveFocus()
    press('ArrowDown')
    expect(screen.getByRole('menuitem', { name: 'One' })).toHaveFocus()
    press('ArrowUp')
    expect(screen.getByRole('menuitem', { name: 'Three' })).toHaveFocus()
    press('Home')
    expect(screen.getByRole('menuitem', { name: 'One' })).toHaveFocus()
  })

  it('closes on Escape and returns focus to the trigger instead of dropping it on body', () => {
    render(<Harness />)
    const trigger = openMenu()
    expect(screen.getByRole('menuitem', { name: 'One' })).toHaveFocus()
    const escape = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })
    act(() => { document.activeElement?.dispatchEvent(escape) })
    expect(escape.defaultPrevented).toBe(true)
    expect(screen.queryByRole('menu')).not.toBeInTheDocument()
    expect(trigger).toHaveFocus()
    expect(trigger).toHaveAttribute('aria-expanded', 'false')
  })

  it('leaves Home and End to the text caret inside an input', () => {
    render(<Harness withInput />)
    openMenu()
    const input = screen.getByRole('textbox', { name: 'Filter' })
    expect(input).toHaveFocus()
    press('End')
    expect(input).toHaveFocus()
  })
})
