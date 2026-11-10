import { render } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { FolderIcon } from './FileIcon'

describe('folder artwork', () => {
  it('uses the reference neutral 16px outline and switches artwork when expanded', () => {
    const { container, rerender } = render(<FolderIcon />)
    const icon = container.querySelector('svg')!
    expect(icon).toHaveAttribute('width', '16')
    expect(icon).toHaveAttribute('height', '16')
    expect(icon).toHaveAttribute('fill', 'none')
    expect(icon).toHaveAttribute('stroke-width', '2')
    const closed = icon.lastElementChild?.getAttribute('d')
    rerender(<FolderIcon open />)
    expect(container.querySelector('svg')?.lastElementChild?.getAttribute('d')).not.toBe(closed)
    expect(container.querySelector('svg')).toHaveClass('text-[var(--text-muted)]')
  })

  it('preserves custom project colors', () => {
    const { container } = render(<FolderIcon style={{ color: 'rgb(10, 100, 200)' }} />)
    expect(container.querySelector('svg')).toHaveStyle({ color: 'rgb(10, 100, 200)' })
  })
})
