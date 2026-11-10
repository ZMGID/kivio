import type { ReactNode } from 'react'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { FileTreePanel } from './FileTreePanel'
import { dockApi } from './api'

// jsdom has no viewport geometry; render every virtual row while using the real tree hook.
vi.mock('virtua', async () => {
  const { forwardRef } = await import('react')
  return { VList: forwardRef<HTMLDivElement, { children?: ReactNode }>(({ children }, ref) => <div ref={ref}>{children}</div>) }
})
vi.mock('./api', () => {
  const entries = ['README.md', 'App.tsx', 'package.json'].map(path => ({ path, kind: 'file', hidden: false }))
  return { dockApi: {
    fsList: (_workdir: string, path: string) => Promise.resolve({ entries: path ? [] : [{ path: 'src', kind: 'dir', hidden: false }, ...entries], hasMore: false }),
    fsSearch: vi.fn(() => Promise.resolve({ entries, truncated: false })),
  } }
})
vi.mock('./workspaceActivity', () => ({ workspaceActivity: { isAvailable: () => true, subscribe: () => () => {} } }))

describe('file tree icons', () => {
  it('updates folder artwork when the directory is expanded', async () => {
    const props = { workdir: '/project', active: true, lang: 'zh' as const, onExpandedChange: () => {} }
    const { rerender } = render(<FileTreePanel {...props} expandedPaths={[]} />)
    const row = (await screen.findByText('src')).closest('[role="button"]')!
    const folder = row.querySelector('svg')!
    expect(folder.querySelector('use')).toHaveAttribute('href', '#file-tree-icon-chevron')
    expect(row.querySelector('.lucide-folder')).toBeNull()
    expect(folder).toHaveAttribute('height', '16')
    expect(folder).toHaveStyle({ transform: 'rotate(-90deg)' })
    rerender(<FileTreePanel {...props} expandedPaths={['src']} />)
    expect(row.querySelector('svg')).not.toHaveStyle({ transform: 'rotate(-90deg)' })
  })

  it.each([false, true])('uses Pierre icons in tree and search rows (search: %s)', async searching => {
    render(<FileTreePanel workdir="/project" active lang="zh" expandedPaths={[]} onExpandedChange={() => {}} />)
    await screen.findByText('README.md')
    if (searching) {
      fireEvent.change(screen.getByRole('textbox'), { target: { value: 'read' } })
      await waitFor(() => expect(dockApi.fsSearch).toHaveBeenCalled())
    }
    for (const [name, token] of [['README.md', 'markdown'], ['App.tsx', 'react'], ['package.json', 'json']]) {
      const row = screen.getByText(name).closest('[role="button"]')
      expect(row?.querySelector('use')).toHaveAttribute('href', `#file-tree-builtin-${token}`)
    }
  })
})
