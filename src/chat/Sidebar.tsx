import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import {
  ChevronRight,
  Folder,
  FolderPlus,
  LayoutGrid,
  MoreHorizontal,
  Search,
  Settings as SettingsIcon,
  SquarePen,
} from 'lucide-react'
import type { ChatProject, ConversationListItem } from './types'
import { ConversationList } from './ConversationList'
import { ChatSectionMenu } from './ChatSectionMenu'
import { ProjectContextMenu } from './ProjectContextMenu'
import { ProjectDialog } from './ProjectDialog'
import { api } from '../api/tauri'
import { chatApi } from './api'
import { ChatTitlebarActions } from './ChatTitlebarActions'
import { chatTitlebarMacInsetClass, chatTitlebarRowClass, isMac } from './platform'
import type { ConversationMenuAnchor } from './ConversationContextMenu'
import { hasChatDisplayName, resolveChatUserProfile, type ChatUserProfile } from './userProfile'
import { UserAvatar } from './UserAvatar'

const modLabel = isMac ? '⌘' : 'Ctrl'

export type ExtensionsNavItem = 'assistants' | 'skill' | 'mcp'

const extensionSubItems: Array<{ id: ExtensionsNavItem; label: string }> = [
  { id: 'assistants', label: '助手' },
  { id: 'skill', label: '技能' },
  { id: 'mcp', label: '连接器' },
]

const PROJECT_PREVIEW_LIMIT = 5

function conversationProjectId(conversation: ConversationListItem): string | null {
  return conversation.project_id ?? conversation.projectId ?? null
}

function conversationBelongsToProject(
  conversation: ConversationListItem,
  project: ChatProject,
): boolean {
  const projectId = conversationProjectId(conversation)
  return projectId ? projectId === project.id : conversation.folder === project.name
}

function conversationMatchesSearch(conversation: ConversationListItem, query: string): boolean {
  if (!query) return true
  return (
    conversation.title.toLowerCase().includes(query) ||
    conversation.preview.toLowerCase().includes(query)
  )
}

function projectMatchesSearch(project: ChatProject, query: string): boolean {
  if (!query) return true
  return (
    project.name.toLowerCase().includes(query) ||
    (project.root_path ?? project.rootPath ?? '').toLowerCase().includes(query)
  )
}

function findConversationProject(
  conversation: ConversationListItem,
  projects: ChatProject[],
): ChatProject | undefined {
  const projectId = conversationProjectId(conversation)
  if (projectId) return projects.find((project) => project.id === projectId)
  return projects.find((project) => conversation.folder === project.name)
}

function conversationProjectLabel(
  conversation: ConversationListItem,
  projects: ChatProject[],
): string {
  return findConversationProject(conversation, projects)?.name ?? conversation.folder ?? ''
}

interface SidebarProps {
  currentConversationId?: string
  generatingConversationIds?: ReadonlySet<string>
  optimisticConversations?: ConversationListItem[]
  selectedProject?: ChatProject | null
  onSelectProject: (project: ChatProject | null) => void
  onSelectConversation: (id: string) => void
  onNewConversation: () => void
  onConversationDeleted?: (id: string) => void
  onForceDropConversation?: (id: string) => void
  onOpenSettings: () => void
  onOpenExtensionsItem: (item: ExtensionsNavItem) => void
  settingsActive?: boolean
  extensionsActive?: ExtensionsNavItem | null
  collapsed: boolean
  onToggleCollapsed: () => void
  refreshKey: number
  profileRefreshKey?: number
  searchOpen: boolean
  onSearchOpenChange: (open: boolean) => void
}

function SidebarUserFooter({
  profile,
  settingsActive,
  onOpenSettings,
}: {
  profile: ChatUserProfile
  settingsActive: boolean
  onOpenSettings: () => void
}) {
  return (
    <div
      className="shrink-0 border-t border-neutral-200/60 px-2 pb-2.5 pt-2 dark:border-neutral-800/80"
      data-tauri-drag-region="false"
    >
      <div className="flex items-center gap-2 px-2 py-1.5">
        <div className="flex min-w-0 flex-1 items-center gap-2">
          <UserAvatar profile={profile} size={28} />
          {hasChatDisplayName(profile) && (
            <span
              className="min-w-0 truncate text-[13px] text-neutral-700 dark:text-neutral-300"
              title={profile.displayName}
            >
              {profile.displayName}
            </span>
          )}
        </div>
        <button
          type="button"
          onClick={onOpenSettings}
          className={`shrink-0 rounded-md p-1.5 transition-colors ${
            settingsActive
              ? 'bg-black/[0.06] text-neutral-800 dark:bg-white/[0.1] dark:text-neutral-100'
              : 'text-neutral-400 hover:bg-black/[0.05] hover:text-neutral-600 dark:text-neutral-500 dark:hover:bg-white/[0.08] dark:hover:text-neutral-300'
          }`}
          title="设置"
          aria-label="设置"
          aria-pressed={settingsActive}
        >
          <SettingsIcon size={16} strokeWidth={1.75} />
        </button>
      </div>
    </div>
  )
}

interface NavRowProps {
  icon: React.ReactNode
  label: string
  onClick?: () => void
  disabled?: boolean
  active?: boolean
  /** 图标在 hover 时的微动效（group-hover transform 工具类） */
  iconMotion?: string
}

function NavRow({ icon, label, onClick, disabled, active, iconMotion }: NavRowProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={`group flex w-full items-center gap-2.5 rounded-lg px-3 py-1.5 text-left text-[13px] transition-colors disabled:cursor-default disabled:opacity-40 ${
        active
          ? 'bg-black/[0.06] font-medium text-neutral-900 dark:bg-white/[0.1] dark:text-neutral-50'
          : 'text-neutral-800 hover:bg-black/[0.04] dark:text-neutral-200 dark:hover:bg-white/[0.06]'
      }`}
    >
      <span
        className={`flex h-5 w-5 shrink-0 items-center justify-center text-neutral-600 transition duration-300 ease-out will-change-transform group-hover:text-neutral-800 group-active:scale-90 dark:text-neutral-400 dark:group-hover:text-neutral-200 ${iconMotion ?? ''}`}
      >
        {icon}
      </span>
      <span className="min-w-0 flex-1 truncate font-medium">{label}</span>
    </button>
  )
}

function ExtensionsNav({
  activeItem,
  onSelectItem,
}: {
  activeItem?: ExtensionsNavItem | null
  onSelectItem: (item: ExtensionsNavItem) => void
}) {
  const [expanded, setExpanded] = useState(() => Boolean(activeItem))

  useEffect(() => {
    if (activeItem) setExpanded(true)
  }, [activeItem])

  const highlighted = expanded || !!activeItem

  return (
    <div className="py-0.5">
      <button
        type="button"
        onClick={() => setExpanded((open) => !open)}
        className={`group flex w-full items-center gap-2.5 rounded-lg px-3 py-1.5 text-left text-[13px] font-medium transition-colors ${
          highlighted
            ? 'bg-black/[0.06] text-neutral-900 dark:bg-white/[0.1] dark:text-neutral-50'
            : 'text-neutral-800 hover:bg-black/[0.04] dark:text-neutral-200 dark:hover:bg-white/[0.06]'
        }`}
        aria-expanded={expanded}
      >
        <span className="flex h-5 w-5 shrink-0 items-center justify-center text-neutral-600 transition duration-300 ease-out will-change-transform group-hover:text-neutral-800 group-active:scale-90 group-hover:rotate-3 group-hover:scale-110 dark:text-neutral-400 dark:group-hover:text-neutral-200">
          <LayoutGrid size={17} strokeWidth={1.75} />
        </span>
        <span className="min-w-0 flex-1 truncate">插件</span>
        <ChevronRight
          size={14}
          strokeWidth={2}
          className={`shrink-0 text-neutral-400 transition-transform duration-200 dark:text-neutral-500 ${
            expanded ? 'rotate-90' : ''
          }`}
        />
      </button>
      {expanded && (
        <div className="relative ml-[34px] mt-0.5 border-l border-neutral-200/90 pl-2 dark:border-neutral-700">
          {extensionSubItems.map((item) => {
            const active = activeItem === item.id
            return (
              <button
                key={item.id}
                type="button"
                onClick={() => onSelectItem(item.id)}
                className={`flex w-full rounded-md py-1.5 pl-3 pr-2 text-left text-[13px] transition-colors ${
                  active
                    ? 'font-medium text-neutral-900 dark:text-neutral-100'
                    : 'text-neutral-700 hover:bg-black/[0.04] hover:text-neutral-900 dark:text-neutral-300 dark:hover:bg-white/[0.06] dark:hover:text-neutral-100'
                }`}
              >
                {item.label}
              </button>
            )
          })}
        </div>
      )}
    </div>
  )
}

function SearchDialog({
  query,
  results,
  currentConversationId,
  projects,
  onQueryChange,
  onSelectConversation,
  onClose,
}: {
  query: string
  results: ConversationListItem[]
  currentConversationId?: string
  projects: ChatProject[]
  onQueryChange: (query: string) => void
  onSelectConversation: (conversation: ConversationListItem) => void
  onClose: () => void
}) {
  const dialogRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    inputRef.current?.focus()
  }, [])

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [onClose])

  const normalizedQuery = query.trim()

  return createPortal(
    <div
      className="fixed inset-0 z-[260] flex items-start justify-center bg-black/45 px-5 pt-[16vh] dark:bg-black/60"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose()
      }}
    >
      <div
        ref={dialogRef}
        className="chat-motion-popover flex max-h-[62vh] w-full max-w-[560px] flex-col overflow-hidden rounded-xl border border-neutral-200 bg-white shadow-2xl shadow-black/25 dark:border-neutral-700 dark:bg-[#242426]"
        role="dialog"
        aria-modal="true"
        aria-label="搜索对话"
      >
        <div className="flex items-center gap-2 border-b border-neutral-200/80 px-3 py-2 dark:border-neutral-700/80">
          <Search size={15} strokeWidth={1.75} className="shrink-0 text-neutral-400" />
          <input
            ref={inputRef}
            type="text"
            value={query}
            onChange={(event) => onQueryChange(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter' && results[0]) {
                if (event.nativeEvent.isComposing || event.keyCode === 229) return
                event.preventDefault()
                onSelectConversation(results[0])
              }
            }}
            placeholder="搜索对话"
            className="min-w-0 flex-1 bg-transparent text-[14px] font-medium text-neutral-900 outline-none placeholder:text-neutral-400 dark:text-neutral-100 dark:placeholder:text-neutral-500"
          />
        </div>

        <div className="px-3 pb-1 pt-2 text-[11px] font-semibold uppercase tracking-wide text-neutral-400 dark:text-neutral-500">
          {normalizedQuery ? '搜索结果' : '近期对话'}
        </div>

        <div className="custom-scrollbar min-h-0 overflow-y-auto px-1.5 pb-1.5">
          {results.length > 0 ? (
            results.map((conversation) => {
              const active = conversation.id === currentConversationId
              const projectLabel = conversationProjectLabel(conversation, projects)
              return (
                <button
                  key={conversation.id}
                  type="button"
                  onClick={() => onSelectConversation(conversation)}
                  className={`group/search-result flex w-full min-w-0 items-center gap-2 rounded-md px-2.5 py-1.5 text-left transition-colors ${
                    active
                      ? 'bg-black/[0.07] dark:bg-white/[0.1]'
                      : 'hover:bg-black/[0.04] dark:hover:bg-white/[0.07]'
                  }`}
                >
                  <span
                    className={`min-w-0 flex-1 truncate text-[13px] ${
                      active
                        ? 'font-semibold text-neutral-950 dark:text-neutral-50'
                        : 'font-medium text-neutral-800 dark:text-neutral-200'
                    }`}
                    title={conversation.title}
                  >
                    {conversation.title}
                  </span>
                  {projectLabel && (
                    <span className="max-w-[100px] shrink-0 truncate text-[12px] text-neutral-400 dark:text-neutral-500">
                      {projectLabel}
                    </span>
                  )}
                </button>
              )
            })
          ) : (
            <div className="px-3 py-6 text-center text-[13px] text-neutral-400 dark:text-neutral-500">
              没有匹配的对话
            </div>
          )}
        </div>
      </div>
    </div>,
    document.body,
  )
}

export const Sidebar = memo(function Sidebar({
  currentConversationId,
  generatingConversationIds = new Set(),
  optimisticConversations = [],
  selectedProject = null,
  onSelectProject,
  onSelectConversation,
  onNewConversation,
  onConversationDeleted,
  onForceDropConversation,
  onOpenSettings,
  onOpenExtensionsItem,
  settingsActive = false,
  extensionsActive = null,
  collapsed,
  onToggleCollapsed,
  refreshKey,
  profileRefreshKey = 0,
  searchOpen,
  onSearchOpenChange,
}: SidebarProps) {
  const asideRef = useRef<HTMLElement>(null)
  // 折叠后侧栏仍挂载（用于滑出动画），用 inert 让其退出 tab 序 / 不可点击 / 不进 a11y 树。
  // useLayoutEffect：在绘制前与 JSX 里的 aria-hidden 原子地一起生效，避免短暂可聚焦窗口。
  useLayoutEffect(() => {
    const el = asideRef.current
    if (el) el.inert = collapsed
  }, [collapsed])
  const [conversations, setConversations] = useState<ConversationListItem[]>([])
  const [projects, setProjects] = useState<ChatProject[]>([])
  const [searchQuery, setSearchQuery] = useState('')
  const [projectSectionCollapsed, setProjectSectionCollapsed] = useState(false)
  const [collapsedProjectIds, setCollapsedProjectIds] = useState<Set<string>>(
    () => new Set(),
  )
  const [expandedProjectConversationIds, setExpandedProjectConversationIds] = useState<Set<string>>(
    () => new Set(),
  )
  const [conversationSectionCollapsed, setConversationSectionCollapsed] = useState(false)
  const [loading, setLoading] = useState(false)
  const [sectionMenuAnchor, setSectionMenuAnchor] = useState<ConversationMenuAnchor | null>(null)
  const [projectMenuState, setProjectMenuState] = useState<{
    projectId: string
    anchor: ConversationMenuAnchor
  } | null>(null)
  const [dialogProject, setDialogProject] = useState<ChatProject | null | undefined>(undefined)
  const [projectSaving, setProjectSaving] = useState(false)
  const [projectError, setProjectError] = useState('')
  const sectionMenuButtonRef = useRef<HTMLButtonElement>(null)
  const sidebarLoadedRef = useRef(false)
  const lastProjectIdRef = useRef(selectedProject?.id)
  const [userProfile, setUserProfile] = useState(() => resolveChatUserProfile())

  useEffect(() => {
    let cancelled = false
    void api.getSettings().then((settings) => {
      if (!cancelled) setUserProfile(resolveChatUserProfile(settings.chat))
    }).catch((err) => {
      console.error('Failed to load chat user profile:', err)
    })
    return () => {
      cancelled = true
    }
  }, [profileRefreshKey])

  const loadSidebarData = useCallback(async (options?: { silent?: boolean; projectOverride?: ChatProject | null }) => {
    const projectForLoad = options?.projectOverride === undefined ? selectedProject : options.projectOverride
    const silent = options?.silent ?? false
    if (!silent) setLoading(true)
    try {
      const [projectData, conversationData] = await Promise.all([
        chatApi.getProjects(),
        chatApi.getConversations(0, 80),
      ])
      setProjects(projectData)
      setConversations(conversationData)
      if (projectForLoad && !projectData.some((project) => project.id === projectForLoad.id)) {
        onSelectProject(null)
      }
    } catch (err) {
      console.error('Failed to load chat sidebar data:', err)
    } finally {
      if (!silent) setLoading(false)
    }
  }, [onSelectProject, selectedProject])

  useEffect(() => {
    const projectChanged = sidebarLoadedRef.current && lastProjectIdRef.current !== selectedProject?.id
    lastProjectIdRef.current = selectedProject?.id
    void loadSidebarData({ silent: sidebarLoadedRef.current && !projectChanged })
    sidebarLoadedRef.current = true
  }, [loadSidebarData, selectedProject?.id])

  useEffect(() => {
    if (refreshKey === 0) return
    void loadSidebarData({ silent: true })
  }, [loadSidebarData, refreshKey])

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (settingsActive) return
      const mod = e.metaKey || e.ctrlKey
      if (!mod || e.key.toLowerCase() !== 'p') return
      e.preventDefault()
      openCreateProjectDialog()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [settingsActive])

  const handleRenameConversation = async (id: string, title: string) => {
    try {
      await chatApi.updateConversation(id, { title })
      await loadSidebarData({ silent: true })
    } catch (err) {
      console.error('Failed to rename conversation:', err)
    }
  }

  const handleDeleteConversation = async (id: string) => {
    if (!window.confirm('确定删除此对话？此操作无法撤销。')) return
    // B3：删"generating"会话先强制清父组件 in-flight/乐观状态，
    // 让乐观合并（visibleConversations）不再保留它。
    if (generatingConversationIds.has(id)) {
      onForceDropConversation?.(id)
      try {
        await chatApi.cancelStream(id)
      } catch (err) {
        console.error('Failed to cancel stream before delete:', err)
      }
    }
    try {
      await chatApi.deleteConversation(id)
    } catch (err) {
      console.error('Failed to delete conversation:', err)
    } finally {
      // 无论后端删除成功或抛错，都本地剔除该 id 并刷新侧栏，确保 ghost 立即消失。
      setConversations((items) => items.filter((item) => item.id !== id))
      onForceDropConversation?.(id)
      if (currentConversationId === id) {
        onConversationDeleted?.(id)
      }
      await loadSidebarData({ silent: true })
    }
  }

  const handleMoveConversationToProject = async (id: string, projectId: string | undefined) => {
    try {
      const conversation = await chatApi.updateConversation(id, { projectId: projectId ?? null })
      const conversationProjectId = conversation.project_id ?? conversation.projectId ?? null
      if (
        currentConversationId === id &&
        selectedProject &&
        conversationProjectId !== selectedProject.id
      ) {
        onConversationDeleted?.(id)
      }
      await loadSidebarData({ silent: true })
    } catch (err) {
      console.error('Failed to move conversation:', err)
    }
  }

  const openSectionMenu = () => {
    const button = sectionMenuButtonRef.current
    if (!button) return
    const rect = button.getBoundingClientRect()
    setSectionMenuAnchor({ left: rect.right - 200, top: rect.bottom + 4 })
  }

  function openCreateProjectDialog() {
    setDialogProject(null)
    setProjectError('')
  }

  const openProjectMenu = (projectId: string, button: HTMLButtonElement) => {
    const rect = button.getBoundingClientRect()
    setProjectMenuState({
      projectId,
      anchor: { left: rect.right - 180, top: rect.bottom + 4 },
    })
  }

  const handleSaveProject = async (name: string, rootPath?: string | null) => {
    setProjectSaving(true)
    setProjectError('')
    try {
      const project = dialogProject
        ? await chatApi.updateProject(dialogProject.id, { name, rootPath })
        : await chatApi.createProject(name, null, null, rootPath)
      onSelectProject(project)
      await loadSidebarData({ silent: true, projectOverride: project })
      setDialogProject(undefined)
    } catch (err) {
      setProjectError(typeof err === 'string' ? err : (err as Error).message || '项目保存失败')
    } finally {
      setProjectSaving(false)
    }
  }

  const handleOpenProjectFolder = async (project: ChatProject) => {
    try {
      await chatApi.openProjectFolder(project.id)
    } catch (err) {
      window.alert(typeof err === 'string' ? err : (err as Error).message || '打开项目文件夹失败')
    }
  }

  const handleDeleteProject = async (project: ChatProject) => {
    if (!window.confirm(`确定删除项目「${project.name}」？项目内的聊天会移出项目，不会被删除。`)) {
      return
    }
    try {
      await chatApi.deleteProject(project.id)
      if (selectedProject?.id === project.id) {
        onSelectProject(null)
        if (currentConversationId) onConversationDeleted?.(currentConversationId)
      }
      await loadSidebarData({ silent: true })
    } catch (err) {
      console.error('Failed to delete project:', err)
    }
  }

  const handleClearAllConversations = async () => {
    const targetConversations = selectedProject
      ? conversations.filter((conv) => conversationBelongsToProject(conv, selectedProject))
      : conversations
    if (targetConversations.length === 0) return
    const scope = selectedProject ? `项目「${selectedProject.name}」中的` : '全部'
    if (!window.confirm(`确定删除${scope} ${targetConversations.length} 个对话？此操作无法撤销。`)) return
    try {
      await Promise.all(targetConversations.map((conv) => chatApi.deleteConversation(conv.id)))
      if (currentConversationId && targetConversations.some((conv) => conv.id === currentConversationId)) {
        onConversationDeleted?.(currentConversationId)
      }
      await loadSidebarData({ silent: true })
    } catch (err) {
      console.error('Failed to clear conversations:', err)
    }
  }

  const visibleConversations = useMemo(() => {
    if (optimisticConversations.length === 0) return conversations
    const realConversationIds = new Set(conversations.map((item) => item.id))
    const visibleOptimisticConversations = optimisticConversations.filter((item) => {
      return generatingConversationIds.has(item.id) || !realConversationIds.has(item.id)
    })
    if (visibleOptimisticConversations.length === 0) return conversations
    const optimisticIds = new Set(visibleOptimisticConversations.map((item) => item.id))
    return [
      ...visibleOptimisticConversations,
      ...conversations.filter((item) => !optimisticIds.has(item.id)),
    ]
  }, [conversations, generatingConversationIds, optimisticConversations])

  const normalizedSearchQuery = searchQuery.trim().toLowerCase()

  const projectConversationMap = useMemo(() => {
    const map = new Map<string, ConversationListItem[]>()
    projects.forEach((project) => {
      map.set(
        project.id,
        visibleConversations.filter((conversation) => conversationBelongsToProject(conversation, project)),
      )
    })
    return map
  }, [projects, visibleConversations])

  const visibleProjects = projects

  const looseConversations = useMemo(
    () =>
      visibleConversations.filter((conversation) => {
        const belongsToKnownProject = projects.some((project) =>
          conversationBelongsToProject(conversation, project),
        )
        return !belongsToKnownProject
      }),
    [projects, visibleConversations],
  )

  const searchResults = useMemo(() => {
    return visibleConversations
      .filter((conversation) => {
        if (!normalizedSearchQuery) return true
        const project = findConversationProject(conversation, projects)
        return (
          conversationMatchesSearch(conversation, normalizedSearchQuery) ||
          (project ? projectMatchesSearch(project, normalizedSearchQuery) : false) ||
          (conversation.folder ?? '').toLowerCase().includes(normalizedSearchQuery)
        )
      })
      .slice(0, 9)
  }, [normalizedSearchQuery, projects, visibleConversations])

  const clearableConversationCount = selectedProject
    ? conversations.filter((conv) => conversationBelongsToProject(conv, selectedProject)).length
    : conversations.length

  const allVisibleProjectsCollapsed = visibleProjects.length > 0 &&
    visibleProjects.every((project) => collapsedProjectIds.has(project.id))

  const closeSearch = useCallback(() => {
    onSearchOpenChange(false)
    setSearchQuery('')
  }, [onSearchOpenChange])

  const handleSelectSearchConversation = useCallback((conversation: ConversationListItem) => {
    const project = findConversationProject(conversation, projects)
    if (project) {
      onSelectProject(project)
    } else if (selectedProject) {
      onSelectProject(null)
    }
    onSelectConversation(conversation.id)
    closeSearch()
  }, [closeSearch, onSelectConversation, onSelectProject, projects, selectedProject])

  const menuProject = projectMenuState
    ? projects.find((project) => project.id === projectMenuState.projectId)
    : undefined

  return (
    <>
      <aside
        ref={asideRef}
        className={`chat-sidebar-shell flex h-full w-[240px] shrink-0 flex-col${
          collapsed ? ' is-collapsed' : ''
        }`}
        aria-hidden={collapsed}
      >
        <div
          className={`${chatTitlebarRowClass} ${chatTitlebarMacInsetClass} pr-3`}
          data-tauri-drag-region
        >
          <ChatTitlebarActions
            sidebarExpanded
            onToggleSidebar={onToggleCollapsed}
            onNewConversation={onNewConversation}
          />
          <div className="min-w-0 flex-1" data-tauri-drag-region />
        </div>

      <nav className="shrink-0 space-y-0.5 px-3 pb-2" data-tauri-drag-region="false">
        <NavRow
          icon={<SquarePen size={17} strokeWidth={1.75} />}
          label="新建聊天"
          onClick={onNewConversation}
          iconMotion="group-hover:-rotate-6 group-hover:scale-110"
        />
        <NavRow
          icon={<Search size={17} strokeWidth={1.75} />}
          label="搜索"
          onClick={() => onSearchOpenChange(true)}
          active={searchOpen}
          iconMotion="group-hover:scale-110"
        />
        <ExtensionsNav
          activeItem={extensionsActive}
          onSelectItem={onOpenExtensionsItem}
        />
      </nav>

      <div className="mx-3 border-t border-neutral-200/90 dark:border-neutral-800" />

      <div className="custom-scrollbar flex min-h-0 flex-1 flex-col overflow-y-auto" data-tauri-drag-region="false">
        {loading ? (
          <div className="space-y-2 px-3 py-3" aria-label="加载中" aria-busy="true">
            {[0, 1, 2, 3, 4, 5].map((i) => (
              <div key={i} className="kv-skeleton h-7 rounded-lg" />
            ))}
          </div>
        ) : (
          <>
            <section className="group/projects px-3 pb-2 pt-3">
              <div className="flex items-center justify-between px-1">
                <button
                  type="button"
                  onClick={() => setProjectSectionCollapsed((collapsed) => !collapsed)}
                  className="flex min-w-0 items-center gap-1 rounded-md py-0.5 pr-2 text-left text-[13px] font-semibold text-neutral-400 transition-colors hover:text-neutral-600 dark:text-neutral-500 dark:hover:text-neutral-300"
                  aria-expanded={!projectSectionCollapsed}
                >
                  <span>项目</span>
                  <ChevronRight
                    size={13}
                    strokeWidth={2}
                    className={`shrink-0 transition-transform ${
                      projectSectionCollapsed ? '' : 'rotate-90'
                    }`}
                  />
                </button>
                <div className="flex shrink-0 items-center gap-1 opacity-0 transition-opacity group-hover/projects:opacity-100 group-focus-within/projects:opacity-100">
                  <button
                    type="button"
                    onClick={() => {
                      setCollapsedProjectIds((previous) => {
                        const next = new Set(previous)
                        if (allVisibleProjectsCollapsed) {
                          visibleProjects.forEach((project) => next.delete(project.id))
                        } else {
                          visibleProjects.forEach((project) => next.add(project.id))
                        }
                        return next
                      })
                    }}
                    className="rounded-md p-0.5 text-neutral-400 transition-colors hover:bg-black/[0.06] hover:text-neutral-600 dark:hover:bg-white/[0.1] dark:hover:text-neutral-200"
                    title={allVisibleProjectsCollapsed ? '展开全部项目' : '折叠全部项目'}
                    aria-label={allVisibleProjectsCollapsed ? '展开全部项目' : '折叠全部项目'}
                  >
                    <MoreHorizontal size={15} />
                  </button>
                  <button
                    type="button"
                    onClick={openCreateProjectDialog}
                    className="rounded-md p-0.5 text-neutral-400 transition-colors hover:bg-black/[0.06] hover:text-neutral-600 dark:hover:bg-white/[0.1] dark:hover:text-neutral-200"
                    title={`新建项目 (${modLabel}P)`}
                    aria-label="新建项目"
                  >
                    <FolderPlus size={15} strokeWidth={1.75} />
                  </button>
                </div>
              </div>

              {!projectSectionCollapsed && (
                <div className="mt-1.5 space-y-1">
                  {visibleProjects.map((project, index) => {
                    const active = selectedProject?.id === project.id
                    const projectConversations = projectConversationMap.get(project.id) ?? []
                    const collapsedProject = collapsedProjectIds.has(project.id)
                    const expanded = expandedProjectConversationIds.has(project.id)
                    const previewConversations = expanded
                      ? projectConversations
                      : projectConversations.slice(0, PROJECT_PREVIEW_LIMIT)
                    return (
                      <div key={project.id}>
                        <div
                          className={`chat-motion-row group flex min-w-0 items-center rounded-lg ${
                            active
                              ? 'bg-black/[0.04] dark:bg-white/[0.08]'
                              : 'hover:bg-black/[0.035] dark:hover:bg-white/[0.06]'
                          }`}
                          style={{
                            ['--chat-motion-delay' as string]: `${Math.min(index, 12) * 18}ms`,
                          }}
                        >
                          <button
                            type="button"
                            onClick={() => {
                              setCollapsedProjectIds((previous) => {
                                const next = new Set(previous)
                                if (next.has(project.id)) next.delete(project.id)
                                else next.add(project.id)
                                return next
                              })
                            }}
                            className={`flex min-w-0 flex-1 items-center gap-1.5 px-2 py-1 text-left text-[13px] ${
                              active
                                ? 'font-semibold text-neutral-900 dark:text-neutral-100'
                                : 'font-medium text-neutral-600 dark:text-neutral-300'
                            }`}
                            title={collapsedProject ? `展开 ${project.name}` : `折叠 ${project.name}`}
                            aria-expanded={!collapsedProject}
                          >
                            <ChevronRight
                              size={13}
                              strokeWidth={2}
                              className={`shrink-0 text-neutral-400 transition-transform dark:text-neutral-500 ${
                                collapsedProject ? '' : 'rotate-90'
                              }`}
                            />
                            <Folder
                              size={15}
                              strokeWidth={1.75}
                              className="shrink-0 text-neutral-500 dark:text-neutral-400"
                            />
                            <span className="min-w-0 truncate">{project.name}</span>
                          </button>
                          <button
                            type="button"
                            onClick={(e) => {
                              e.stopPropagation()
                              openProjectMenu(project.id, e.currentTarget)
                            }}
                            className={`shrink-0 rounded-md p-0.5 text-neutral-400 transition-opacity hover:bg-black/[0.06] hover:text-neutral-600 dark:hover:bg-white/[0.1] dark:hover:text-neutral-200 ${
                              projectMenuState?.projectId === project.id
                                ? 'opacity-100'
                                : 'opacity-0 group-hover:opacity-100'
                            }`}
                            aria-label="项目操作"
                          >
                            <MoreHorizontal size={15} />
                          </button>
                          <button
                            type="button"
                            onClick={(e) => {
                              e.stopPropagation()
                              setCollapsedProjectIds((previous) => {
                                const next = new Set(previous)
                                next.delete(project.id)
                                return next
                              })
                              onSelectProject(project)
                            }}
                            className="mr-1 shrink-0 rounded-md p-0.5 text-neutral-400 opacity-0 transition-opacity hover:bg-black/[0.06] hover:text-neutral-600 group-hover:opacity-100 dark:hover:bg-white/[0.1] dark:hover:text-neutral-200"
                            aria-label="新建聊天"
                            title="新建聊天"
                          >
                            <SquarePen size={15} strokeWidth={1.75} />
                          </button>
                        </div>

                      {!collapsedProject && previewConversations.length > 0 && (
                        <ConversationList
                          conversations={previewConversations}
                          currentConversationId={currentConversationId}
                          generatingConversationIds={generatingConversationIds}
                          projects={projects}
                          compact
                          indent
                          showAssistantName={false}
                          onSelectConversation={(id) => {
                            if (selectedProject?.id !== project.id) onSelectProject(project)
                            onSelectConversation(id)
                          }}
                          onRenameConversation={handleRenameConversation}
                          onDeleteConversation={handleDeleteConversation}
                          onMoveConversationToProject={handleMoveConversationToProject}
                        />
                      )}

                      {!collapsedProject && projectConversations.length > PROJECT_PREVIEW_LIMIT && (
                        <button
                          type="button"
                          onClick={() => {
                            setExpandedProjectConversationIds((previous) => {
                              const next = new Set(previous)
                              if (next.has(project.id)) next.delete(project.id)
                              else next.add(project.id)
                              return next
                            })
                          }}
                          className="ml-8 rounded-md px-2.5 py-0.5 text-left text-[13px] font-medium text-neutral-400 transition-colors hover:bg-black/[0.035] hover:text-neutral-600 dark:text-neutral-500 dark:hover:bg-white/[0.06] dark:hover:text-neutral-300"
                        >
                          {expanded ? '收起' : '展开显示'}
                        </button>
                      )}
                      </div>
                    )
                  })}
                </div>
              )}
            </section>

            <section className="group/conversations px-3 pb-5 pt-2">
              <div className="flex min-w-0 items-center justify-between px-1">
                <button
                  type="button"
                  onClick={() => setConversationSectionCollapsed((collapsed) => !collapsed)}
                  className="flex min-w-0 items-center gap-1 rounded-md py-0.5 pr-2 text-left text-[13px] font-semibold text-neutral-400 transition-colors hover:text-neutral-600 dark:text-neutral-500 dark:hover:text-neutral-300"
                  aria-expanded={!conversationSectionCollapsed}
                >
                  <span>对话</span>
                  <ChevronRight
                    size={13}
                    strokeWidth={2}
                    className={`shrink-0 transition-transform ${
                      conversationSectionCollapsed ? '' : 'rotate-90'
                    }`}
                  />
                </button>
                <div
                  className={`flex shrink-0 items-center gap-1 transition-opacity ${
                    sectionMenuAnchor
                      ? 'opacity-100'
                      : 'opacity-0 group-hover/conversations:opacity-100 group-focus-within/conversations:opacity-100'
                  }`}
                >
                  <button
                    ref={sectionMenuButtonRef}
                    type="button"
                    onClick={openSectionMenu}
                    className={`rounded-md p-0.5 text-neutral-400 transition-colors hover:bg-black/[0.06] hover:text-neutral-600 dark:hover:bg-white/[0.1] dark:hover:text-neutral-200 ${
                      sectionMenuAnchor
                        ? 'bg-black/[0.06] text-neutral-600 dark:bg-white/[0.1] dark:text-neutral-200'
                        : ''
                    }`}
                    aria-label="对话列表操作"
                    aria-haspopup="menu"
                    aria-expanded={sectionMenuAnchor !== null}
                  >
                    <MoreHorizontal size={15} />
                  </button>
                  <button
                    type="button"
                    onClick={onNewConversation}
                    className="rounded-md p-0.5 text-neutral-400 transition-colors hover:bg-black/[0.06] hover:text-neutral-600 dark:hover:bg-white/[0.1] dark:hover:text-neutral-200"
                    aria-label="新建聊天"
                    title="新建聊天"
                  >
                    <SquarePen size={15} strokeWidth={1.75} />
                  </button>
                </div>
              </div>

              {sectionMenuAnchor && (
                <ChatSectionMenu
                  anchor={sectionMenuAnchor}
                  hasConversations={clearableConversationCount > 0}
                  onNewConversation={onNewConversation}
                  onOpenSearch={() => onSearchOpenChange(true)}
                  onClearAll={() => void handleClearAllConversations()}
                  onClose={() => setSectionMenuAnchor(null)}
                />
              )}

              {!conversationSectionCollapsed ? (
                <div className="mt-1.5">
                  {looseConversations.length > 0 ? (
                    <ConversationList
                      conversations={looseConversations}
                      currentConversationId={currentConversationId}
                      generatingConversationIds={generatingConversationIds}
                      projects={projects}
                      compact
                      showAssistantName={false}
                      onSelectConversation={(id) => {
                        if (selectedProject) onSelectProject(null)
                        onSelectConversation(id)
                      }}
                      onRenameConversation={handleRenameConversation}
                      onDeleteConversation={handleDeleteConversation}
                      onMoveConversationToProject={handleMoveConversationToProject}
                    />
                  ) : null}
                </div>
              ) : null}
            </section>
          </>
        )}
      </div>

      <SidebarUserFooter
        profile={userProfile}
        settingsActive={settingsActive}
        onOpenSettings={onOpenSettings}
      />

      {projectMenuState && menuProject && (
        <ProjectContextMenu
          anchor={projectMenuState.anchor}
          hasRootFolder={Boolean((menuProject.root_path ?? menuProject.rootPath ?? '').trim())}
          onRename={() => {
            setDialogProject(menuProject)
            setProjectError('')
          }}
          onOpenFolder={() => void handleOpenProjectFolder(menuProject)}
          onDelete={() => void handleDeleteProject(menuProject)}
          onClose={() => setProjectMenuState(null)}
        />
      )}

      {dialogProject !== undefined && (
        <ProjectDialog
          project={dialogProject}
          saving={projectSaving}
          error={projectError}
          onSave={(name, rootPath) => void handleSaveProject(name, rootPath)}
          onClose={() => setDialogProject(undefined)}
        />
      )}
    </aside>

    {searchOpen && (
      <SearchDialog
        query={searchQuery}
        results={searchResults}
        currentConversationId={currentConversationId}
        projects={projects}
        onQueryChange={setSearchQuery}
        onSelectConversation={handleSelectSearchConversation}
        onClose={closeSearch}
      />
    )}
    </>
  )
})
