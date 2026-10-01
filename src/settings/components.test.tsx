import { act, fireEvent, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { FieldBlock, Select, TextArea, Toggle } from './components'

describe('Toggle', () => {
  it('reflects checked state and toggles on click', async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    render(<Toggle checked={false} onChange={onChange} />)
    const toggle = screen.getByRole('switch')
    expect(toggle).toHaveAttribute('aria-checked', 'false')
    await user.click(toggle)
    expect(onChange).toHaveBeenCalledWith(true)
  })
})

describe('Select', () => {
  it('opens menu and selects an option', async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    render(
      <Select
        value="a"
        onChange={onChange}
        options={[
          { value: 'a', label: 'Option A' },
          { value: 'b', label: 'Option B' },
        ]}
      />,
    )
    expect(screen.getByRole('button', { name: /Option A/i })).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: /Option A/i }))
    await user.click(screen.getByRole('option', { name: 'Option B' }))
    expect(onChange).toHaveBeenCalledWith('b')
  })

  it('cannot open or change an option while its fieldset is disabled', () => {
    const onChange = vi.fn()
    const options = [{ value: 'a', label: 'Option A' }, { value: 'b', label: 'Option B' }]
    const view = render(<fieldset><Select value="a" onChange={onChange} options={options} /></fieldset>)
    const trigger = screen.getByRole('button', { name: 'Option A' })
    fireEvent.click(trigger)
    expect(screen.getByRole('listbox')).toBeInTheDocument()

    view.rerender(<fieldset disabled><Select value="a" onChange={onChange} options={options} /></fieldset>)
    expect(trigger).toBeDisabled()
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
    fireEvent.keyDown(trigger, { key: 'ArrowDown' })
    fireEvent.click(trigger)
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
    expect(onChange).not.toHaveBeenCalled()
  })
})

describe('TextArea', () => {
  it('copies the selected text from its context menu', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText, readText: vi.fn().mockResolvedValue('pasted') },
    })

    render(<TextArea value="hello world" onChange={() => undefined} />)
    const field = screen.getByRole('textbox') as HTMLTextAreaElement

    field.setSelectionRange(0, 5)
    fireEvent.contextMenu(field, { clientX: 12, clientY: 20 })
    expect(screen.getByRole('menuitem', { name: '复制' })).toBeTruthy()
    expect(screen.getByRole('menuitem', { name: '剪切' })).toBeTruthy()
    expect(screen.getByRole('menuitem', { name: '粘贴' })).toBeTruthy()
    expect(screen.getByRole('menuitem', { name: '全选' })).toBeTruthy()

    fireEvent.click(screen.getByRole('menuitem', { name: '复制' }))
    expect(writeText).toHaveBeenCalledWith('hello')
  })

  it('associates its label with the textarea and exposes its description', async () => {
    const user = userEvent.setup()
    render(<FieldBlock label="Prompt" htmlFor="prompt">
      <TextArea id="prompt" value="hello" onChange={() => undefined} aria-describedby="hint" />
      <p id="hint">Describe the task</p>
    </FieldBlock>)
    const field = screen.getByRole('textbox', { name: 'Prompt', description: 'Describe the task' })
    await user.click(screen.getByText('Prompt'))
    expect(field).toHaveFocus()
  })

  it('keeps its context menu inside the closest open dialog and disables edits when read-only', () => {
    render(<dialog open aria-label="Outer"><dialog open aria-label="Editor">
      <TextArea value="hello" onChange={() => undefined} readOnly />
    </dialog></dialog>)
    const field = screen.getByRole('textbox') as HTMLTextAreaElement
    field.setSelectionRange(0, 5)
    fireEvent.contextMenu(field)
    const menu = screen.getByRole('menu')
    expect(menu.parentElement).toBe(screen.getByRole('dialog', { name: 'Editor' }))
    expect(screen.getByRole('menuitem', { name: '剪切' })).toBeDisabled()
    expect(screen.getByRole('menuitem', { name: '粘贴' })).toBeDisabled()
    expect(screen.getByRole('menuitem', { name: '复制' })).toBeEnabled()
  })

  it('does not offer editing actions when disabled', () => {
    render(<TextArea value="hello" onChange={() => undefined} disabled />)
    const field = screen.getByRole('textbox')
    expect(field).toBeDisabled()
    fireEvent.contextMenu(field)
    expect(screen.queryByRole('menu')).not.toBeInTheDocument()
  })

  it('closes a context menu when its fieldset becomes disabled', () => {
    const view = render(<fieldset><TextArea value="hello" onChange={() => undefined} /></fieldset>)
    fireEvent.contextMenu(screen.getByRole('textbox'))
    expect(screen.getByRole('menu')).toBeInTheDocument()
    view.rerender(<fieldset disabled><TextArea value="hello" onChange={() => undefined} /></fieldset>)
    expect(screen.getByRole('textbox')).toBeDisabled()
    expect(screen.queryByRole('menu')).not.toBeInTheDocument()
  })

  it('does not apply an outstanding paste after its fieldset becomes disabled', async () => {
    let resolvePaste!: (text: string) => void
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { readText: () => new Promise<string>(resolve => { resolvePaste = resolve }) },
    })
    const onChange = vi.fn()
    const view = render(<fieldset><TextArea value="hello" onChange={onChange} /></fieldset>)
    fireEvent.contextMenu(screen.getByRole('textbox'))
    fireEvent.click(screen.getByRole('menuitem', { name: '粘贴' }))
    view.rerender(<fieldset disabled><TextArea value="hello" onChange={onChange} /></fieldset>)
    await act(async () => { resolvePaste('pasted') })
    expect(screen.getByRole('textbox')).toHaveValue('hello')
    expect(onChange).not.toHaveBeenCalled()
  })
})
