import { act, render, screen, waitFor } from '@testing-library/react'
import { StrictMode } from 'react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { getSettingsCached } from '../api/settingsCache'
import { chatApi } from './api'
import * as dialogs from '../components/dialogQueue'
import * as nativeDialog from '@tauri-apps/plugin-dialog'
import { Sidebar, type SidebarProps } from './Sidebar'
import type { ChatProject, Conversation, ConversationListItem } from './types'

vi.mock('../api/settingsCache', () => ({
  getSettingsCached: vi.fn().mockResolvedValue({ chat: {} }),
}))

vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }))

const project1: ChatProject = {
  id: 'project-1',
  name: '项目1',
  created_at: 1,
  updated_at: 1,
}

const project2: ChatProject = {
  id: 'project-2',
  name: '项目2',
  created_at: 2,
  updated_at: 2,
}

function conversation(id: string, title: string, project: ChatProject): ConversationListItem {
  return {
    id,
    title,
    preview: '',
    provider_id: 'provider',
    model: 'model',
    message_count: 1,
    created_at: 1,
    updated_at: 1,
    folder: project.name,
    project_id: project.id,
  }
}

afterEach(() => {
  vi.restoreAllMocks()
})

beforeEach(() => {
  vi.mocked(getSettingsCached).mockResolvedValue({ chat: {} } as Awaited<ReturnType<typeof getSettingsCached>>)
})

describe('Sidebar conversation navigation', () => {
  it('selects a conversation in another project without first opening that project new-chat view', async () => {
    const user = userEvent.setup()
    const target = conversation('conversation-2b', '项目2第二个对话', project2)
    vi.spyOn(chatApi, 'getProjects').mockResolvedValue([project1, project2])
    vi.spyOn(chatApi, 'getSets').mockResolvedValue([])
    vi.spyOn(chatApi, 'getAssistants').mockResolvedValue([])
    vi.spyOn(chatApi, 'getConversations').mockResolvedValue([
      conversation('conversation-1', '项目1对话', project1),
      conversation('conversation-2a', '项目2第一个对话', project2),
      target,
    ])
    vi.spyOn(chatApi, 'getConversationPins').mockResolvedValue({})

    const onSelectProject = vi.fn()
    const onSelectConversation = vi.fn()
    render(
      <Sidebar
        lang="zh"
        currentConversationId="conversation-1"
        selectedProject={project1}
        onSelectProject={onSelectProject}
        selectedSet={null}
        onSelectSet={vi.fn()}
        onSelectConversation={onSelectConversation}
        onNewConversation={vi.fn()}
        onOpenSettings={vi.fn()}
        onOpenExtensionsItem={vi.fn()}
        onSelectLang={vi.fn()}
        onOpenUsage={vi.fn()}
        collapsed={false}
        onToggleCollapsed={vi.fn()}
        refreshKey={0}
        searchOpen={false}
        onSearchOpenChange={vi.fn()}
      />,
    )

    await user.click(await screen.findByRole('button', { name: '项目', current: false }))
    await user.click(await screen.findByRole('button', { name: target.title }))

    expect(onSelectProject).not.toHaveBeenCalled()
    expect(onSelectConversation).toHaveBeenCalledOnce()
    expect(onSelectConversation).toHaveBeenCalledWith(
      target.id,
      target,
      { project: project2, set: null },
    )
    await waitFor(() => expect(chatApi.getConversations).toHaveBeenCalled())
  })

  it('opens the only conversation in another project on the first click', async () => {
    const user = userEvent.setup()
    const onlyConversation = conversation('conversation-only', '项目2唯一对话', project2)
    vi.spyOn(chatApi, 'getProjects').mockResolvedValue([project1, project2])
    vi.spyOn(chatApi, 'getSets').mockResolvedValue([])
    vi.spyOn(chatApi, 'getAssistants').mockResolvedValue([])
    vi.spyOn(chatApi, 'getConversations').mockResolvedValue([
      conversation('conversation-1', '项目1对话', project1),
      onlyConversation,
    ])
    vi.spyOn(chatApi, 'getConversationPins').mockResolvedValue({})

    const onSelectProject = vi.fn()
    const onSelectConversation = vi.fn()
    render(
      <Sidebar
        lang="zh"
        currentConversationId="conversation-1"
        selectedProject={project1}
        onSelectProject={onSelectProject}
        selectedSet={null}
        onSelectSet={vi.fn()}
        onSelectConversation={onSelectConversation}
        onNewConversation={vi.fn()}
        onOpenSettings={vi.fn()}
        onOpenExtensionsItem={vi.fn()}
        onSelectLang={vi.fn()}
        onOpenUsage={vi.fn()}
        collapsed={false}
        onToggleCollapsed={vi.fn()}
        refreshKey={0}
        searchOpen={false}
        onSearchOpenChange={vi.fn()}
      />,
    )

    await user.click(await screen.findByRole('button', { name: '项目', current: false }))
    await user.click(await screen.findByRole('button', { name: onlyConversation.title }))

    expect(onSelectProject).not.toHaveBeenCalled()
    expect(onSelectConversation).toHaveBeenCalledOnce()
    expect(onSelectConversation).toHaveBeenCalledWith(
      onlyConversation.id,
      onlyConversation,
      { project: project2, set: null },
    )
  })
})

describe('Sidebar pin while generating', () => {
  it('pins a generating conversation immediately even when the optimistic row is showing', async () => {
    const user = userEvent.setup()
    const running = conversation('conversation-run', 'desktop-cc-gui', project1)
    let persistPinned = false
    vi.spyOn(chatApi, 'getProjects').mockResolvedValue([project1])
    vi.spyOn(chatApi, 'getSets').mockResolvedValue([])
    vi.spyOn(chatApi, 'getAssistants').mockResolvedValue([])
    vi.spyOn(chatApi, 'getConversations').mockImplementation(async () => [
      { ...running, pinned: persistPinned },
    ])
    vi.spyOn(chatApi, 'getConversationPins').mockResolvedValue({})
    vi.spyOn(chatApi, 'updateConversation').mockImplementation(async (_id, updates) => {
      persistPinned = Boolean(updates.pinned)
      return { id: running.id } as Conversation
    })

    render(
      <Sidebar
        lang="zh"
        currentConversationId={running.id}
        generatingConversationIds={new Set([running.id])}
        optimisticConversations={[{ ...running, pinned: false }]}
        selectedProject={project1}
        onSelectProject={vi.fn()}
        selectedSet={null}
        onSelectSet={vi.fn()}
        onSelectConversation={vi.fn()}
        onNewConversation={vi.fn()}
        onOpenSettings={vi.fn()}
        onOpenExtensionsItem={vi.fn()}
        onSelectLang={vi.fn()}
        onOpenUsage={vi.fn()}
        collapsed={false}
        onToggleCollapsed={vi.fn()}
        refreshKey={0}
        searchOpen={false}
        onSearchOpenChange={vi.fn()}
      />,
    )

    await user.click(await screen.findByRole('button', { name: '置顶聊天', hidden: true }))

    expect(await screen.findByRole('button', { name: '取消置顶' })).toBeInTheDocument()
    expect(chatApi.updateConversation).toHaveBeenCalledWith(running.id, { pinned: true })
  })
})

describe('Sidebar open archived conversation', () => {
  it('keeps the open conversation visible when the list omitted it as archived', async () => {
    const open = conversation('conversation-open', '交给你一个任务，使用浏览器已经登录', project1)
    vi.spyOn(chatApi, 'getProjects').mockResolvedValue([project1])
    vi.spyOn(chatApi, 'getSets').mockResolvedValue([])
    vi.spyOn(chatApi, 'getAssistants').mockResolvedValue([])
    vi.spyOn(chatApi, 'getConversations').mockResolvedValue([
      conversation('conversation-other', '其他对话', project1),
    ])
    vi.spyOn(chatApi, 'getConversationPins').mockResolvedValue({})

    render(
      <Sidebar
        lang="zh"
        currentConversationId={open.id}
        openConversation={open}
        selectedProject={null}
        onSelectProject={vi.fn()}
        selectedSet={null}
        onSelectSet={vi.fn()}
        onSelectConversation={vi.fn()}
        onNewConversation={vi.fn()}
        onOpenSettings={vi.fn()}
        onOpenExtensionsItem={vi.fn()}
        onSelectLang={vi.fn()}
        onOpenUsage={vi.fn()}
        collapsed={false}
        onToggleCollapsed={vi.fn()}
        refreshKey={0}
        searchOpen={false}
        onSearchOpenChange={vi.fn()}
      />,
    )

    expect(await screen.findByRole('button', { name: /交给你一个任务/ })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /其他对话/ })).toBeInTheDocument()
  })
})

describe('Sidebar archive race', () => {
  it('does not flash an archived row back when a refresh races persist', async () => {
    const user = userEvent.setup()
    const leaving = conversation('conversation-archive', '要归档的对话', project1)
    const staying = conversation('conversation-keep', '留下的对话', project1)
    let persisted = false
    let releasePersist!: () => void
    const persistGate = new Promise<void>((resolve) => {
      releasePersist = resolve
    })

    vi.spyOn(chatApi, 'getProjects').mockResolvedValue([project1])
    vi.spyOn(chatApi, 'getSets').mockResolvedValue([])
    vi.spyOn(chatApi, 'getAssistants').mockResolvedValue([])
    vi.spyOn(chatApi, 'getConversations').mockImplementation(async () => (
      persisted ? [staying] : [leaving, staying]
    ))
    vi.spyOn(chatApi, 'getConversationPins').mockResolvedValue({})
    vi.spyOn(chatApi, 'updateConversation').mockImplementation(async () => {
      await persistGate
      persisted = true
      return { id: leaving.id } as Conversation
    })

    const onConversationDeleted = vi.fn()
    const { rerender } = render(
      <Sidebar
        lang="zh"
        currentConversationId={leaving.id}
        selectedProject={project1}
        onSelectProject={vi.fn()}
        selectedSet={null}
        onSelectSet={vi.fn()}
        onSelectConversation={vi.fn()}
        onNewConversation={vi.fn()}
        onConversationDeleted={onConversationDeleted}
        onOpenSettings={vi.fn()}
        onOpenExtensionsItem={vi.fn()}
        onSelectLang={vi.fn()}
        onOpenUsage={vi.fn()}
        collapsed={false}
        onToggleCollapsed={vi.fn()}
        refreshKey={0}
        searchOpen={false}
        onSearchOpenChange={vi.fn()}
      />,
    )

    await screen.findByRole('button', { name: /要归档的对话/ })
    await user.click(screen.getAllByRole('button', { name: '归档' })[0])
    expect(onConversationDeleted).toHaveBeenCalledWith(leaving.id)
    expect(screen.queryByRole('button', { name: /要归档的对话/ })).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: /留下的对话/ })).toBeInTheDocument()

    rerender(
      <Sidebar
        lang="zh"
        currentConversationId={undefined}
        selectedProject={project1}
        onSelectProject={vi.fn()}
        selectedSet={null}
        onSelectSet={vi.fn()}
        onSelectConversation={vi.fn()}
        onNewConversation={vi.fn()}
        onConversationDeleted={onConversationDeleted}
        onOpenSettings={vi.fn()}
        onOpenExtensionsItem={vi.fn()}
        onSelectLang={vi.fn()}
        onOpenUsage={vi.fn()}
        collapsed={false}
        onToggleCollapsed={vi.fn()}
        refreshKey={1}
        searchOpen={false}
        onSearchOpenChange={vi.fn()}
      />,
    )

    await waitFor(() => expect(vi.mocked(chatApi.getConversations).mock.calls.length).toBeGreaterThan(1))
    expect(screen.queryByRole('button', { name: /要归档的对话/ })).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: /留下的对话/ })).toBeInTheDocument()

    releasePersist()
    await waitFor(() => expect(persisted).toBe(true))
    expect(screen.queryByRole('button', { name: /要归档的对话/ })).not.toBeInTheDocument()
  })
})

describe('Sidebar resize handle', () => {
  function renderSidebar(onWidthChange: (width: number) => void, collapsed = false) {
    return render(
      <Sidebar
        lang="zh"
        selectedProject={null}
        onSelectProject={vi.fn()}
        selectedSet={null}
        onSelectSet={vi.fn()}
        onSelectConversation={vi.fn()}
        onNewConversation={vi.fn()}
        onOpenSettings={vi.fn()}
        onOpenExtensionsItem={vi.fn()}
        onSelectLang={vi.fn()}
        onOpenUsage={vi.fn()}
        collapsed={collapsed}
        onToggleCollapsed={vi.fn()}
        width={240}
        onWidthChange={onWidthChange}
        refreshKey={0}
        searchOpen={false}
        onSearchOpenChange={vi.fn()}
      />,
    )
  }

  beforeEach(() => {
    vi.spyOn(chatApi, 'getProjects').mockResolvedValue([])
    vi.spyOn(chatApi, 'getSets').mockResolvedValue([])
    vi.spyOn(chatApi, 'getAssistants').mockResolvedValue([])
    vi.spyOn(chatApi, 'getConversations').mockResolvedValue([])
    vi.spyOn(chatApi, 'getConversationPins').mockResolvedValue({})
  })

  it('shows a drag handle when expanded and hides it when collapsed', async () => {
    const { container, rerender } = renderSidebar(vi.fn())
    await waitFor(() => expect(chatApi.getConversations).toHaveBeenCalled())
    expect(container.querySelector('.chat-sidebar-resize')).toBeTruthy()

    rerender(
      <Sidebar
        lang="zh"
        selectedProject={null}
        onSelectProject={vi.fn()}
        selectedSet={null}
        onSelectSet={vi.fn()}
        onSelectConversation={vi.fn()}
        onNewConversation={vi.fn()}
        onOpenSettings={vi.fn()}
        onOpenExtensionsItem={vi.fn()}
        onSelectLang={vi.fn()}
        onOpenUsage={vi.fn()}
        collapsed
        onToggleCollapsed={vi.fn()}
        width={240}
        refreshKey={0}
        searchOpen={false}
        onSearchOpenChange={vi.fn()}
      />,
    )
    expect(container.querySelector('.chat-sidebar-resize')).toBeNull()
  })
})


describe('Sidebar refresh lifecycle', () => {
  function setup() {
    const reads = vi.spyOn(chatApi, 'getConversations').mockResolvedValue([])
    vi.spyOn(chatApi, 'getProjects').mockResolvedValue([project1, project2])
    vi.spyOn(chatApi, 'getSets').mockResolvedValue([])
    vi.spyOn(chatApi, 'getAssistants').mockResolvedValue([])
    vi.spyOn(chatApi, 'getConversationPins').mockResolvedValue({})
    const noop = () => {}
    const props: SidebarProps = {
      lang: 'zh', currentConversationId: undefined, selectedProject: null, selectedSet: null,
      onSelectProject: noop, onSelectSet: noop, onSelectConversation: noop,
      onNewConversation: noop, onOpenSettings: noop, onOpenExtensionsItem: noop,
      onSelectLang: noop, onOpenUsage: noop, collapsed: false, onToggleCollapsed: noop,
      refreshKey: 0, searchOpen: false, onSearchOpenChange: noop,
    }
    return { reads, props }
  }

  it('does not reload the catalog when navigation callbacks or the selected group change', async () => {
    const { reads, props } = setup()
    const view = render(<Sidebar {...props} />)
    await act(async () => {})
    view.rerender(<Sidebar {...props} selectedProject={project1} onSelectProject={() => {}} onSelectSet={() => {}} />)
    await act(async () => {})
    expect(reads).toHaveBeenCalledTimes(1)
    view.rerender(<Sidebar {...props} refreshKey={1} />)
    await act(async () => {})
    expect(reads).toHaveBeenCalledTimes(2)
  })

  it('finishes initial loading after a StrictMode effect restart', async () => {
    const { reads, props } = setup()
    const latest = { ...conversation('latest', 'Latest conversation', project1), project_id: undefined, folder: undefined }
    reads.mockResolvedValue([latest])
    render(<StrictMode><Sidebar {...props} /></StrictMode>)
    expect(await screen.findByRole('button', { name: latest.title })).toBeInTheDocument()
    expect(screen.queryByLabelText('加载中')).not.toBeInTheDocument()
  })

  it('coalesces refreshes during a slow read and publishes the final catalog', async () => {
    const { reads, props } = setup()
    let resolve!: (items: ConversationListItem[]) => void
    reads.mockImplementationOnce(() => new Promise(done => { resolve = done }))
    const latest = { ...conversation('latest', 'Latest conversation', project1), project_id: undefined, folder: undefined }
    reads.mockResolvedValue([latest])
    const onConversationsLoaded = vi.fn()
    props.onConversationsLoaded = onConversationsLoaded
    const view = render(<Sidebar {...props} />)
    for (let refreshKey = 1; refreshKey <= 10; refreshKey++) {
      view.rerender(<Sidebar {...props} refreshKey={refreshKey} />)
    }
    expect(reads).toHaveBeenCalledTimes(1)
    await act(async () => { resolve([]) })
    expect(reads).toHaveBeenCalledTimes(2)
    expect(await screen.findByRole('button', { name: latest.title })).toBeInTheDocument()
    // Only the fresh result may retire optimistic rows created during the read.
    expect(onConversationsLoaded).toHaveBeenCalledTimes(1)
  })

  it('does not navigate from a response after the sidebar has unmounted', async () => {
    const { props } = setup()
    let resolve!: (items: ChatProject[]) => void
    vi.mocked(chatApi.getProjects).mockImplementationOnce(() => new Promise(done => { resolve = done }))
    const onSelectProject = vi.fn()
    const view = render(<Sidebar {...props} selectedProject={project1} onSelectProject={onSelectProject} />)
    await act(async () => {})
    view.unmount()
    await act(async () => { resolve([]) })
    expect(onSelectProject).not.toHaveBeenCalled()
  })

  it('waits for the remaining reads after one fails, then recovers on a queued refresh', async () => {
    const { reads, props } = setup()
    vi.spyOn(console, 'error').mockImplementation(() => {})
    vi.mocked(chatApi.getProjects).mockRejectedValueOnce(new Error('temporary failure'))
    let finish!: () => void
    vi.mocked(chatApi.getSets).mockImplementationOnce(() => new Promise(resolve => { finish = () => resolve([]) }))
    const view = render(<Sidebar {...props} />)
    await act(async () => {})
    view.rerender(<Sidebar {...props} refreshKey={1} />)
    await act(async () => {})
    expect(reads).toHaveBeenCalledTimes(1)
    const latest = { ...conversation('recovered', 'Recovered conversation', project1), project_id: undefined, folder: undefined }
    reads.mockResolvedValue([latest])
    await act(async () => finish())
    expect(reads).toHaveBeenCalledTimes(2)
    expect(await screen.findByRole('button', { name: latest.title })).toBeInTheDocument()
  })
})


describe('Sidebar view choice', () => {
  beforeEach(() => window.localStorage.removeItem('kivio-chat-sidebar-view'))
  afterEach(() => window.localStorage.removeItem('kivio-chat-sidebar-view'))

  function setup() {
    const set = { id: 'writing', name: '写作', created_at: 1, updated_at: 1 }
    const projectChat = conversation('project-chat', '修改导航', project1)
    const setChat = { ...conversation('set-chat', '润色文章', project2), project_id: undefined, folder: undefined, set_id: set.id }
    const looseChat = { ...conversation('loose-chat', '随便聊聊', project2), project_id: undefined, folder: undefined }
    vi.spyOn(chatApi, 'getProjects').mockResolvedValue([project1, project2])
    vi.spyOn(chatApi, 'getSets').mockResolvedValue([set])
    vi.spyOn(chatApi, 'getAssistants').mockResolvedValue([])
    vi.spyOn(chatApi, 'getConversationPins').mockResolvedValue({})
    vi.spyOn(chatApi, 'getConversations').mockResolvedValue([projectChat, setChat, looseChat])
    const props: SidebarProps = {
      lang: 'zh', selectedProject: project2, selectedSet: null,
      onSelectProject: vi.fn(), onSelectSet: vi.fn(), onSelectConversation: vi.fn(),
      onNewConversation: vi.fn(), onOpenSettings: vi.fn(), onOpenExtensionsItem: vi.fn(),
      onSelectLang: vi.fn(), onOpenUsage: vi.fn(), collapsed: false, onToggleCollapsed: vi.fn(),
      refreshKey: 0, searchOpen: false, onSearchOpenChange: vi.fn(),
    }
    return { props, projectChat, setChat, looseChat, set }
  }

  async function toggleView(user: ReturnType<typeof userEvent.setup>, name: string) {
    await user.click(await screen.findByRole('button', { name: '对话列表操作' }))
    await user.click(await screen.findByRole('menuitem', { name }))
    await waitFor(() => expect(screen.queryByRole('menu')).not.toBeInTheDocument())
  }

  it('keeps both views available and remembers the choice across remounts', async () => {
    const user = userEvent.setup()
    const { props, projectChat } = setup()
    const view = render(<Sidebar {...props} />)
    expect(await screen.findByRole('button', { name: '最近', current: true })).toBeInTheDocument()
    await toggleView(user, '切换为扁平视图')
    expect(screen.queryByRole('button', { name: '最近' })).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: '筛选对话归属' })).toHaveTextContent('全部对话')
    expect(screen.queryByRole('button', { name: '搜索对话' })).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: projectChat.title })).toHaveTextContent(project1.name)
    expect(props.onSelectConversation).not.toHaveBeenCalled()
    view.unmount()
    const restored = render(<Sidebar {...props} />)
    expect(await screen.findByRole('button', { name: '筛选对话归属' })).toBeInTheDocument()
    await toggleView(user, '切换为经典视图')
    await user.click(screen.getByRole('button', { name: '项目', current: false }))
    expect(await screen.findByRole('button', { name: projectChat.title })).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: '集', current: false }))
    expect(await screen.findByText('写作')).toBeInTheDocument()
    restored.unmount()
    render(<Sidebar {...props} />)
    expect(await screen.findByRole('button', { name: '最近', current: true })).toBeInTheDocument()
  })

  it('opens the selected project or set and creates in that group', async () => {
    const user = userEvent.setup()
    const { props, projectChat, setChat, looseChat, set } = setup()
    render(<Sidebar {...props} />)
    await toggleView(user, '切换为扁平视图')
    for (const [label, visible, hidden] of [
      ['项目 · 项目1', projectChat, setChat],
      ['集 · 写作', setChat, looseChat],
      ['未分组', looseChat, projectChat],
    ] as const) {
      await user.click(screen.getByRole('button', { name: '筛选对话归属' }))
      await user.click(screen.getByRole('option', { name: label }))
      expect(screen.getByRole('button', { name: '筛选对话归属' })).toHaveTextContent(label.split(' · ').pop()!)
      expect(screen.getByRole('button', { name: visible.title })).toBeInTheDocument()
      expect(screen.queryByRole('button', { name: hidden.title })).not.toBeInTheDocument()
    }
    expect(props.onSelectProject).toHaveBeenCalledWith(project1)
    expect(props.onSelectSet).toHaveBeenCalledWith(set)
    expect(props.onSelectConversation).not.toHaveBeenCalled()
    await user.click(screen.getByRole('button', { name: looseChat.title }))
    expect(props.onSelectConversation).toHaveBeenCalledWith(looseChat.id, looseChat, { project: null, set: null })
    await user.click(screen.getByRole('button', { name: '筛选对话归属' }))
    await user.click(screen.getByRole('option', { name: '集 · 写作' }))
    await user.click(screen.getByRole('button', { name: '在当前归属中新建聊天' }))
    expect(props.onSelectSet).toHaveBeenCalledWith(set)
    expect(props.onNewConversation).not.toHaveBeenCalled()
    await user.click(screen.getByRole('button', { name: '筛选对话归属' }))
    await user.click(screen.getByRole('option', { name: '项目 · 项目2' }))
    expect(screen.getByRole('status')).toHaveTextContent('当前列表中没有此归属的对话')
    await user.click(screen.getByRole('button', { name: '在当前归属中新建聊天' }))
    expect(props.onSelectProject).toHaveBeenCalledWith(project2)
  })

  it('clears only the filtered list after confirmation, even with another project open', async () => {
    const user = userEvent.setup()
    const { props, setChat } = setup()
    const confirm = vi.spyOn(dialogs, 'confirmDialog').mockResolvedValue(false)
    const remove = vi.spyOn(chatApi, 'deleteConversation').mockResolvedValue([])
    render(<Sidebar {...props} />)
    await toggleView(user, '切换为扁平视图')
    await user.click(screen.getByRole('button', { name: '筛选对话归属' }))
    await user.click(screen.getByRole('option', { name: '集 · 写作' }))
    for (const accepted of [false, true]) {
      confirm.mockResolvedValue(accepted)
      await user.click(screen.getByRole('button', { name: '对话列表操作' }))
      await user.click(screen.getByRole('menuitem', { name: '清空当前列表' }))
      await waitFor(() => expect(screen.queryByRole('menu')).not.toBeInTheDocument())
      if (!accepted) expect(remove).not.toHaveBeenCalled()
    }
    expect(confirm).toHaveBeenLastCalledWith(expect.objectContaining({
      message: '确定删除当前列表中的 1 条对话？此操作无法撤销。',
    }))
    expect(remove).toHaveBeenCalledTimes(1)
    expect(remove).toHaveBeenCalledWith(setChat.id)
  })

  it('adds a project in flat view, reports save failure, and creates the next chat in the new project', async () => {
    const user = userEvent.setup()
    const { props } = setup()
    const created = { ...project1, id: 'new-project', name: '新工程', root_path: '/tmp/new-project' }
    vi.mocked(nativeDialog.open).mockResolvedValue('/tmp/new-project')
    const create = vi.spyOn(chatApi, 'createProject').mockRejectedValueOnce(new Error('保存失败'))
      .mockImplementationOnce(async () => {
        vi.mocked(chatApi.getProjects).mockResolvedValue([created, project1, project2])
        return created
      })
    render(<Sidebar {...props} />)
    await toggleView(user, '切换为扁平视图')
    await user.click(screen.getByRole('button', { name: '新建项目' }))
    await user.type(screen.getByPlaceholderText('例如：产品发布计划'), created.name)
    await user.click(screen.getByRole('button', { name: '选择文件夹' }))
    expect(await screen.findByText('/tmp/new-project')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: '创建' }))
    expect(await screen.findByText('保存失败')).toBeInTheDocument()
    expect(props.onSelectProject).not.toHaveBeenCalled()
    await user.click(screen.getByRole('button', { name: '创建' }))
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
    expect(create).toHaveBeenCalledTimes(2)
    expect(create).toHaveBeenLastCalledWith(created.name, null, null, '/tmp/new-project')
    expect(props.onSelectProject).toHaveBeenCalledWith(created)
    expect(screen.getByRole('button', { name: '筛选对话归属' })).toHaveAttribute('title', '项目 · 新工程')
    vi.mocked(props.onSelectProject).mockClear()
    await user.click(screen.getByRole('button', { name: '在当前归属中新建聊天' }))
    expect(props.onSelectProject).toHaveBeenCalledWith(created)
    expect(props.onNewConversation).not.toHaveBeenCalled()
  })

})
