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
  itemOverrides: {} as Record<string, boolean>,
  generateCommitMessage: vi.fn(),
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
    gitPublish: vi.fn(),
    gitPublishReadiness: vi.fn(),
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
vi.mock('../api', () => ({ chatApi: { getBranches: mocks.getBranches, generateCommitMessage: mocks.generateCommitMessage } }))
vi.mock('../persistence', () => ({
  getRememberedEditor: () => null,
  rememberEditor: vi.fn(),
  getWorkspacePanelOpen: (id: string) => mocks.panelChoice.get(id) ?? null,
  rememberWorkspacePanelOpen: (id: string, open: boolean) => { mocks.panelChoice.set(id, open) },
  getWorkspaceItemOverrides: () => mocks.itemOverrides,
  rememberWorkspaceItemOverrides: (next: Record<string, boolean>) => { mocks.itemOverrides = next },
}))
vi.mock('./WorkspaceUsageSection', () => ({
  WorkspaceUsageSection: () => <div data-testid="usage-section" />,
  WorkspaceContextSection: () => <div data-testid="context-section" />,
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
  mocks.itemOverrides = {}
  mocks.generateCommitMessage.mockRejectedValue(new Error('no model'))
  mocks.dockApi.gitPublishReadiness.mockResolvedValue({ ready: true, account: 'me', hint: '' })
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

  it('offers publishing instead of push when the repository has no remote', async () => {
    const panel = openPanel()
    await waitFor(() => expect(within(panel).getByRole('button', { name: '提交' })).toBeEnabled())
    fireEvent.click(within(panel).getByRole('button', { name: 'Git 操作' }))
    const menu = screen.getByRole('menu')
    expect(within(menu).getByRole('menuitem', { name: '提交并推送' })).toBeDisabled()
    expect(within(menu).getByRole('menuitem', { name: '推送' })).toBeDisabled()
    fireEvent.click(within(menu).getByRole('menuitem', { name: '发布仓库' }))
    const name = await within(panel).findByRole('textbox', { name: '仓库名（账号/名称）' })
    expect(name).toHaveValue('me/repo')
    mocks.dockApi.gitPublish.mockResolvedValue({ ok: true, state: { ...ready, upstream: 'origin/main' }, stdout: '', stderr: '', message: '' })
    fireEvent.click(within(panel).getByRole('radio', { name: '公开' }))
    fireEvent.click(within(panel).getByRole('button', { name: '发布' }))
    await waitFor(() => expect(mocks.dockApi.gitPublish).toHaveBeenCalledWith('/repo', 'me/repo', false))
    expect(screen.queryByRole('dialog', { name: '发布仓库' })).not.toBeInTheDocument()
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

  it('hides items chosen from the right-click menu and can turn on usage and context', () => {
    const panel = openPanel()
    expect(within(panel).queryByTestId('usage-section')).not.toBeInTheDocument()
    fireEvent.contextMenu(panel)
    const menu = screen.getByRole('menu', { name: '选择显示的内容' })
    fireEvent.click(within(menu).getByRole('menuitemcheckbox', { name: '任务' }))
    fireEvent.click(within(menu).getByRole('menuitemcheckbox', { name: '上下文窗口' }))
    expect(mocks.itemOverrides).toEqual({ tasks: false, context: true })
    expect(within(panel).queryByRole('button', { name: '任务' })).not.toBeInTheDocument()
    expect(within(panel).getByTestId('context-section')).toBeInTheDocument()
  })

  it('copies a branch name on right-click', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.assign(navigator, { clipboard: { writeText } })
    const panel = openPanel()
    fireEvent.click(await within(panel).findByRole('button', { name: 'main' }))
    const dev = within(screen.getByRole('menu')).getByRole('menuitemradio', { name: /dev/ })
    fireEvent.contextMenu(dev)
    await waitFor(() => expect(dev).toHaveTextContent('已复制'))
    expect(mocks.dockApi.gitSwitchBranch).not.toHaveBeenCalled()
  })

  it('switches the environment of a new chat and locks it once started', () => {
    const onChange = vi.fn()
    const view = render(<WorkspaceActionsMenu workdir="/repo" projectName="Repo" conversationId="c1" dockOpen lang="zh"
      environment={{ worktree: false, worktreeBranch: null, locked: false, onChange }} />)
    fireEvent.click(screen.getByRole('button', { name: /^工作区/ }))
    fireEvent.click(screen.getByRole('button', { name: /本地/ }))
    fireEvent.click(within(screen.getByRole('menu')).getByRole('menuitemradio', { name: /新建 worktree/ }))
    expect(onChange).toHaveBeenCalledWith(true)
    view.rerender(<WorkspaceActionsMenu workdir="/repo" projectName="Repo" conversationId="c1" dockOpen lang="zh"
      environment={{ worktree: true, worktreeBranch: 'kivio/abcd1234', locked: true, onChange }} />)
    expect(screen.getByRole('button', { name: /新建 worktree/ })).toBeDisabled()
    expect(screen.getByText('kivio/abcd1234')).toBeInTheDocument()
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

  it('commits in place with a model-written message and falls back to file names', async () => {
    mocks.generateCommitMessage.mockResolvedValueOnce('Fix the login redirect')
    const panel = openPanel()
    fireEvent.click(await within(panel).findByRole('button', { name: '提交' }))
    await waitFor(() => expect(mocks.dockApi.gitCommit).toHaveBeenCalledWith('/repo', 'Fix the login redirect'))
    expect(mocks.generateCommitMessage).toHaveBeenCalledWith('/repo', 'c1')
    expect(screen.queryByRole('dialog', { name: '提交' })).not.toBeInTheDocument()
    expect(await within(panel).findByRole('status')).toHaveTextContent('已提交')
  })

  it('falls back to a file-name message when the model is unavailable', async () => {
    const panel = openPanel()
    fireEvent.click(await within(panel).findByRole('button', { name: '提交' }))
    await waitFor(() => expect(mocks.dockApi.gitCommit).toHaveBeenCalledWith('/repo', 'Update a.txt'))
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
