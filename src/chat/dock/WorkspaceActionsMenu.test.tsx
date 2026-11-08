import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { GitRepoState } from '../../api/dockContracts'
import { WorkspaceActionsMenu } from './WorkspaceActionsMenu'

const mocks = vi.hoisted(() => ({
  badge: { state: null as GitRepoState | null, diffStat: null, loading: false },
  badgeWorkdirs: [] as string[],
  applyMutationState: vi.fn(),
  refresh: vi.fn().mockResolvedValue(undefined),
  tasks: [] as Array<{ status: string; startedAtMs: number }>,
  getBranches: vi.fn(),
  panelChoice: new Map<string, boolean>(),
  hiddenItems: [] as string[],
  dockApi: {
    availableEditors: vi.fn(),
    gitBranches: vi.fn(),
    gitSwitchBranch: vi.fn(),
    gitCreateBranch: vi.fn(),
    gitPull: vi.fn(),
    gitStageAll: vi.fn(),
    gitCommit: vi.fn(),
    gitPush: vi.fn(),
    gitInit: vi.fn(),
    openInEditor: vi.fn(),
    fsOpenPath: vi.fn(),
  },
}))

vi.mock('./api', () => ({ dockApi: mocks.dockApi }))
vi.mock('./useGitBadge', () => ({
  useGitBadge: (workdir: string) => {
    mocks.badgeWorkdirs.push(workdir)
    return { ...mocks.badge, applyMutationState: mocks.applyMutationState, refresh: mocks.refresh }
  },
}))
vi.mock('../useBackgroundTasks', () => ({ useBackgroundTasks: () => mocks.tasks }))
vi.mock('../useSubagentRoster', () => ({ useSubagentRoster: () => ({ roster: null, refresh: vi.fn() }) }))
vi.mock('../AgentAvatar', () => ({ AgentAvatar: () => null }))
vi.mock('../AgentProgress', () => ({ AgentStatus: () => null }))
vi.mock('../api', () => ({ chatApi: { getBranches: mocks.getBranches } }))
vi.mock('../persistence', () => ({
  getRememberedEditor: () => null,
  rememberEditor: vi.fn(),
  getWorkspacePanelOpen: (id: string) => mocks.panelChoice.get(id) ?? null,
  rememberWorkspacePanelOpen: (id: string, open: boolean) => { mocks.panelChoice.set(id, open) },
  getWorkspaceHiddenItems: () => mocks.hiddenItems,
  rememberWorkspaceHiddenItems: (items: string[]) => { mocks.hiddenItems = items },
}))

const ready: GitRepoState = {
  status: 'ready', repoRoot: '/repo', head: 'main', upstream: null,
  ahead: 0, behind: 0, stashCount: 0, error: null,
  entries: [{ path: 'a.txt', oldPath: null, indexStatus: ' ', worktreeStatus: 'M', kind: 'modified', staged: false, conflicted: false, untracked: false }],
}

function openPanel(props: Partial<Parameters<typeof WorkspaceActionsMenu>[0]> = {}) {
  render(<WorkspaceActionsMenu workdir="/repo" projectName="Repo" conversationId="c1" dockOpen lang="zh" {...props} />)
  fireEvent.click(screen.getByRole('button', { name: /^工作区/ }))
  return screen.getByRole('dialog', { name: '工作区' })
}

beforeEach(() => {
  mocks.badge = { state: ready, diffStat: null, loading: false }
  mocks.badgeWorkdirs = []
  mocks.tasks = []
  mocks.panelChoice.clear()
  mocks.hiddenItems = []
  mocks.getBranches.mockResolvedValue([])
  mocks.dockApi.gitStageAll.mockResolvedValue({ ok: true, state: ready, stdout: '', stderr: '', message: '' })
  mocks.dockApi.gitCommit.mockResolvedValue({ ok: true, state: { ...ready, entries: [] }, stdout: '', stderr: '', message: '' })
  mocks.dockApi.availableEditors.mockResolvedValue(['vscode'])
  mocks.dockApi.gitBranches.mockResolvedValue({
    branches: [{ name: 'main', current: true, upstream: null, ahead: 0, behind: 0 }, { name: 'dev', current: false, upstream: null, ahead: 0, behind: 0 }],
    remotes: [],
    remoteBranches: [],
    defaultBranch: 'main',
  })
  mocks.dockApi.gitSwitchBranch.mockResolvedValue({ ok: true, state: { ...ready, head: 'dev' }, stdout: '', stderr: '', message: '' })
  HTMLDialogElement.prototype.showModal ??= function () { this.setAttribute('open', '') }
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('WorkspaceActionsMenu', () => {
  it('reads Git only while the panel is visible', () => {
    render(<WorkspaceActionsMenu workdir="/repo" projectName="Repo" conversationId="c1" dockOpen lang="zh" />)
    expect(mocks.badgeWorkdirs.every((workdir) => workdir === '')).toBe(true)
    fireEvent.click(screen.getByRole('button', { name: /^工作区/ }))
    expect(mocks.badgeWorkdirs.at(-1)).toBe('/repo')
  })

  it('hides Git for chats without a project', () => {
    const panel = openPanel({ projectName: null })
    expect(within(panel).queryByText('main')).not.toBeInTheDocument()
    expect(within(panel).queryByText('查看变更')).not.toBeInTheDocument()
    expect(mocks.badgeWorkdirs.every((workdir) => workdir === '')).toBe(true)
  })

  it('only offers commit when the repository has no remote', async () => {
    const panel = openPanel()
    await waitFor(() => expect(within(panel).getByRole('button', { name: '提交' })).toBeEnabled())
    fireEvent.click(within(panel).getByRole('button', { name: 'Git 操作' }))
    const menu = screen.getByRole('menu')
    expect(within(menu).getByRole('menuitem', { name: '提交并推送' })).toBeDisabled()
    expect(within(menu).getByRole('menuitem', { name: '推送' })).toBeDisabled()
    expect(within(menu).getByText('仓库没有配置远程，只能本地提交')).toBeInTheDocument()
  })

  it('opens the branch list from the branch name and reloads it after switching', async () => {
    const panel = openPanel()
    fireEvent.click(await within(panel).findByRole('button', { name: 'main' }))
    fireEvent.click(within(screen.getByRole('menu')).getByRole('menuitemradio', { name: 'dev' }))
    await waitFor(() => expect(mocks.dockApi.gitSwitchBranch).toHaveBeenCalledWith('/repo', 'dev'))
    await waitFor(() => expect(mocks.dockApi.gitBranches).toHaveBeenCalledTimes(2))
    expect(mocks.applyMutationState).toHaveBeenCalledWith(expect.objectContaining({ head: 'dev' }))
  })

  it('lists remote branches, filters by search and offers to create a missing branch', async () => {
    mocks.dockApi.gitBranches.mockResolvedValue({
      branches: [{ name: 'main', current: true, upstream: 'origin/main', ahead: 0, behind: 0 }],
      remotes: ['origin'],
      remoteBranches: [{ name: 'origin/feat/freeroute', remote: 'origin' }],
      defaultBranch: 'main',
    })
    mocks.dockApi.gitCreateBranch.mockResolvedValue({ ok: true, state: { ...ready, head: 'new-thing' }, stdout: '', stderr: '', message: '' })
    const panel = openPanel()
    fireEvent.click(await within(panel).findByRole('button', { name: 'main' }))
    const menu = screen.getByRole('menu')
    expect(within(menu).getByRole('menuitemradio', { name: /origin\/feat\/freeroute/ })).toHaveTextContent('远程')
    expect(within(menu).getByRole('menuitemradio', { name: /main/ })).toHaveTextContent('当前')
    fireEvent.change(within(menu).getByRole('textbox', { name: '搜索分支…' }), { target: { value: 'new thing' } })
    fireEvent.click(within(menu).getByRole('menuitem', { name: '创建分支「new-thing」' }))
    await waitFor(() => expect(mocks.dockApi.gitCreateBranch).toHaveBeenCalledWith('/repo', 'new-thing'))
  })

  it('hides items chosen from the right-click menu', () => {
    const panel = openPanel()
    fireEvent.contextMenu(panel)
    fireEvent.click(within(screen.getByRole('menu', { name: '选择显示的内容' })).getByRole('menuitemcheckbox', { name: '任务' }))
    expect(mocks.hiddenItems).toEqual(['tasks'])
    expect(within(panel).queryByRole('button', { name: '任务' })).not.toBeInTheDocument()
  })

  it('shows a loading placeholder until the remote list arrives', async () => {
    let resolve!: (value: unknown) => void
    mocks.dockApi.gitBranches.mockReturnValueOnce(new Promise((done) => { resolve = done }))
    const panel = openPanel()
    expect(within(panel).getByRole('status', { name: '加载中…' })).toBeInTheDocument()
    expect(within(panel).queryByRole('button', { name: 'main' })).not.toBeInTheDocument()
    await act(async () => { resolve({ branches: [], remotes: ['origin'], remoteBranches: [], defaultBranch: '' }) })
    expect(within(panel).getByRole('button', { name: '提交并推送' })).toBeEnabled()
  })

  it('commits in place without a dialog and shows the result on the button', async () => {
    const panel = openPanel()
    fireEvent.click(await within(panel).findByRole('button', { name: '提交' }))
    await waitFor(() => expect(mocks.dockApi.gitCommit).toHaveBeenCalledWith('/repo', 'Update a.txt'))
    expect(screen.queryByRole('dialog', { name: '提交' })).not.toBeInTheDocument()
    expect(await within(panel).findByRole('status')).toHaveTextContent('已提交')
  })

  it('switches history branches from an in-panel dropdown', async () => {
    mocks.getBranches.mockResolvedValue([
      { id: 'b1', label: '原始分支', active: true },
      { id: 'b2', label: '编辑后', active: false },
    ])
    const onSwitch = vi.fn().mockResolvedValue(true)
    const panel = openPanel({ historyBranches: { disabled: false, onSwitch } })
    fireEvent.click(await within(panel).findByRole('button', { name: /原始分支/ }))
    fireEvent.click(within(screen.getByRole('menu')).getByRole('menuitemradio', { name: '编辑后' }))
    await waitFor(() => expect(onSwitch).toHaveBeenCalledWith('b2'))
    expect(screen.queryByRole('dialog', { name: '历史分支' })).not.toBeInTheDocument()
  })

  it('shows running tasks with a count badge', () => {
    mocks.tasks = [{ status: 'running', startedAtMs: 1 }]
    const panel = openPanel()
    expect(within(panel).getByLabelText('1 个后台任务运行中')).toHaveTextContent('1')
  })

  it('stays collapsed for a blank conversation and remembers the choice per conversation', () => {
    const props = { workdir: '/repo', projectName: 'Repo', dockOpen: false, lang: 'zh' as const }
    const view = render(<WorkspaceActionsMenu {...props} conversationId="new" blank />)
    expect(screen.queryByRole('complementary', { name: '工作区' })).not.toBeInTheDocument()
    view.rerender(<WorkspaceActionsMenu {...props} conversationId="old" />)
    expect(screen.getByRole('complementary', { name: '工作区' })).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: /^工作区/ }))
    expect(mocks.panelChoice.get('old')).toBe(false)
  })

  it('anchors the popover to the trigger and plays the exit animation before unmounting', () => {
    vi.useFakeTimers()
    try {
      const panel = openPanel()
      expect(panel).toHaveClass('chat-motion-popover')
      expect(panel.style.right).not.toBe('')
      fireEvent.click(screen.getByRole('button', { name: /^工作区/ }))
      expect(panel).toHaveClass('chat-motion-popover-out')
      expect(panel).toBeInTheDocument()
      act(() => { vi.advanceTimersByTime(200) })
      expect(panel).not.toBeInTheDocument()
    } finally {
      vi.useRealTimers()
    }
  })

  it('closes the popover with Escape but lets an open dropdown consume it first', async () => {
    const panel = openPanel()
    fireEvent.click(await within(panel).findByRole('button', { name: 'main' }))
    expect(screen.getByRole('menu')).toBeInTheDocument()
    act(() => { fireEvent.keyDown(window, { key: 'Escape' }) })
    expect(screen.queryByRole('menu')).not.toBeInTheDocument()
    expect(screen.getByRole('dialog', { name: '工作区' })).toBeInTheDocument()
    act(() => { fireEvent.keyDown(document, { key: 'Escape' }) })
    expect(screen.queryByRole('dialog', { name: '工作区' })).not.toBeInTheDocument()
  })
})
