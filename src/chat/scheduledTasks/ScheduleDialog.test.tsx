import { fireEvent, render, screen } from '@testing-library/react'
import { useRef, useState } from 'react'
import { beforeAll, describe, expect, it, vi } from 'vitest'
import { ScheduleDialog } from './ScheduleDialog'
import { Select, TextArea } from '../../settings/public/controls'
import { usePopoverMenu } from '../usePopoverMenu'

beforeAll(() => {
  HTMLDialogElement.prototype.showModal ??= function showModal(this: HTMLDialogElement) {
    this.setAttribute('open', '')
  }
})

function Popover() {
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)
  usePopoverMenu(open, () => setOpen(false), ref)
  return <><button onClick={() => setOpen(true)}>Models</button>
    {open && <div role="menu" ref={ref}><button role="menuitem">Model A</button></div>}
  </>
}

function escapeFrom(target: Element) {
  const key = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true })
  fireEvent(target, key)
  // jsdom does not perform the native default action for an uncancelled Escape.
  if (!key.defaultPrevented) {
    fireEvent(target.closest('dialog')!, new Event('cancel', { cancelable: true }))
  }
  return key.defaultPrevented
}

describe('ScheduleDialog', () => {
  it('stays open when a text selection drag ends on the backdrop', () => {
    const onClose = vi.fn()
    render(<ScheduleDialog title="编辑" onClose={onClose}><textarea aria-label="prompt" /></ScheduleDialog>)
    const dialog = screen.getByRole('dialog', { hidden: true })

    fireEvent.mouseDown(screen.getByLabelText('prompt'))
    fireEvent.click(dialog)
    expect(onClose).not.toHaveBeenCalled()

    fireEvent.mouseDown(dialog)
    fireEvent.click(dialog)
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it.each(['select', 'context menu', 'model popover'])('closes only the child %s on first Escape, then cancels the dialog', (kind) => {
    const onClose = vi.fn()
    render(<ScheduleDialog title="Editor" onClose={onClose}>
      {kind === 'select' ? <Select value="a" onChange={() => undefined} options={[{ value: 'a', label: 'Option A' }]} />
        : kind === 'context menu' ? <TextArea value="hello" onChange={() => undefined} aria-label="Prompt" />
        : <Popover />}
    </ScheduleDialog>)
    const dialog = screen.getByRole('dialog', { name: 'Editor' })
    const trigger = kind === 'context menu' ? screen.getByRole('textbox') : screen.getByRole('button', { name: kind === 'select' ? 'Option A' : 'Models' })
    if (kind === 'context menu') fireEvent.contextMenu(trigger)
    else { trigger.focus(); fireEvent.click(trigger) }
    expect(screen.getByRole(kind === 'select' ? 'listbox' : 'menu')).toBeInTheDocument()

    expect(escapeFrom(document.activeElement ?? trigger)).toBe(true)
    expect(screen.queryByRole(kind === 'select' ? 'listbox' : 'menu')).not.toBeInTheDocument()
    expect(onClose).not.toHaveBeenCalled()
    expect(escapeFrom(dialog)).toBe(false)
    expect(onClose).toHaveBeenCalledTimes(1)
  })
})
