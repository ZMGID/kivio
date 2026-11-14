import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { DraftInput } from './shared'

describe('DraftInput', () => {
  it('keeps composition Enter focused and commits only after editing finishes', () => {
    const commit = vi.fn()
    render(<DraftInput aria-label="path" value="" onCommit={commit} />)
    const input = screen.getByRole('textbox', { name: 'path' })
    input.focus()
    fireEvent.change(input, { target: { value: '中文目录' } })
    fireEvent.keyDown(input, { key: 'Enter', isComposing: true })
    expect(document.activeElement).toBe(input)
    expect(commit).not.toHaveBeenCalled()
    fireEvent.keyDown(input, { key: 'Enter', isComposing: false })
    expect(commit).toHaveBeenCalledWith('中文目录')
  })

  it('restores the saved value on Escape without saving the draft', () => {
    const commit = vi.fn()
    render(<DraftInput aria-label="path" value="original" onCommit={commit} />)
    const input = screen.getByRole('textbox', { name: 'path' })
    fireEvent.change(input, { target: { value: 'unfinished' } })
    fireEvent.keyDown(input, { key: 'Escape' })
    fireEvent.blur(input)
    expect(input).toHaveValue('original')
    expect(commit).not.toHaveBeenCalled()
  })
})
