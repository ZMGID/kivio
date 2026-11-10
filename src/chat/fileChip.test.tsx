import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { FileChip, FileKindIcon } from './fileChip'

describe('Pierre file icons', () => {
  it.each([
    ['main.ts', 'typescript'],
    ['C:\\project\\View.TSX', 'react'],
    ['main.py', 'python'],
    ['main.rs', 'rust'],
    ['README.md', 'markdown'],
    ['package.json', 'json'],
    ['table.xlsx', 'table'],
    ['unknown.custom', 'default'],
  ])('renders the bundled %s icon', (name, token) => {
    const { container } = render(<FileKindIcon name={name} />)
    const icon = container.querySelector('svg')!
    const id = `file-tree-builtin-${token}`
    expect(icon.querySelector('use')).toHaveAttribute('href', `#${id}`)
    expect(document.getElementById(id)?.querySelector('path')).not.toBeNull()
    expect(icon).toHaveAttribute('aria-hidden', 'true')
    expect(icon).toHaveAttribute('height', '1em')
    expect(container.textContent).toBe('\u2060')
  })

  it('shares the sprite across icons and remounts without adding layout space', () => {
    const first = render(<FileKindIcon name="main.ts" />)
    const sprite = document.getElementById('kivio-file-icon-sprite')
    first.unmount()
    render(<><FileKindIcon name="main.py" /><FileKindIcon name="main.rs" /></>)
    expect(document.querySelectorAll('#kivio-file-icon-sprite')).toHaveLength(1)
    expect(document.getElementById('kivio-file-icon-sprite')).toBe(sprite)
    expect(sprite).toHaveStyle({ width: '0px', height: '0px' })
  })

  it('keeps type colors separate from link colors in both themes', () => {
    const { container } = render(<FileKindIcon name="main.rs" />)
    expect(container.querySelector('svg')).toHaveClass('text-orange-600', 'dark:text-orange-400')
  })

  it('keeps folder icons distinct from files', () => {
    const { container } = render(<FileKindIcon name="src" folder />)
    expect(container.querySelector('.lucide-folder')).toHaveAttribute('fill', 'none')
    expect(container.querySelector('svg')).toHaveClass('text-[var(--text-muted)]')
    expect(container.querySelector('use')).toBeNull()
  })

  it.each(['card', 'compact', 'inline'] as const)('preserves file names and open actions in %s attachments', variant => {
    const open = vi.fn()
    render(<FileChip name="报告.pdf" variant={variant} onClick={open} />)
    const button = screen.getByRole('button', { name: '报告.pdf' })
    expect(button.querySelector('use')).toHaveAttribute('href', '#file-tree-builtin-default')
    expect(button.querySelector('svg')).toHaveClass('text-red-500')
    fireEvent.click(button)
    expect(open).toHaveBeenCalledOnce()
  })
})
