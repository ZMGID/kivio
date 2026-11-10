import { act, fireEvent, render, screen, within } from '@testing-library/react'
import type { ComponentProps } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { api, type BackgroundTaskInfo } from '../../api/tauri'
import { subagentsApi } from '../../api/subagents'
import { useRightDock } from '../hooks/useRightDock'
import { requestDockDiffPreview, requestDockMarkdownPreview, requestDockPreview, requestDockSubagent } from './dockPreview'
import { SubagentLifecycleEvent } from '../SubagentTimelineEvents'
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
vi.mock('./api', () => ({ dockApi: { resolveCwd: vi.fn(async () => '/tmp/project') } }))
vi.mock('../../api/settingsCache', () => ({
  peekSettings: () => null,
  subscribeSettings: () => () => {},
}))
vi.mock('../../api/tauri', () => ({ api: {
  chatListBackgroundTasks: vi.fn(),
  chatKillBackgroundCommand: vi.fn(),
  chatStopExternalBackgroundTask: vi.fn(),
  chatClearFinishedBackgroundTasks: vi.fn(),
} }))

const command: BackgroundTaskInfo = {
  id: 'command-1', source: 'builtin', kind: 'bash', title: 'node server.js',
  status: 'running', elapsedSecs: 1, startedAtMs: 1,
}

beforeEach(() => {
  vi.spyOn(document, 'hidden', 'get').mockReturnValue(false)
  vi.mocked(api.chatListBackgroundTasks).mockReset().mockResolvedValue([])
  vi.mocked(api.chatKillBackgroundCommand).mockReset().mockResolvedValue(undefined)
  vi.mocked(api.chatStopExternalBackgroundTask).mockReset().mockResolvedValue(undefined)
  vi.mocked(api.chatClearFinishedBackgroundTasks).mockReset().mockResolvedValue(undefined)
})

afterEach(() => {
  vi.restoreAllMocks()
})

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
  const dock = useRightDock({ conversationId: 'conv-1', projectId: null, agentRuntimeKind: 'builtin' })
  return (
    <>
      <button onClick={dock.toggle}>Toggle dock</button>
      <button onClick={dock.openGit}>Open Git externally</button>
      <button onClick={dock.openTasks}>Open tasks externally</button>
      <button onClick={() => dock.revealInTree('src/reveal.ts')}>Reveal in tree</button>
      <SubagentLifecycleEvent conversationId="conv-1" event={{ agentId: 'Scout', name: 'Review tests',
        status: 'completed', parentToolCallId: 'task', afterSegmentId: null, afterOrder: null, textChars: null }} />
      {mounted && (
        <RightDock
          {...dockProps({
            open: dock.open,
            activeTab: dock.tab,
            openTabRequest: dock.openTabRequest,
            onOpenTabRequestHandled: dock.onOpenTabRequestHandled,
            workdir: dock.workdir,
            treeExpanded: dock.treeExpanded,
            revealRequest: dock.reveal,
            previewRequest: dock.preview,
            subagent: dock.subagent.selection,
            onBackFromSubagent: dock.subagent.close,
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
  it('lists subagents in the existing Tasks tab', async () => {
    const snapshot = vi.spyOn(subagentsApi, 'snapshot').mockResolvedValue({
      runningCount: 1,
      agents: [{
        id: 'Scout', displayName: 'Review tests', parentId: 'Main', generation: 1, agentType: 'scout',
        status: 'running', depth: 1, lastActivity: 1, activity: 'Reading logs',
        model: null, progress: null, revivable: true, reviveError: null,
      }],
    })
    render(<RightDock {...dockProps({ activeTab: 'tasks' })} />)
    expect(snapshot).not.toHaveBeenCalled()
    fireEvent.click(screen.getByText('任务'))
    expect(await screen.findByRole('region', { name: '子代理任务' })).toHaveTextContent('Review tests')
    expect(screen.getByRole('button', { name: /Review tests/ })).not.toHaveAttribute('aria-haspopup')
    expect(snapshot).toHaveBeenCalledWith('conv-1')
  })
  it('opens the same read-only sidebar from the chat and task list', async () => {
    vi.spyOn(subagentsApi, 'snapshot').mockResolvedValue({ runningCount: 0, agents: [{
      id: 'Scout', displayName: 'Review tests', parentId: 'Main', generation: 1, agentType: 'scout',
      status: 'idle', depth: 1, lastActivity: 1, activity: null,
      model: 'hidden/model', progress: null, revivable: true, reviveError: null,
    }] })
    vi.spyOn(subagentsApi, 'transcript').mockResolvedValue({
      id: 'Scout', generation: 1, cursor: { fileId: 'file', offset: 1 }, reset: true, hasMore: false,
      entries: [{ id: 'reply', kind: 'message', role: 'assistant', content: '**Checked the tests.**', timestamp: null }],
    })
    render(<DockHarness />)
    act(() => requestDockSubagent({ conversationId: 'another', agentId: 'Scout' }))
    expect(screen.queryByRole('region', { name: 'Review tests' })).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Review tests 已完成' }))
    expect(await screen.findByText('Checked the tests.')).toBeVisible()
    expect(screen.getByRole('region', { name: 'Review tests' }).closest('aside')).not.toBeNull()
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    expect(screen.queryByRole('textbox', { name: '发送给代理的消息' })).not.toBeInTheDocument()
    expect(within(screen.getByRole('region', { name: 'Review tests' })).getByText('hidden/model')).toBeVisible()
    fireEvent.click(screen.getByRole('button', { name: '返回' }))
    fireEvent.click(within(await screen.findByRole('region', { name: '子代理任务' })).getByRole('button', { name: /^Review tests/ }))
    expect(await screen.findByText('Checked the tests.')).toBeVisible()
  })
  it('opens panels from the window toolbar and removes its controls when collapsed', () => {
    const host = document.createElement('div')
    document.body.appendChild(host)
    try {
      const props = dockProps({ toolbarHost: host })
      const view = render(<RightDock {...props} />)
      expect(host.querySelector('[aria-expanded]')).toBeNull()
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

  it('shows and controls background processes in the task list', async () => {
    const external: BackgroundTaskInfo = {
      id: 'external-1', source: 'external', kind: 'local_agent', title: 'Indexer',
      status: 'running', elapsedSecs: 2, startedAtMs: 2, conversationId: 'conv-1',
    }
    const finished: BackgroundTaskInfo = { ...command, id: 'finished-1', title: 'Older task', status: 'completed' }
    vi.mocked(api.chatListBackgroundTasks).mockResolvedValue([command, external, finished])
    render(<RightDock {...dockProps({ activeTab: 'tasks' })} />)
    expect(api.chatListBackgroundTasks).not.toHaveBeenCalled()
    fireEvent.click(screen.getByText('任务'))
    expect(await screen.findByText('node server.js')).toBeVisible()
    expect(screen.getByText('Indexer')).toBeVisible()
    expect(screen.getByText('Older task')).toBeVisible()

    await act(async () => { fireEvent.click(screen.getByRole('button', { name: '终止 node server.js' })) })
    expect(api.chatKillBackgroundCommand).toHaveBeenCalledWith('command-1')
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: '终止 Indexer' })) })
    expect(api.chatStopExternalBackgroundTask).toHaveBeenCalledWith('conv-1', 'external-1')
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: '清空' })) })
    expect(api.chatClearFinishedBackgroundTasks).toHaveBeenCalledWith('conv-1')
    expect(screen.getByText('没有后台任务。agent 后台运行的命令和子代理会出现在这里。')).toBeVisible()
  })

  it('updates background processes when the conversation changes and shows the regular empty state', async () => {
    const second: BackgroundTaskInfo = { ...command, id: 'command-2', title: 'npm test' }
    vi.mocked(api.chatListBackgroundTasks).mockImplementation(async (id) =>
      id === 'conv-1' ? [command] : id === 'conv-2' ? [second] : [],
    )
    const props = dockProps({ activeTab: 'tasks' })
    const view = render(<RightDock {...props} />)
    fireEvent.click(screen.getByText('任务'))
    expect(await screen.findByText('node server.js')).toBeVisible()
    view.rerender(<RightDock {...props} conversationId="conv-2" />)
    expect(await screen.findByText('npm test')).toBeVisible()
    expect(screen.queryByText('node server.js')).toBeNull()
    const count = vi.mocked(api.chatListBackgroundTasks).mock.calls.length
    view.rerender(<RightDock {...props} conversationId={null} />)
    expect(screen.getByText('没有后台任务。agent 后台运行的命令和子代理会出现在这里。')).toBeVisible()
    expect(api.chatListBackgroundTasks).toHaveBeenCalledTimes(count)
    view.rerender(<RightDock {...props} conversationId="conv-empty" />)
    await act(async () => {})
    expect(api.chatListBackgroundTasks).toHaveBeenCalledWith('conv-empty')
    expect(screen.getByText('没有后台任务。agent 后台运行的命令和子代理会出现在这里。')).toBeVisible()
  })

  it('polls only while the task tab and dock are open', async () => {
    vi.useFakeTimers()
    try {
      const props = dockProps({ activeTab: 'tasks', conversationId: 'conv-poll' })
      const view = render(<RightDock {...props} />)
      expect(api.chatListBackgroundTasks).not.toHaveBeenCalled()
      fireEvent.click(screen.getByText('任务'))
      await act(async () => { await Promise.resolve() })
      expect(api.chatListBackgroundTasks).toHaveBeenCalledTimes(1)
      await act(async () => { await vi.advanceTimersByTimeAsync(2500) })
      expect(api.chatListBackgroundTasks).toHaveBeenCalledTimes(2)

      view.rerender(<RightDock {...props} open={false} />)
      await act(async () => { await vi.advanceTimersByTimeAsync(7500) })
      expect(api.chatListBackgroundTasks).toHaveBeenCalledTimes(2)
      view.rerender(<RightDock {...props} />)
      await act(async () => { await Promise.resolve() })
      expect(api.chatListBackgroundTasks).toHaveBeenCalledTimes(3)

      view.rerender(<RightDock {...props} activeTab="files" openTabRequest={{ tab: 'files' }} />)
      await act(async () => { await vi.advanceTimersByTimeAsync(7500) })
      expect(api.chatListBackgroundTasks).toHaveBeenCalledTimes(3)
      view.unmount()
    } finally {
      vi.useRealTimers()
    }
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

  it('reopens tasks on repeated commands', async () => {
    vi.mocked(api.chatListBackgroundTasks).mockResolvedValue([command])
    render(<DockHarness />)
    await act(async () => {})
    fireEvent.click(screen.getByText('Open tasks externally'))
    expect(await screen.findByText('node server.js')).toBeVisible()
    fireEvent.click(screen.getByRole('button', { name: '打开面板' }))
    expect(screen.getByRole('button', { name: '任务 k' })).toBeVisible()
    fireEvent.click(screen.getByText('Open tasks externally'))
    expect(await screen.findByText('node server.js')).toBeVisible()
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
