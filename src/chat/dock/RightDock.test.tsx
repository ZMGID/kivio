import { fireEvent, render, screen } from '@testing-library/react'
import type { ComponentProps } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { RightDock } from './RightDock'

const { loadedPanels } = vi.hoisted(() => ({ loadedPanels: [] as string[] }))

vi.mock('./FileTreePanel', () => {
  loadedPanels.push('files')
  return {
    FileTreePanel: ({ active, previewRequest, revealPath }: {
      active: boolean
      previewRequest: { text?: string } | null
      revealPath: string | null
    }) => (
      <div data-testid="files-panel" data-active={active}>
        <input aria-label="File search" defaultValue="" />
        <span>{previewRequest?.text}</span>
        <span>{revealPath}</span>
      </div>
    ),
  }
})
vi.mock('./GitPanel', () => {
  loadedPanels.push('git')
  return { GitPanel: () => <div data-testid="git-panel" /> }
})
vi.mock('./TerminalPanel', () => {
  loadedPanels.push('terminal')
  return {
    TerminalPanel: ({ active, workdir }: { active: boolean; workdir: string }) => (
      <div data-testid="terminal-panel" data-active={active} data-workdir={workdir} />
    ),
  }
})
vi.mock('./BackgroundTasksPanel', () => ({ BackgroundTasksPanel: () => <div /> }))

function dockProps(overrides: Partial<ComponentProps<typeof RightDock>> = {}): ComponentProps<typeof RightDock> {
  return {
    open: true,
    width: 360,
    activeTab: 'files',
    workdir: '/tmp/project',
    lang: 'zh',
    conversationId: 'conv-1',
    treeExpanded: [],
    revealRequest: null,
    previewRequest: null,
    onToggleTab: vi.fn(),
    onWidthChange: vi.fn(),
    onClose: vi.fn(),
    onTreeExpandedChange: vi.fn(),
    onRevealInTree: vi.fn(),
    ...overrides,
  }
}

describe('RightDock tabs', () => {
  it('renders panels immediately on opening without waiting for hover or an import', () => {
    const props = dockProps({ open: false })
    const { rerender } = render(<RightDock {...props} />)
    expect(loadedPanels).toHaveLength(3)
    expect(loadedPanels).toEqual(expect.arrayContaining(['files', 'git', 'terminal']))
    expect(screen.queryByTestId('terminal-panel')).toBeNull()

    rerender(<RightDock {...props} open />)
    expect(screen.getByTestId('files-panel')).toHaveAttribute('data-active', 'true')
    expect(screen.queryByTestId('terminal-panel')).toBeNull()

    rerender(<RightDock {...props} open activeTab="git" />)
    expect(screen.getByTestId('git-panel')).toBeVisible()

    rerender(<RightDock {...props} open activeTab="terminal" />)
    expect(screen.getByTestId('terminal-panel')).toHaveAttribute('data-active', 'true')
  })

  it('exposes files, git, terminal, and tasks — not trajectory', async () => {
    render(<RightDock {...dockProps()} />)
    await screen.findByTestId('files-panel')
    expect(screen.getByText('文件')).toBeTruthy()
    expect(screen.getByText('Git')).toBeTruthy()
    expect(screen.getByText('终端')).toBeTruthy()
    expect(screen.getByText('任务')).toBeTruthy()
    expect(screen.queryByText('轨迹')).toBeNull()
  })

  it('does not mount a remembered terminal tab while the dock is closed', async () => {
    const props = dockProps({ open: false, activeTab: 'terminal' })
    const { rerender } = render(<RightDock {...props} />)
    expect(screen.queryByTestId('terminal-panel')).toBeNull()

    rerender(<RightDock {...props} open />)
    expect(await screen.findByTestId('terminal-panel')).toHaveAttribute('data-active', 'true')
  })

  it('keeps an opened terminal mounted across tab changes and closing the dock', async () => {
    const props = dockProps({ activeTab: 'terminal' })
    const { rerender } = render(<RightDock {...props} />)
    const terminal = await screen.findByTestId('terminal-panel')

    rerender(<RightDock {...props} activeTab="files" />)
    await screen.findByTestId('files-panel')
    expect(screen.getByTestId('terminal-panel')).toBe(terminal)
    expect(terminal).toHaveAttribute('data-active', 'false')

    rerender(<RightDock {...props} open={false} workdir="/tmp/other-project" />)
    expect(screen.getByTestId('terminal-panel')).toBe(terminal)
    expect(terminal).toHaveAttribute('data-workdir', '/tmp/other-project')

    rerender(<RightDock {...props} />)
    expect(screen.getByTestId('terminal-panel')).toBe(terminal)
    expect(terminal).toHaveAttribute('data-active', 'true')
  })

  it('delivers the initial preview and reveal and preserves file search across tabs', async () => {
    const props = dockProps({ open: false })
    const { rerender } = render(<RightDock {...props} />)
    expect(screen.getByTestId('files-panel')).toHaveAttribute('data-active', 'false')

    rerender(
      <RightDock
        {...props}
        open
        previewRequest={{ kind: 'markdown', title: 'Plan', text: 'First preview', nonce: 1 }}
        revealRequest={{ path: 'src/main.ts', nonce: 1 }}
      />,
    )
    const files = await screen.findByTestId('files-panel')
    expect(files).toHaveTextContent('First preview')
    expect(files).toHaveTextContent('src/main.ts')
    fireEvent.change(screen.getByLabelText('File search'), { target: { value: 'pending search' } })

    rerender(<RightDock {...props} open activeTab="git" />)
    await screen.findByTestId('git-panel')
    expect(screen.getByTestId('files-panel')).toBe(files)
    expect(files).toHaveAttribute('data-active', 'false')

    rerender(<RightDock {...props} open />)
    expect(screen.getByTestId('files-panel')).toBe(files)
    expect(screen.getByLabelText('File search')).toHaveValue('pending search')
  })
})
