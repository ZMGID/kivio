import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { GitRepoState } from '../../api/dockContracts'
import { WorkspaceActionsMenu } from './WorkspaceActionsMenu'
import { ChatRouteKeepAlive } from '../ChatRouteKeepAlive'
import { getNotices } from '../../components/appNoticeStore'
import type { SubagentRoster } from '../../api/subagents'
import type { BackgroundTaskInfo } from '../../api/tauri'

const mocks = vi.hoisted(() => ({
  badge: { state: null as GitRepoState | null, diffStat: null, loading: false },
  badgeWorkdirs: [] as string[],
  applyMutationState: vi.fn(),
  refresh: vi.fn().mockResolvedValue(undefined),
  tasks: [] as BackgroundTaskInfo[],
  stopBackgroundTask: vi.fn(),
  getBranches: vi.fn(),
  panelChoice: new Map<string, boolean>(),
  itemOverrides: {} as Record<string, boolean>,
  rememberedEditor: null as string | null,
  rememberEditor: vi.fn(),
  roster: null as SubagentRoster | null,
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
vi.mock('../useBackgroundTasks', () => ({ useBackgroundTasks: () => mocks.tasks, stopBackgroundTask: mocks.stopBackgroundTask }))
vi.mock('../useSubagentRoster', () => ({ useSubagentRoster: () => ({ roster: mocks.roster, refresh: vi.fn() }) }))
vi.mock('../AgentAvatar', () => ({ AgentAvatar: () => null }))
vi.mock('../AgentProgress', () => ({ AgentStatus: () => null }))
vi.mock('../api', () => ({ chatApi: { getBranches: mocks.getBranches, generateCommitMessage: mocks.generateCommitMessage } }))
vi.mock('../persistence', () => ({
  getRememberedEditor: () => mocks.rememberedEditor,
  rememberEditor: mocks.rememberEditor,
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

const runningTask: BackgroundTaskInfo = {
  id: 'build', source: 'builtin', kind: 'bash', title: 'npm run build', status: 'running',
  summary: 'Building the frontend', elapsedSecs: 72, startedAtMs: 1,
}

function openPanel(props: Partial<Parameters<typeof WorkspaceActionsMenu>[0]> = {}) {
  render(<WorkspaceActionsMenu workdir="/repo" projectName="Repo" conversationId="c1" dockOpen inlineAllowed={false} lang="zh" {...props} />)
  fireEvent.click(screen.getByRole('button', { name: /^工作区/ }))
  return screen.getByRole('dialog', { name: '工作区' })
}

beforeEach(() => {
  mocks.badge = { state: ready, diffStat: null, loading: false }
  mocks.badgeWorkdirs = []
  mocks.tasks = []
  mocks.stopBackgroundTask.mockResolvedValue(undefined)
  mocks.panelChoice.clear()
  mocks.itemOverrides = {}
  mocks.rememberedEditor = null
  mocks.roster = null
  mocks.dockApi.openInEditor.mockResolvedValue(undefined)
  mocks.dockApi.fsOpenPath.mockResolvedValue({ path: '', kind: 'dir' })
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
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

describe('WorkspaceActionsMenu', () => {
  it.each(['center', 'settings'])('removes the inline panel and its menus on %s, restoring the saved choice on return', async (route) => {
    const panel = <WorkspaceActionsMenu workdir="/repo" projectName="Repo" conversationId="c1" dockOpen={false} lang="zh" />
    const view = render(<ChatRouteKeepAlive activeKey="conversation">{panel}</ChatRouteKeepAlive>)
    const workspace = screen.getByRole('complementary', { name: '工作区' })
    fireEvent.click(await within(workspace).findByRole('button', { name: '选择打开方式' }))
    expect(screen.getByRole('menu')).toBeInTheDocument()
    expect(document.documentElement.style.getPropertyValue('--chat-details-inset')).toBe('316px')

    view.rerender(<ChatRouteKeepAlive activeKey={route}><main>{route}</main></ChatRouteKeepAlive>)
    expect(workspace).not.toBeInTheDocument()
    expect(screen.queryByRole('menu')).not.toBeInTheDocument()
    expect(document.documentElement.style.getPropertyValue('--chat-details-inset')).toBe('')

    view.rerender(<ChatRouteKeepAlive activeKey="conversation">{panel}</ChatRouteKeepAlive>)
    expect(screen.getByRole('complementary', { name: '工作区' })).toBeInTheDocument()
    expect(screen.queryByRole('menu')).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: /^工作区/ }))
    view.rerender(<ChatRouteKeepAlive activeKey={route}><main>{route}</main></ChatRouteKeepAlive>)
    view.rerender(<ChatRouteKeepAlive activeKey="conversation">{panel}</ChatRouteKeepAlive>)
    expect(screen.queryByRole('complementary', { name: '工作区' })).not.toBeInTheDocument()
  })

  it('closes a temporary popover when the conversation is cached and leaves it closed on return', () => {
    const panel = <WorkspaceActionsMenu workdir="/repo" projectName="Repo" conversationId="c1" dockOpen={false} inlineAllowed={false} lang="zh" />
    const view = render(<ChatRouteKeepAlive activeKey="conversation">{panel}</ChatRouteKeepAlive>)
    fireEvent.click(screen.getByRole('button', { name: /^工作区/ }))
    const popover = screen.getByRole('dialog', { name: '工作区' })
    view.rerender(<ChatRouteKeepAlive activeKey="center"><main>market</main></ChatRouteKeepAlive>)
    expect(popover).not.toBeInTheDocument()
    view.rerender(<ChatRouteKeepAlive activeKey="conversation">{panel}</ChatRouteKeepAlive>)
    expect(screen.queryByRole('dialog', { name: '工作区' })).not.toBeInTheDocument()
  })

  it('keeps running subagents visible while folding finished ones', () => {
    const baseAgent = { parentId: 'main', generation: 1, agentType: 'scout', depth: 1, lastActivity: 0,
      activity: null, model: null, progress: null, revivable: true, reviveError: null }
    mocks.roster = { agents: [
      { ...baseAgent, id: 'active', displayName: 'Active', status: 'running' },
      { ...baseAgent, id: 'finished', displayName: 'Finished', status: 'parked' },
    ], runningCount: 1 }
    const panel = openPanel()
    expect(within(panel).getByText('Active')).toBeVisible()
    expect(within(panel).getByText('Finished')).not.toBeVisible()
    fireEvent.click(within(panel).getByText(/已结束/))
    expect(within(panel).getByText('Finished')).toBeVisible()
    expect(within(panel).queryByText('已休眠')).not.toBeInTheDocument()
    expect(within(panel).getByRole('button', { name: '停止子代理 Active' })).toBeVisible()
  })

  it.each([
    ['vscode', 'VS Code'], ['cursor', 'Cursor'], ['zed', 'Zed'], ['windsurf', 'Windsurf'],
  ])('shows the %s logo in the menu and primary button and opens that editor', async (id, label) => {
    mocks.dockApi.availableEditors.mockResolvedValue(['vscode', 'cursor', 'zed', 'windsurf'])
    const panel = openPanel()
    await within(panel).findByRole('button', { name: '用 VS Code 打开' })
    fireEvent.click(within(panel).getByRole('button', { name: '选择打开方式' }))
    const item = screen.getByRole('menuitemradio', { name: label })
    expect(item.querySelector('.lucide-code-xml')).toBeNull()
    expect(item.querySelector('svg, img, [style*="mask"]')).not.toBeNull()
    fireEvent.click(item)
    await waitFor(() => expect(mocks.dockApi.openInEditor).toHaveBeenCalledWith('/repo', id))
    const primary = within(panel).getByRole('button', { name: `用 ${label} 打开` })
    expect(primary.querySelector('.lucide-code-xml')).toBeNull()
    expect(primary.querySelector('svg, img, [style*="mask"]')).not.toBeNull()
  })

  it('offers the detected editor and file manager even when only one editor is installed', async () => {
    const panel = openPanel()
    await within(panel).findByRole('button', { name: '用 VS Code 打开' })
    fireEvent.click(within(panel).getByRole('button', { name: '选择打开方式' }))
    const menu = screen.getByRole('menu')
    expect(within(menu).getByRole('menuitemradio', { name: 'VS Code' })).toHaveAttribute('aria-checked', 'true')
    fireEvent.click(within(menu).getByRole('menuitemradio', { name: '文件管理器' }))
    await waitFor(() => expect(mocks.rememberEditor).toHaveBeenCalledWith('file-manager'))
    expect(mocks.dockApi.fsOpenPath).toHaveBeenCalledWith('/repo', '', 'open')
    fireEvent.click(within(panel).getByRole('button', { name: '用 文件管理器 打开' }))
    await waitFor(() => expect(mocks.dockApi.fsOpenPath).toHaveBeenCalledTimes(2))
    expect(mocks.dockApi.openInEditor).not.toHaveBeenCalled()
  })

  it.each(['missing', 'error'])('falls back to the file manager when editor detection is %s', async (scenario) => {
    if (scenario === 'error') mocks.dockApi.availableEditors.mockRejectedValue(new Error('detection failed'))
    else mocks.dockApi.availableEditors.mockResolvedValue([])
    mocks.rememberedEditor = 'vscode'
    const panel = openPanel()
    fireEvent.click(await within(panel).findByRole('button', { name: '用 文件管理器 打开' }))
    await waitFor(() => expect(mocks.dockApi.fsOpenPath).toHaveBeenCalledWith('/repo', '', 'open'))
    expect(within(panel).queryByRole('button', { name: '用 VS Code 打开' })).not.toBeInTheDocument()
    expect(mocks.dockApi.openInEditor).not.toHaveBeenCalled()
  })

  it('restores a file manager preference when editors are available', async () => {
    mocks.rememberedEditor = 'file-manager'
    const panel = openPanel()
    fireEvent.click(within(panel).getByRole('button', { name: '选择打开方式' }))
    const menu = screen.getByRole('menu')
    await within(menu).findByRole('menuitemradio', { name: 'VS Code' })
    expect(within(menu).getByRole('menuitemradio', { name: '文件管理器' })).toHaveAttribute('aria-checked', 'true')
    expect(within(panel).getByRole('button', { name: '用 文件管理器 打开' })).toBeInTheDocument()
  })

  it('updates the primary button after switching editors and preserves it on launch failure', async () => {
    mocks.dockApi.availableEditors.mockResolvedValue(['vscode', 'cursor'])
    mocks.rememberedEditor = 'cursor'
    const panel = openPanel()
    await within(panel).findByRole('button', { name: '用 Cursor 打开' })
    fireEvent.click(within(panel).getByRole('button', { name: '选择打开方式' }))
    fireEvent.click(screen.getByRole('menuitemradio', { name: 'VS Code' }))
    await waitFor(() => expect(mocks.rememberEditor).toHaveBeenCalledWith('vscode'))
    expect(mocks.dockApi.openInEditor).toHaveBeenCalledWith('/repo', 'vscode')
    expect(within(panel).getByRole('button', { name: '用 VS Code 打开' })).toBeInTheDocument()

    mocks.dockApi.openInEditor.mockRejectedValue(new Error('Editor launch failed'))
    mocks.rememberEditor.mockClear()
    fireEvent.click(within(panel).getByRole('button', { name: '选择打开方式' }))
    fireEvent.click(screen.getByRole('menuitemradio', { name: 'Cursor' }))
    await waitFor(() => expect(getNotices().some((notice) => notice.message === 'Editor launch failed')).toBe(true))
    expect(mocks.rememberEditor).not.toHaveBeenCalled()
    expect(within(panel).getByRole('button', { name: '用 VS Code 打开' })).toBeInTheDocument()
  })

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
    mocks.tasks = [runningTask]
    const panel = openPanel()
    expect(within(panel).getByLabelText('1 个后台任务运行中')).toHaveTextContent('1')
  })

  it('shows task names and progress directly, folds finished tasks, and opens the task panel', () => {
    mocks.tasks = [runningTask, { ...runningTask, id: 'done', title: 'Old build', summary: 'Previous build completed', status: 'completed', elapsedSecs: 999 }]
    const onOpenTasks = vi.fn()
    const panel = openPanel({ onOpenTasks })
    expect(within(panel).getByText('后台任务')).toBeVisible()
    expect(within(panel).getByText('npm run build')).toBeVisible()
    expect(within(panel).getByText('Building the frontend')).toBeVisible()
    expect(within(panel).getByText('1m12s')).toBeVisible()
    expect(within(panel).getByText('Old build')).not.toBeVisible()
    fireEvent.click(within(panel).getByText(/已结束/))
    expect(within(panel).getByText('Old build')).toBeVisible()
    expect(within(panel).queryByText('16m39s')).not.toBeInTheDocument()
    fireEvent.click(within(panel).getByRole('button', { name: /npm run build.*Building the frontend/ }))
    expect(onOpenTasks).toHaveBeenCalledOnce()
  })

  it('disables duplicate stop clicks and leaves the task visible after a stop failure', async () => {
    mocks.tasks = [runningTask]
    let rejectStop: (error: Error) => void = () => { throw new Error('Stop request not started') }
    mocks.stopBackgroundTask.mockReturnValueOnce(new Promise<void>((_resolve, reject) => { rejectStop = reject }))
    const panel = openPanel()
    const stop = within(panel).getByRole('button', { name: '终止 npm run build' })
    fireEvent.click(stop)
    expect(stop).toBeDisabled()
    expect(mocks.stopBackgroundTask).toHaveBeenCalledWith('c1', runningTask)
    await act(async () => { rejectStop(new Error('Background stop failed')) })
    expect(stop).toBeEnabled()
    expect(within(panel).getByText(runningTask.title)).toBeVisible()
    expect(getNotices().some(notice => notice.message === 'Background stop failed')).toBe(true)
  })

  it('honors the task visibility preference', () => {
    mocks.tasks = [runningTask]
    mocks.itemOverrides = { tasks: false }
    const panel = openPanel()
    expect(within(panel).queryByText(runningTask.title)).not.toBeInTheDocument()
    expect(within(panel).queryByText('后台任务')).not.toBeInTheDocument()
  })

  it('opens by default for project chats, including a new one, and stays collapsed without a project', () => {
    const props = { workdir: '/repo', dockOpen: false, lang: 'zh' as const }
    const view = render(<WorkspaceActionsMenu {...props} projectName="Repo" conversationId={null} />)
    expect(screen.getByRole('complementary', { name: '工作区' })).toBeInTheDocument()
    view.rerender(<WorkspaceActionsMenu {...props} projectName={null} conversationId="loose" />)
    expect(screen.queryByRole('complementary', { name: '工作区' })).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: /^工作区/ }))
    expect(mocks.panelChoice.get('loose')).toBe(true)
    view.rerender(<WorkspaceActionsMenu {...props} projectName="Repo" conversationId="old" />)
    fireEvent.click(screen.getByRole('button', { name: /^工作区/ }))
    expect(mocks.panelChoice.get('old')).toBe(false)
  })

  it('omits folder rows for a chat without a working directory', () => {
    const panel = openPanel({ projectName: null, workdir: '' })
    expect(within(panel).queryByRole('button', { name: /打开/ })).not.toBeInTheDocument()
  })

  it('never restores an inline preference as an automatic popover on another conversation', () => {
    mocks.panelChoice.set('saved-open', true)
    const props = { workdir: '/repo', projectName: 'Repo', dockOpen: true, inlineAllowed: false, lang: 'zh' as const }
    const view = render(<WorkspaceActionsMenu {...props} conversationId="first" />)
    view.rerender(<WorkspaceActionsMenu {...props} conversationId="saved-open" />)
    expect(screen.queryByRole('dialog', { name: '工作区' })).not.toBeInTheDocument()
    expect(mocks.badgeWorkdirs.every((workdir) => !workdir)).toBe(true)
  })

  it('keeps inline and popover choices separate through window resizing', () => {
    mocks.panelChoice.set('c1', false)
    const resize = (width: number) => act(() => {
      vi.stubGlobal('innerWidth', width)
      fireEvent(window, new Event('resize'))
    })
    resize(1200)
    render(<WorkspaceActionsMenu workdir="/repo" projectName="Repo" conversationId="c1" dockOpen={false} lang="zh" />)
    resize(800)
    fireEvent.click(screen.getByRole('button', { name: /^工作区/ }))
    expect(screen.getByRole('dialog', { name: '工作区' })).toBeInTheDocument()
    resize(1200)
    expect(screen.queryByRole('complementary', { name: '工作区' })).not.toBeInTheDocument()
    expect(mocks.panelChoice.get('c1')).toBe(false)
    resize(800)
    expect(screen.queryByRole('dialog', { name: '工作区' })).not.toBeInTheDocument()
  })

  it('restores an explicitly opened no-project panel after narrowing and widening the window', () => {
    render(<WorkspaceActionsMenu workdir="/repo" projectName={null} conversationId="new" dockOpen={false} lang="zh" />)
    fireEvent.click(screen.getByRole('button', { name: /^工作区/ }))
    expect(mocks.panelChoice.get('new')).toBe(true)
    act(() => { vi.stubGlobal('innerWidth', 800); fireEvent(window, new Event('resize')) })
    expect(screen.queryByRole('dialog', { name: '工作区' })).not.toBeInTheDocument()
    act(() => { vi.stubGlobal('innerWidth', 1200); fireEvent(window, new Event('resize')) })
    expect(screen.getByRole('complementary', { name: '工作区' })).toBeInTheDocument()
  })

  it('closes a transient popover when switching conversations or workdirs', () => {
    const props = { workdir: '/repo', projectName: 'Repo', dockOpen: true, inlineAllowed: false, lang: 'zh' as const }
    const view = render(<WorkspaceActionsMenu {...props} conversationId="first" />)
    fireEvent.click(screen.getByRole('button', { name: /^工作区/ }))
    view.rerender(<WorkspaceActionsMenu {...props} conversationId="second" />)
    expect(screen.queryByRole('dialog', { name: '工作区' })).not.toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: /^工作区/ }))
    view.rerender(<WorkspaceActionsMenu {...props} workdir="/other" conversationId="second" />)
    expect(screen.queryByRole('dialog', { name: '工作区' })).not.toBeInTheDocument()
  })

  it('unmounts nested menus when leaving the conversation', async () => {
    const props = { workdir: '/repo', projectName: 'Repo', dockOpen: false, inlineAllowed: false, lang: 'zh' as const }
    const view = render(<WorkspaceActionsMenu {...props} conversationId="first" />)
    fireEvent.click(screen.getByRole('button', { name: /^工作区/ }))
    await screen.findByRole('button', { name: '用 VS Code 打开' })
    fireEvent.click(screen.getByRole('button', { name: '选择打开方式' }))
    expect(screen.getByRole('menu')).toBeInTheDocument()
    view.rerender(<WorkspaceActionsMenu {...props} conversationId="second" />)
    expect(screen.queryByRole('menu')).not.toBeInTheDocument()
    expect(screen.queryByRole('dialog', { name: '工作区' })).not.toBeInTheDocument()
  })

  it('keeps a manual collapse of a project chat across remounts', () => {
    const props = { workdir: '/repo', projectName: 'Repo', conversationId: 'new', dockOpen: false, lang: 'zh' as const }
    const view = render(<WorkspaceActionsMenu {...props} />)
    expect(screen.getByRole('complementary', { name: '工作区' })).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: /^工作区/ }))
    view.unmount()
    render(<WorkspaceActionsMenu {...props} />)
    expect(screen.queryByRole('complementary', { name: '工作区' })).not.toBeInTheDocument()
  })

  it('allows the inline panel beside the dock when the chat pane has enough room', () => {
    const pane = document.createElement('div')
    pane.className = 'chat-main-pane'
    document.body.append(pane)
    vi.spyOn(pane, 'getBoundingClientRect').mockReturnValue(new DOMRect(0, 52, 1200, 800))
    try {
      render(<WorkspaceActionsMenu workdir="/repo" projectName="Repo" conversationId="c1" dockOpen lang="zh" />)
      expect(screen.getByRole('complementary', { name: '工作区' })).toBeInTheDocument()
    } finally { pane.remove() }
  })

  it('does not automatically show the panel in a short window', () => {
    vi.stubGlobal('innerHeight', 200)
    render(<WorkspaceActionsMenu workdir="/repo" projectName="Repo" conversationId="c1" dockOpen={false} lang="zh" />)
    expect(screen.queryByRole('complementary', { name: '工作区' })).not.toBeInTheDocument()
  })

  describe('panel placement', () => {
    let pane: HTMLDivElement
    let paneRect: DOMRect
    let observers: Set<() => void>
    const layoutChanged = () => act(() => { for (const measure of [...observers]) measure() })

    beforeEach(() => {
      vi.stubGlobal('innerWidth', 1600)
      vi.stubGlobal('innerHeight', 900)
      pane = document.createElement('div')
      pane.className = 'chat-main-pane'
      document.body.append(pane)
      paneRect = new DOMRect(240, 44, 1360, 856)
      observers = new Set()
      vi.stubGlobal('ResizeObserver', class {
        constructor(private measure: () => void) {}
        observe() { observers.add(this.measure) }
        disconnect() { observers.delete(this.measure) }
      })
      vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
        return this === pane ? paneRect : new DOMRect(700, 8, 24, 28)
      })
    })

    afterEach(() => { pane.remove() })

    it('follows the chat edge when the dock opens, resizes and closes without a window resize', () => {
      const props = { workdir: '/repo', projectName: 'Repo', conversationId: 'c1', lang: 'zh' as const }
      const view = render(<WorkspaceActionsMenu {...props} dockOpen={false} />)
      const panel = screen.getByRole('complementary', { name: '工作区' })
      expect(panel.style.right).toBe('8px')
      paneRect = new DOMRect(240, 44, 1040, 856)
      view.rerender(<WorkspaceActionsMenu {...props} dockOpen />)
      layoutChanged()
      expect(panel.style.right).toBe('328px')
      paneRect = new DOMRect(240, 44, 960, 856)
      layoutChanged()
      expect(panel.style.right).toBe('408px')
      paneRect = new DOMRect(240, 44, 1360, 856)
      view.rerender(<WorkspaceActionsMenu {...props} dockOpen={false} />)
      layoutChanged()
      expect(panel.style.right).toBe('8px')
    })

    it('aligns with the settled chat edge while the pane is still sliding into place', () => {
      vi.stubGlobal('DOMMatrixReadOnly', class {
        m41: number
        m42: number
        constructor(transform: string) {
          const [x = 0, y = 0] = (transform.match(/-?\d+(\.\d+)?/g) ?? []).map(Number)
          this.m41 = x
          this.m42 = y
        }
      })
      const props = { workdir: '/repo', projectName: 'Repo', conversationId: 'c1', lang: 'zh' as const }
      render(<WorkspaceActionsMenu {...props} dockOpen={false} />)
      const panel = screen.getByRole('complementary', { name: '工作区' })
      // 侧栏收起的第一帧：主区已变宽到窗口右缘，却仍被平移动画向左拉回 240px。
      pane.style.transform = 'translate(-240px, 0px)'
      Object.assign(pane, { getAnimations: () => [{}] })
      paneRect = new DOMRect(-240, 44, 1600, 856)
      layoutChanged()
      expect(panel.style.right).toBe('8px')
    })

    it('aligns a narrow-window popover with the chat edge below the titlebar', () => {
      vi.stubGlobal('innerWidth', 900)
      paneRect = new DOMRect(220, 44, 680, 856)
      const panel = openPanel({ dockOpen: false })
      expect(panel.style.right).toBe('8px')
      expect(panel.style.top).toBe('52px')
    })

    it('releases the reserved chat width when opening the dock leaves too little room', () => {
      render(<WorkspaceActionsMenu workdir="/repo" projectName="Repo" conversationId="c1" dockOpen={false} lang="zh" />)
      expect(document.documentElement.style.getPropertyValue('--chat-details-inset')).toBe('316px')
      paneRect = new DOMRect(240, 44, 700, 856)
      layoutChanged()
      expect(screen.queryByRole('complementary', { name: '工作区' })).not.toBeInTheDocument()
      expect(document.documentElement.style.getPropertyValue('--chat-details-inset')).toBe('')
      fireEvent.click(screen.getByRole('button', { name: /^工作区/ }))
      expect(screen.getByRole('dialog', { name: '工作区' }).style.right).toBe('668px')
    })

    it('keeps the popover usable when the dock leaves a narrow chat column', () => {
      paneRect = new DOMRect(240, 44, 260, 856)
      const panel = openPanel()
      expect(panel.style.width).toBe('300px')
      expect(panel.style.right).toBe('1108px')
    })

    it('constrains the popover to the viewport in a very small window', () => {
      vi.stubGlobal('innerWidth', 280)
      paneRect = new DOMRect(0, 44, 280, 856)
      const panel = openPanel()
      expect(panel.style.width).toBe('264px')
      expect(panel.style.right).toBe('8px')
    })
  })

  it('hides the empty task section', () => {
    const panel = openPanel()
    expect(within(panel).queryByText('任务')).not.toBeInTheDocument()
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
