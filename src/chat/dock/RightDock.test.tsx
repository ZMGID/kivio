import { act, fireEvent, render, screen } from '@testing-library/react'
import { useRef, type ComponentProps } from 'react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { useRightDock } from '../hooks/useRightDock'
import { requestDockDiffPreview, requestDockMarkdownPreview, requestDockPreview, requestDockSubAgent } from './dockPreview'
import { RightDock } from './RightDock'

const { loadedPanels } = vi.hoisted(() => ({ loadedPanels: [] as string[] }))

vi.mock('./FileTreePanel', () => {
  loadedPanels.push('files')
  return {
    FileTreePanel: ({ active, previewRequest, revealPath }: {
      active: boolean
      previewRequest: { kind: string; nonce: number; path?: string; text?: string } | null
      revealPath: string | null
    }) => (
      <div
        data-testid="files-panel"
        data-active={active}
        data-preview-kind={previewRequest?.kind}
        data-preview-path={previewRequest?.path}
        data-preview-nonce={previewRequest?.nonce}
      >
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
vi.mock('./BackgroundTasksPanel', () => ({ BackgroundTasksPanel: () => <div>node server.js</div> }))
vi.mock('./api', () => ({ dockApi: { resolveCwd: vi.fn(async () => '/tmp/project') } }))
vi.mock('../../api/settingsCache', () => ({
  peekSettings: () => null,
  subscribeSettings: () => () => {},
}))
vi.mock('../../api/tauri', () => ({ api: { chatSubagentControl: vi.fn(async (_id, args) => {
  const child = { id: 'child', name: 'Environment', sequence: 1, profile: { model: 'test' }, runs: [], history: [], messages: [], tools: [] }
  if (args.operation === 'list') return { sequence: 1, agents: [child] }
  if (args.operation === 'wait') return new Promise(() => {})
  return child
}) } }))

function dockProps(overrides: Partial<ComponentProps<typeof RightDock>> = {}): ComponentProps<typeof RightDock> {
  return {
    open: true,
    width: 360,
    activeTab: 'files',
    openTabRequest: null,
    workdir: '/tmp/project',
    lang: 'zh',
    conversationId: 'conv-1',
    treeExpanded: [],
    revealRequest: null,
    previewRequest: null,
    onToggleTab: vi.fn(),
    onOpenTabRequestHandled: vi.fn(),
    onWidthChange: vi.fn(),
    onClose: vi.fn(),
    onTreeExpandedChange: vi.fn(),
    onRevealInTree: vi.fn(),
    ...overrides,
  }
}

function DockHarness({ mounted = true }: { mounted?: boolean }) {
  const currentConversationIdRef = useRef<string | null>('conv-1')
  const dock = useRightDock({ conversationId: 'conv-1', projectId: null, agentRuntimeKind: 'builtin', currentConversationIdRef })
  return (
    <>
      <button onClick={dock.toggle}>Toggle dock</button>
      <button onClick={dock.openGit}>Open Git externally</button>
      <button onClick={dock.openTasks}>Open tasks externally</button>
      <button onClick={() => dock.revealInTree('src/reveal.ts')}>Reveal in tree</button>
      {mounted && (
        <RightDock
          {...dockProps({
            open: dock.open,
            activeTab: dock.tab,
            openTabRequest: dock.openTabRequest,
            onOpenTabRequestHandled: dock.onOpenTabRequestHandled,
            workdir: dock.workdir,
            subAgentRequest: dock.subAgentRequest,
            treeExpanded: dock.treeExpanded,
            revealRequest: dock.reveal,
            previewRequest: dock.preview,
            onToggleTab: dock.setTab,
            onWidthChange: dock.setWidth,
            onClose: dock.close,
            onTreeExpandedChange: dock.setTreeExpanded,
            onRevealInTree: dock.revealInTree,
          })}
        />
      )}
    </>
  )
}

describe('RightDock tabs', () => {
  it('opens panels from the window toolbar and removes its controls when collapsed', () => {
    const host = document.createElement('div')
    document.body.appendChild(host)
    try {
      const props = dockProps({ toolbarHost: host })
      const view = render(<RightDock {...props} />)
      fireEvent.click(screen.getByText('文件'))
      expect(screen.getByTestId('files-panel')).toHaveAttribute('data-active', 'true')
      fireEvent.click(host.querySelector<HTMLButtonElement>('[aria-label="打开面板"]')!)
      expect(screen.getByRole('button', { name: '终端 t' })).toBeVisible()
      view.rerender(<RightDock {...props} open={false} />)
      expect(host.querySelector('button')).toBeNull()
      view.rerender(<RightDock {...props} open />)
      expect(host.querySelector('[aria-label="打开面板"]')).not.toBeNull()
    } finally {
      host.remove()
    }
  })

  it('shows parent commands only in the task list, never inside a child detail', async () => {
    render(<RightDock {...dockProps({ activeTab: 'tasks' })} />)
    fireEvent.click(screen.getByText('任务'))
    expect(screen.getByText('node server.js')).toBeVisible()
    fireEvent.click(await screen.findByText('Environment'))
    expect(screen.queryByText('node server.js')).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: '返回任务列表' }))
    expect(screen.getByText('node server.js')).toBeVisible()
  })

  it('keeps parent commands out of direct child reveals and restores the list on conversation changes', async () => {
    const props = dockProps({ activeTab: 'tasks', subAgentRequest: { conversationId: 'conv-1', agentId: 'child', nonce: 1 } })
    const view = render(<RightDock {...props} />)
    fireEvent.click(screen.getByText('任务'))
    await screen.findByRole('button', { name: '返回任务列表' })
    expect(screen.queryByText('node server.js')).toBeNull()
    view.rerender(<RightDock {...props} conversationId="conv-2" />)
    expect(screen.queryByRole('button', { name: '返回任务列表' })).toBeNull()
    expect(screen.getByText('node server.js')).toBeVisible()
  })
  it('renders panels immediately on opening without waiting for hover or an import', () => {
    const props = dockProps({ open: false })
    const { rerender } = render(<RightDock {...props} />)
    expect(loadedPanels).toHaveLength(3)
    expect(loadedPanels).toEqual(expect.arrayContaining(['files', 'git', 'terminal']))
    expect(screen.queryByTestId('terminal-panel')).toBeNull()

    rerender(<RightDock {...props} open />)
    fireEvent.click(screen.getByText('文件'))
    expect(screen.getByTestId('files-panel')).toHaveAttribute('data-active', 'true')
    expect(screen.queryByTestId('terminal-panel')).toBeNull()

    rerender(<RightDock {...props} open activeTab="git" openTabRequest={{ tab: 'git' }} />)
    expect(screen.getByTestId('git-panel')).toBeVisible()

    rerender(<RightDock {...props} open activeTab="terminal" openTabRequest={{ tab: 'terminal' }} />)
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
    fireEvent.click(screen.getByText('终端'))
    expect(await screen.findByTestId('terminal-panel')).toHaveAttribute('data-active', 'true')
  })

  it('keeps an opened terminal mounted across tab changes and closing the dock', async () => {
    const props = dockProps({ activeTab: 'terminal' })
    const { rerender } = render(<RightDock {...props} />)
    fireEvent.click(screen.getByText('终端'))
    const terminal = await screen.findByTestId('terminal-panel')

    rerender(<RightDock {...props} activeTab="files" openTabRequest={{ tab: 'files' }} />)
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
        openTabRequest={{ tab: 'files' }}
        previewRequest={{ kind: 'markdown', title: 'Plan', text: 'First preview', nonce: 1 }}
        revealRequest={{ path: 'src/main.ts', nonce: 1 }}
      />,
    )
    const files = await screen.findByTestId('files-panel')
    expect(files).toBeVisible()
    expect(files).toHaveAttribute('data-active', 'true')
    expect(files).toHaveTextContent('First preview')
    expect(files).toHaveTextContent('src/main.ts')
    fireEvent.change(screen.getByLabelText('File search'), { target: { value: 'pending search' } })

    rerender(<RightDock {...props} open activeTab="git" openTabRequest={{ tab: 'git' }} />)
    await screen.findByTestId('git-panel')
    expect(screen.getByTestId('files-panel')).toBe(files)
    expect(files).toHaveAttribute('data-active', 'false')

    rerender(<RightDock {...props} open openTabRequest={{ tab: 'files' }} />)
    expect(screen.getByTestId('files-panel')).toBe(files)
    expect(screen.getByLabelText('File search')).toHaveValue('pending search')
  })

  it('makes the collapsed dock inert so Tab cannot focus its off-screen buttons', () => {
    const props = dockProps({ open: false })
    const view = render(<RightDock {...props} />)
    const shell = view.container.querySelector('aside.chat-dock-shell') as HTMLElement
    expect(shell.inert).toBe(true)
    view.rerender(<RightDock {...props} open />)
    expect(shell.inert).toBe(false)
  })
})

describe('RightDock external requests', () => {
  beforeEach(() => {
    window.localStorage.clear()
  })

  it('shows the launcher on ordinary open but displays the first and repeated files previews', async () => {
    render(<DockHarness />)
    await act(async () => {})
    const files = screen.getByTestId('files-panel')
    fireEvent.click(screen.getByText('Toggle dock'))
    expect(screen.getByRole('button', { name: '文件 f' })).toBeVisible()
    expect(files).not.toBeVisible()
    fireEvent.click(screen.getByText('Toggle dock'))

    act(() => requestDockPreview('src/first.ts'))
    expect(files).toBeVisible()
    expect(files).toHaveAttribute('data-active', 'true')
    expect(files).toHaveAttribute('data-preview-kind', 'file')
    expect(files).toHaveAttribute('data-preview-path', 'src/first.ts')
    expect(files).toHaveTextContent('src/first.ts')

    fireEvent.click(screen.getByRole('button', { name: '打开面板' }))
    expect(screen.getByRole('button', { name: '文件 f' })).toBeVisible()
    expect(files).toHaveAttribute('data-active', 'false')
    act(() => requestDockPreview('src/second.ts'))
    expect(files).toBeVisible()
    expect(files).toHaveAttribute('data-active', 'true')
    expect(files).toHaveAttribute('data-preview-path', 'src/second.ts')
    expect(files).toHaveAttribute('data-preview-nonce', '2')
  })

  it('reopens files for diff, markdown and repeated tree reveal requests', async () => {
    render(<DockHarness />)
    await act(async () => {})
    act(() => requestDockDiffPreview({ title: 'Changes', patch: '+added' }))
    const files = screen.getByTestId('files-panel')
    expect(files).toBeVisible()
    expect(files).toHaveAttribute('data-preview-kind', 'diff')

    fireEvent.click(screen.getByRole('button', { name: '打开面板' }))
    act(() => requestDockMarkdownPreview({ title: 'Plan', text: '# plan' }))
    expect(files).toBeVisible()
    expect(files).toHaveTextContent('# plan')

    fireEvent.click(screen.getByRole('button', { name: '打开面板' }))
    fireEvent.click(screen.getByText('Reveal in tree'))
    expect(files).toBeVisible()
    expect(files).toHaveTextContent('src/reveal.ts')
    fireEvent.click(screen.getByRole('button', { name: '打开面板' }))
    fireEvent.click(screen.getByText('Reveal in tree'))
    expect(files).toBeVisible()
  })

  it('reopens an already selected Git tab after showing the launcher', async () => {
    render(<DockHarness />)
    await act(async () => {})
    fireEvent.click(screen.getByText('Open Git externally'))
    expect(screen.getByTestId('git-panel')).toBeVisible()
    fireEvent.click(screen.getByRole('button', { name: '打开面板' }))
    expect(screen.getByTestId('git-panel')).not.toBeVisible()
    fireEvent.click(screen.getByText('Open Git externally'))
    expect(screen.getByTestId('git-panel')).toBeVisible()
  })

  it('reopens tasks on repeated commands and current-conversation subagent requests', async () => {
    render(<DockHarness />)
    await act(async () => {})
    fireEvent.click(screen.getByText('Open tasks externally'))
    expect(screen.getByText('node server.js')).toBeVisible()
    fireEvent.click(screen.getByRole('button', { name: '打开面板' }))
    act(() => requestDockSubAgent({ conversationId: 'other', agentId: 'child' }))
    expect(screen.getByRole('button', { name: '任务 k' })).toBeVisible()
    fireEvent.click(screen.getByText('Open tasks externally'))
    expect(screen.getByText('node server.js')).toBeVisible()

    fireEvent.click(screen.getByRole('button', { name: '打开面板' }))
    act(() => requestDockSubAgent({ conversationId: 'conv-1', agentId: 'child' }))
    expect(await screen.findByRole('button', { name: '返回任务列表' })).toBeVisible()
    fireEvent.click(screen.getByRole('button', { name: '打开面板' }))
    act(() => requestDockSubAgent({ conversationId: 'conv-1', agentId: 'child' }))
    expect(await screen.findByRole('button', { name: '返回任务列表' })).toBeVisible()
  })

  it('keeps the launcher and terminal session across collapse, reopen and explicit requests', async () => {
    render(<DockHarness />)
    await act(async () => {})
    fireEvent.click(screen.getByText('Toggle dock'))
    expect(screen.queryByTestId('terminal-panel')).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: '终端 t' }))
    const terminal = screen.getByTestId('terminal-panel')
    expect(terminal).toHaveAttribute('data-active', 'true')

    fireEvent.click(screen.getByRole('button', { name: '打开面板' }))
    fireEvent.click(screen.getByText('Toggle dock'))
    fireEvent.click(screen.getByText('Toggle dock'))
    expect(screen.getByRole('button', { name: '终端 t' })).toBeVisible()
    expect(screen.getByTestId('terminal-panel')).toBe(terminal)
    expect(terminal).toHaveAttribute('data-active', 'false')

    act(() => requestDockMarkdownPreview({ title: 'Plan', text: '# plan' }))
    expect(screen.getByTestId('files-panel')).toBeVisible()
    expect(screen.getByTestId('terminal-panel')).toBe(terminal)
    fireEvent.click(screen.getByRole('button', { name: '终端' }))
    expect(terminal).toHaveAttribute('data-active', 'true')
    fireEvent.click(screen.getByText('Toggle dock'))
    expect(terminal).toHaveAttribute('data-active', 'false')
    fireEvent.click(screen.getByText('Toggle dock'))
    expect(screen.getByTestId('terminal-panel')).toBe(terminal)
    expect(terminal).toHaveAttribute('data-active', 'true')
  })

  it('delivers a request made before mounting without replaying it on a later mount', async () => {
    const view = render(<DockHarness mounted={false} />)
    await act(async () => {})
    act(() => requestDockMarkdownPreview({ title: 'Plan', text: '# before mount' }))
    view.rerender(<DockHarness />)
    expect(screen.getByTestId('files-panel')).toBeVisible()
    expect(screen.getByTestId('files-panel')).toHaveTextContent('# before mount')

    view.rerender(<DockHarness mounted={false} />)
    view.rerender(<DockHarness />)
    expect(screen.getByRole('button', { name: '文件 f' })).toBeVisible()
    expect(screen.getByTestId('files-panel')).not.toBeVisible()
  })

  it('does not replay a pending request after the dock is manually closed and reopened', async () => {
    const view = render(<DockHarness mounted={false} />)
    await act(async () => {})
    act(() => requestDockMarkdownPreview({ title: 'Plan', text: '# old request' }))
    fireEvent.click(screen.getByText('Toggle dock'))
    fireEvent.click(screen.getByText('Toggle dock'))
    view.rerender(<DockHarness />)
    expect(screen.getByRole('button', { name: '文件 f' })).toBeVisible()
    expect(screen.getByTestId('files-panel')).not.toBeVisible()
  })
})
