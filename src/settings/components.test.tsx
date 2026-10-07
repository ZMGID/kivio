import { act, fireEvent, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { FieldBlock, Select, SuggestInput, TextArea, Toggle } from './components'

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
  it.each(['text', 'icon', 'labeled-icon'])('opens menu and selects an option (%s trigger)', async (mode) => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    render(
      <Select
        ariaLabel="Option A"
        triggerIcon={mode !== 'text' ? <span aria-hidden>+</span> : undefined}
        triggerLabel={mode === 'labeled-icon' ? 'Current option' : undefined}
        value="a"
        onChange={onChange}
        options={[
          { value: 'a', label: 'Option A' },
          { value: 'b', label: 'Option B' },
        ]}
      />,
    )
    expect(screen.getByRole('button', { name: /Option A/i })).toBeInTheDocument()
    if (mode === 'labeled-icon') {
      expect(within(screen.getByRole('button', { name: 'Option A' })).getByText('Current option')).toBeVisible()
    }
    await user.click(screen.getByRole('button', { name: /Option A/i }))
    expect(screen.getByRole('button', { name: /Option A/i })).toHaveFocus()
    await user.click(screen.getByRole('option', { name: 'Option B' }))
    expect(onChange).toHaveBeenCalledWith('b')
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Option A/i })).toHaveFocus()
  })

  it.each(['text', 'icon', 'labeled-icon'])('focuses the selected option on keyboard open and selects with Enter (%s trigger)', async (mode) => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    render(<Select
      ariaLabel="Choose option"
      triggerIcon={mode !== 'text' ? <span aria-hidden>+</span> : undefined}
      triggerLabel={mode === 'labeled-icon' ? 'Current option' : undefined}
      value="b"
      onChange={onChange}
      options={[
        { value: 'a', label: 'Option A' },
        { value: 'b', label: 'Option B' },
        { value: 'c', label: 'Option C' },
      ]}
    />)
    const trigger = screen.getByRole('button', { name: 'Choose option' })
    trigger.focus()
    await user.keyboard('{Enter}')
    expect(screen.getByRole('option', { name: 'Option B', selected: true })).toHaveFocus()
    expect(onChange).not.toHaveBeenCalled()
    await user.keyboard('{ArrowDown}{Enter}')
    expect(onChange).toHaveBeenCalledTimes(1)
    expect(onChange).toHaveBeenCalledWith('c')
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
    expect(trigger).toHaveFocus()
  })

  it.each([' ', '{ArrowDown}', '{ArrowUp}'])('focuses the first option when the current value is unavailable (%s)', async (key) => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    render(<Select ariaLabel="Choose option" value="missing" onChange={onChange} options={[
      { value: 'a', label: 'Option A' }, { value: 'b', label: 'Option B' },
    ]} />)
    const trigger = screen.getByRole('button', { name: 'Choose option' })
    trigger.focus()
    await user.keyboard(key)
    expect(screen.getByRole('option', { name: 'Option A' })).toHaveFocus()
    expect(onChange).not.toHaveBeenCalled()
    await user.keyboard(' ')
    expect(onChange).toHaveBeenCalledTimes(1)
    expect(onChange).toHaveBeenCalledWith('a')
    expect(trigger).toHaveFocus()
  })

  it('navigates enabled options with arrows, Home and End without changing the value, then cancels with Escape', async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    render(<Select ariaLabel="Choose option" value="b" onChange={onChange} options={[
      { value: 'a', label: 'Option A' },
      { value: 'b', label: 'Option B' },
      { value: 'c', label: 'Option C' },
      { value: 'd', label: 'Option D' },
    ]} />)
    const trigger = screen.getByRole('button', { name: 'Choose option' })
    trigger.focus()
    await user.keyboard('{ArrowDown}')
    const optionA = screen.getByRole('option', { name: 'Option A' })
    const optionB = screen.getByRole('option', { name: 'Option B' })
    const optionC = screen.getByRole('option', { name: 'Option C' }) as HTMLButtonElement
    const optionD = screen.getByRole('option', { name: 'Option D' })
    optionC.disabled = true
    expect(optionB).toHaveFocus()
    await user.keyboard('{ArrowDown}')
    expect(optionD).toHaveFocus()
    await user.keyboard('{ArrowDown}')
    expect(optionA).toHaveFocus()
    await user.keyboard('{ArrowUp}')
    expect(optionD).toHaveFocus()
    await user.keyboard('{Home}')
    expect(optionA).toHaveFocus()
    await user.keyboard('{End}')
    expect(optionD).toHaveFocus()
    await user.keyboard('{ArrowUp}')
    expect(optionB).toHaveFocus()
    expect(onChange).not.toHaveBeenCalled()
    await user.keyboard('{Escape}')
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
    expect(trigger).toHaveFocus()
    expect(onChange).not.toHaveBeenCalled()
    await user.keyboard('{ArrowUp}')
    expect(screen.getByRole('option', { name: 'Option B', selected: true })).toHaveFocus()
  })

  it.each([false, true])('closes on Tab and continues from the trigger in document order (reverse: %s)', async (shift) => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    render(<>
      <button>Before</button>
      <Select ariaLabel="Choose option" value="b" onChange={onChange} options={[
        { value: 'a', label: 'Option A' }, { value: 'b', label: 'Option B' },
      ]} />
      <button>After</button>
    </>)
    const trigger = screen.getByRole('button', { name: 'Choose option' })
    trigger.focus()
    await user.keyboard('{ArrowDown}')
    const option = screen.getByRole('option', { name: 'Option B' })
    expect(option).toHaveFocus()
    // user-event v14 snapshots the key target for Tab rather than using the
    // focus moved by the handler. Assert that handoff before native traversal.
    expect(fireEvent.keyDown(option, { key: 'Tab', shiftKey: shift })).toBe(true)
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
    expect(trigger).toHaveFocus()
    await user.tab({ shift })
    expect(screen.getByRole('button', { name: shift ? 'Before' : 'After' })).toHaveFocus()
    expect(onChange).not.toHaveBeenCalled()
  })

  it('toggles on repeated trigger clicks, cancels outside, and closes when tabbing from a mouse-opened menu', async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    render(<>
      <Select ariaLabel="Choose option" value="a" onChange={onChange} options={[
        { value: 'a', label: 'Option A' }, { value: 'b', label: 'Option B' },
      ]} />
      <button>Outside</button>
    </>)
    const trigger = screen.getByRole('button', { name: 'Choose option' })
    const outside = screen.getByRole('button', { name: 'Outside' })
    await user.click(trigger)
    expect(screen.getByRole('listbox')).toBeInTheDocument()
    await user.click(trigger)
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
    await user.click(trigger)
    await user.click(outside)
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
    expect(outside).toHaveFocus()
    await user.click(trigger)
    await user.tab()
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
    expect(outside).toHaveFocus()
    expect(onChange).not.toHaveBeenCalled()
  })

  it.each(['text', 'icon', 'labeled-icon'])('cannot open or change an option while its fieldset is disabled (%s trigger)', (mode) => {
    const onChange = vi.fn()
    const options = [{ value: 'a', label: 'Option A' }, { value: 'b', label: 'Option B' }]
    const triggerProps = { ariaLabel: 'Option A', triggerIcon: mode !== 'text' ? <span aria-hidden>+</span> : undefined, triggerLabel: mode === 'labeled-icon' ? 'Current option' : undefined }
    const view = render(<fieldset><Select {...triggerProps} value="a" onChange={onChange} options={options} /></fieldset>)
    const trigger = screen.getByRole('button', { name: 'Option A' })
    fireEvent.click(trigger)
    expect(screen.getByRole('listbox')).toBeInTheDocument()

    view.rerender(<fieldset disabled><Select {...triggerProps} value="a" onChange={onChange} options={options} /></fieldset>)
    expect(trigger).toBeDisabled()
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
    for (const key of ['ArrowDown', 'ArrowUp', 'Enter', ' ']) fireEvent.keyDown(trigger, { key })
    fireEvent.click(trigger)
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
    expect(onChange).not.toHaveBeenCalled()
  })
})

describe('SuggestInput', () => {
  it('keeps text navigation keys in its input while suggestions are open', async () => {
    const user = userEvent.setup()
    render(<SuggestInput ariaLabel="Model" value="model-name" onChange={vi.fn()} options={[
      { value: 'model-name', label: 'Model name' }, { value: 'another', label: 'Another model' },
    ]} />)
    await user.click(screen.getByRole('button', { name: 'Model list' }))
    const input = screen.getByRole('textbox', { name: 'Model' }) as HTMLInputElement
    await user.click(input)
    await user.keyboard('{Home}')
    expect(input).toHaveFocus()
    expect(input.selectionStart).toBe(0)
    await user.keyboard('{End}')
    expect(input.selectionStart).toBe('model-name'.length)
    expect(input).toHaveFocus()
    expect(screen.getByRole('listbox')).toBeInTheDocument()
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
