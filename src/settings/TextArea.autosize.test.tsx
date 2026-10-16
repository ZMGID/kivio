import { useState } from 'react'
import { act, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { TextArea } from './components'

const fieldStyle = { width: '300px', boxSizing: 'border-box' as const, lineHeight: '20px', padding: '6px 8px', border: '1px solid' }
const autoSize = { minRows: 2, maxRows: 5 }

describe('TextArea bounded autosizing', () => {
  let visible: boolean
  let resize: ResizeObserverCallback
  let observe: ReturnType<typeof vi.fn>
  let disconnect: ReturnType<typeof vi.fn>
  let measured: ReturnType<typeof vi.fn>

  beforeEach(() => {
    visible = true
    observe = vi.fn()
    disconnect = vi.fn()
    measured = vi.fn()
    vi.stubGlobal('ResizeObserver', class {
      constructor(callback: ResizeObserverCallback) { resize = callback }
      observe = observe
      disconnect = disconnect
    })
    vi.spyOn(HTMLTextAreaElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLTextAreaElement) {
      return { width: visible ? Number.parseFloat(this.style.width) : 0 } as DOMRect
    })
    // jsdom has no layout. Model wrapping and row metrics for the hidden measuring
    // textarea; browser tests verify the shared control against real geometry.
    vi.spyOn(HTMLTextAreaElement.prototype, 'scrollHeight', 'get').mockImplementation(function (this: HTMLTextAreaElement) {
      measured()
      const columns = Math.max(1, Math.floor((Number.parseFloat(this.style.width) - 18) / 10))
      const lines = this.value.split('\n').reduce((count, line) => count + Math.max(1, Math.ceil(line.length / columns)), 0)
      return lines * 20 + 12
    })
  })

  afterEach(() => {
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  function notifyResize() {
    act(() => resize([], {} as ResizeObserver))
  }

  it('leaves existing callers and their sizing unchanged', () => {
    render(<TextArea value="old caller" onChange={() => undefined} rows={4} style={{ ...fieldStyle, height: '90px', resize: 'vertical' }} />)
    expect(screen.getByRole('textbox')).toHaveStyle({ height: '90px', resize: 'vertical' })
    expect(screen.getByRole('textbox')).toHaveAttribute('rows', '4')
    expect(observe).not.toHaveBeenCalled()
    expect(measured).not.toHaveBeenCalled()
  })

  it('grows, caps with internal scrolling, shrinks on clear, and sizes restored values', () => {
    const view = render(<TextArea value="" onChange={() => undefined} autoSize={autoSize} style={fieldStyle} />)
    const field = screen.getByRole('textbox')
    expect(field).toHaveStyle({ height: '54px', overflowY: 'hidden', resize: 'none' })
    view.rerender(<TextArea value={'one\ntwo\nthree'} onChange={() => undefined} autoSize={autoSize} style={fieldStyle} />)
    expect(field).toHaveStyle({ height: '74px', overflowY: 'hidden' })
    view.rerender(<TextArea value={'line\n'.repeat(20)} onChange={() => undefined} autoSize={autoSize} style={fieldStyle} />)
    expect(field).toHaveStyle({ height: '114px', overflowY: 'auto' })
    view.rerender(<TextArea value="" onChange={() => undefined} autoSize={autoSize} style={fieldStyle} />)
    expect(field).toHaveStyle({ height: '54px', overflowY: 'hidden' })
    view.rerender(<TextArea value={'restored\nwith\nthree rows'} onChange={() => undefined} autoSize={autoSize} style={fieldStyle} />)
    expect(field).toHaveStyle({ height: '74px', overflowY: 'hidden' })
  })

  it('remeasures wrapping on width changes without reacting to its own height changes', () => {
    render(<TextArea value={'x'.repeat(70)} onChange={() => undefined} autoSize={autoSize} style={fieldStyle} />)
    const field = screen.getByRole('textbox') as HTMLTextAreaElement
    expect(field).toHaveStyle({ height: '74px' })
    const initialMeasurements = measured.mock.calls.length
    notifyResize()
    expect(measured).toHaveBeenCalledTimes(initialMeasurements)
    field.style.width = '160px'
    notifyResize()
    expect(field).toHaveStyle({ height: '114px' })
    const resizedMeasurements = measured.mock.calls.length
    notifyResize()
    expect(measured).toHaveBeenCalledTimes(resizedMeasurements)
    field.style.width = '500px'
    notifyResize()
    expect(field).toHaveStyle({ height: '54px', overflowY: 'hidden' })
  })

  it('measures fields revealed after mounting hidden and after edits inside closed details', () => {
    visible = false
    const view = render(<details><summary>More</summary><TextArea value={'line\n'.repeat(20)} onChange={() => undefined} autoSize={autoSize} style={fieldStyle} /></details>)
    const field = view.container.querySelector('textarea')!
    expect(field.style.height).toBe('')
    visible = true
    notifyResize()
    expect(field).toHaveStyle({ height: '114px', overflowY: 'auto' })
    visible = false
    notifyResize()
    view.rerender(<details><summary>More</summary><TextArea value={'one\ntwo\nthree'} onChange={() => undefined} autoSize={autoSize} style={fieldStyle} /></details>)
    visible = true
    notifyResize()
    expect(field).toHaveStyle({ height: '74px', overflowY: 'hidden' })
  })

  it('keeps the live field, focus, selection, and scroll position untouched during measurement', () => {
    const view = render(<TextArea value={'line\n'.repeat(20)} onChange={() => undefined} autoSize={autoSize} style={fieldStyle} />)
    const field = screen.getByRole('textbox') as HTMLTextAreaElement
    field.focus()
    field.setSelectionRange(10, 14)
    field.scrollTop = 70
    view.rerender(<TextArea value={'line\n'.repeat(21)} onChange={() => undefined} autoSize={autoSize} style={fieldStyle} />)
    // React itself moves the selection on a replaced value. Width-only updates
    // must not replace or collapse the live textarea while measuring it.
    field.setSelectionRange(10, 14)
    field.style.width = '250px'
    notifyResize()
    expect(screen.getByRole('textbox')).toBe(field)
    expect(field).toHaveFocus()
    expect(field.selectionStart).toBe(10)
    expect(field.selectionEnd).toBe(14)
    expect(field.scrollTop).toBe(70)
    expect(document.querySelectorAll('textarea')).toHaveLength(1)
  })

  it('resizes context-menu paste and cut while restoring the caret', async () => {
    const pasted = 'one\ntwo\nthree\nfour\nfive\nsix'
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText: vi.fn().mockResolvedValue(undefined), readText: vi.fn().mockResolvedValue(pasted) },
    })
    function Editor() {
      const [value, setValue] = useState('')
      return <TextArea value={value} onChange={setValue} autoSize={autoSize} style={fieldStyle} />
    }
    render(<Editor />)
    const field = screen.getByRole('textbox') as HTMLTextAreaElement
    fireEvent.contextMenu(field)
    await act(async () => fireEvent.click(screen.getByRole('menuitem', { name: '粘贴' })))
    expect(field).toHaveValue(pasted)
    expect(field).toHaveStyle({ height: '114px', overflowY: 'auto' })
    expect(field).toHaveFocus()
    expect(field.selectionStart).toBe(pasted.length)
    expect(field.selectionEnd).toBe(pasted.length)
    field.setSelectionRange(0, pasted.length)
    fireEvent.contextMenu(field)
    fireEvent.click(screen.getByRole('menuitem', { name: '剪切' }))
    expect(field).toHaveValue('')
    expect(field).toHaveStyle({ height: '54px', overflowY: 'hidden' })
    expect(field.selectionStart).toBe(0)
  })

  it('releases its observer and restores caller sizing when autosizing is disabled', () => {
    const view = render(<TextArea value={'line\n'.repeat(20)} onChange={() => undefined} autoSize={autoSize} style={{ ...fieldStyle, height: '90px' }} />)
    expect(screen.getByRole('textbox')).toHaveStyle({ height: '114px' })
    view.rerender(<TextArea value="text" onChange={() => undefined} rows={4} style={{ ...fieldStyle, height: '90px' }} />)
    expect(screen.getByRole('textbox')).toHaveStyle({ height: '90px' })
    expect(screen.getByRole('textbox')).toHaveAttribute('rows', '4')
    expect((screen.getByRole('textbox') as HTMLTextAreaElement).style.resize).toBe('')
    expect(disconnect).toHaveBeenCalledTimes(1)
  })
})
