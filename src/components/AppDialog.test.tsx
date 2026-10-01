import { act, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import { AppDialogHost } from './AppDialog'
import { alertDialog, confirmDialog } from './dialogQueue'

beforeAll(() => {
  HTMLDialogElement.prototype.showModal ??= function showModal(this: HTMLDialogElement) {
    this.setAttribute('open', '')
  }
  HTMLDialogElement.prototype.close ??= function close(this: HTMLDialogElement) {
    this.removeAttribute('open')
  }
})

afterEach(() => {
  vi.restoreAllMocks()
})

describe('AppDialogHost', () => {
  it('resolves true on confirm and false on cancel', async () => {
    render(<AppDialogHost />)
    let answer: Promise<boolean> = Promise.resolve(false)
    act(() => {
      answer = confirmDialog({ message: 'Delete it?', confirmLabel: 'Delete' })
    })
    expect(screen.getByRole('alertdialog', { name: 'Delete it?' })).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }))
    await expect(answer).resolves.toBe(true)
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()

    act(() => {
      answer = confirmDialog('Again?')
    })
    fireEvent.click(screen.getByRole('button', { name: '取消' }))
    await expect(answer).resolves.toBe(false)
  })

  it('treats Escape (dialog cancel) as cancel and keeps the key away from window listeners', async () => {
    render(<AppDialogHost />)
    const windowKeys = vi.fn()
    window.addEventListener('keydown', windowKeys)
    let answer: Promise<boolean> = Promise.resolve(true)
    act(() => {
      answer = confirmDialog({ message: 'Clear?', danger: true })
    })
    const dialog = screen.getByRole('alertdialog')
    fireEvent.keyDown(dialog, { key: 'Escape' })
    fireEvent(dialog, new Event('cancel', { cancelable: true }))
    await expect(answer).resolves.toBe(false)
    expect(windowKeys).not.toHaveBeenCalled()
    window.removeEventListener('keydown', windowKeys)
  })

  it('focuses Cancel for destructive confirms so Enter cannot delete by accident', () => {
    render(<AppDialogHost />)
    act(() => {
      void confirmDialog({ message: 'Delete?', confirmLabel: 'Delete', danger: true })
    })
    expect(screen.getByRole('button', { name: '取消' })).toHaveFocus()
    act(() => {
      void confirmDialog({ message: 'Next' })
    })
    fireEvent.click(screen.getByRole('button', { name: '取消' }))
    // 非破坏性确认：焦点落在确认键。
    expect(screen.getByRole('button', { name: '确定' })).toHaveFocus()
    fireEvent.click(screen.getByRole('button', { name: '确定' }))
  })

  it('shows queued dialogs one at a time in order', async () => {
    render(<AppDialogHost />)
    let first: Promise<boolean> = Promise.resolve(false)
    let second: Promise<void> = Promise.resolve()
    act(() => {
      first = confirmDialog('First?')
      second = alertDialog('Second')
    })
    expect(screen.getAllByRole('alertdialog')).toHaveLength(1)
    expect(screen.getByRole('alertdialog', { name: 'First?' })).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '确定' }))
    await expect(first).resolves.toBe(true)
    expect(screen.getByRole('alertdialog', { name: 'Second' })).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: '好' }))
    await expect(second).resolves.toBeUndefined()
  })

  it('resolves pending dialogs as cancelled when the host unmounts', async () => {
    const view = render(<AppDialogHost />)
    let answer: Promise<boolean> = Promise.resolve(true)
    act(() => {
      answer = confirmDialog('Pending?')
    })
    view.unmount()
    await expect(answer).resolves.toBe(false)
  })

  it('restores focus inside the existing editor after a queued confirmation is dismissed', async () => {
    render(<><dialog open aria-label="Editor"><button type="button">Save editor</button></dialog><AppDialogHost /></>)
    const save = screen.getByRole('button', { name: 'Save editor' })
    save.focus()
    let answer: Promise<boolean> = Promise.resolve(true)
    act(() => { answer = confirmDialog('Discard changes?') })
    const confirmation = screen.getByRole('alertdialog', { name: 'Discard changes?' })
    expect(confirmation).toHaveAttribute('open')
    expect(screen.getByRole('button', { name: '确定' })).toHaveFocus()
    fireEvent.click(screen.getByRole('button', { name: '取消' }))
    await expect(answer).resolves.toBe(false)
    expect(save).toHaveFocus()
    expect(screen.getByRole('dialog', { name: 'Editor' })).toHaveAttribute('open')
  })

  it('falls back to the native dialogs when no host is mounted', async () => {
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(true)
    const alert = vi.spyOn(window, 'alert').mockImplementation(() => {})
    await expect(confirmDialog('Native?')).resolves.toBe(true)
    await alertDialog('Heads up')
    expect(confirm).toHaveBeenCalledWith('Native?')
    expect(alert).toHaveBeenCalledWith('Heads up')
  })
})
