import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  ChevronLeft,
  ChevronRight,
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

interface SidebarProps {
  currentConversationId?: string
  generatingConversationIds?: ReadonlySet<string>
  optimisticConversations?: ConversationListItem[]
  selectedProject?: ChatProject | null
  onSelectProject: (project: ChatProject | null) => void
  onSelectConversation: (id: string) => void
  onNewConversation: () => void
  onConversationDeleted?: (id: string) => void
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
  shortcut?: string
  onClick?: () => void
  disabled?: boolean
  active?: boolean
  /** 图标在 hover 时的微动效（group-hover transform 工具类） */
  iconMotion?: string
}

function NavRow({ icon, label, shortcut, onClick, disabled, active, iconMotion }: NavRowProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={`group flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left text-[13px] transition-colors disabled:cursor-default disabled:opacity-40 ${
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
      {shortcut && (
        <span className="shrink-0 text-[11px] text-neutral-400 opacity-0 transition-opacity group-hover:opacity-100 dark:text-neutral-500">
          {shortcut}
        </span>
      )}
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
        className={`group flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left text-[13px] font-medium transition-colors ${
          highlighted
            ? 'bg-black/[0.06] text-neutral-900 dark:bg-white/[0.1] dark:text-neutral-50'
            : 'text-neutral-800 hover:bg-black/[0.04] dark:text-neutral-200 dark:hover:bg-white/[0.06]'
        }`}
        aria-expanded={expanded}
      >
        <span className="flex h-5 w-5 shrink-0 items-center justify-center text-neutral-600 transition duration-300 ease-out will-change-transform group-hover:text-neutral-800 group-active:scale-90 group-hover:rotate-3 group-hover:scale-110 dark:text-neutral-400 dark:group-hover:text-neutral-200">
          <LayoutGrid size={17} strokeWidth={1.75} />
        </span>
        <span className="min-w-0 flex-1 truncate">拓展</span>
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

export const Sidebar = memo(function Sidebar({
  currentConversationId,
  generatingConversationIds = new Set(),
  optimisticConversations = [],
  selectedProject = null,
  onSelectProject,
  onSelectConversation,
  onNewConversation,
  onConversationDeleted,
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
  const [conversations, setConversations] = useState<ConversationListItem[]>([])
  const [projects, setProjects] = useState<ChatProject[]>([])
  const [searchQuery, setSearchQuery] = useState('')
  const [loading, setLoading] = useState(false)
  const [sectionMenuAnchor, setSectionMenuAnchor] = useState<ConversationMenuAnchor | null>(null)
  const [projectMenuState, setProjectMenuState] = useState<{
    projectId: string
    anchor: ConversationMenuAnchor
  } | null>(null)
  const [dialogProject, setDialogProject] = useState<ChatProject | null | undefined>(undefined)
  const [projectSaving, setProjectSaving] = useState(false)
  const [projectError, setProjectError] = useState('')
  const searchInputRef = useRef<HTMLInputElement>(null)
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
        chatApi.getConversations(0, 50, projectForLoad?.name),
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
    if (searchOpen) {
      searchInputRef.current?.focus()
    }
  }, [searchOpen])

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

  const projectFolders = useMemo(
    () => projects.map((project) => project.name),
    [projects],
  )

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
    try {
      if (generatingConversationIds.has(id)) {
        await chatApi.cancelStream(id)
      }
      await chatApi.deleteConversation(id)
      if (currentConversationId === id) {
        onConversationDeleted?.(id)
      }
      await loadSidebarData({ silent: true })
    } catch (err) {
      console.error('Failed to delete conversation:', err)
    }
  }

  const handleMoveConversationToFolder = async (id: string, folder: string | undefined) => {
    try {
      const conversation = await chatApi.updateConversation(id, { folder })
      if (
        currentConversationId === id &&
        selectedProject &&
        conversation.folder !== selectedProject.name
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

  const handleSaveProject = async (name: string) => {
    setProjectSaving(true)
    setProjectError('')
    try {
      const project = dialogProject
        ? await chatApi.updateProject(dialogProject.id, { name })
        : await chatApi.createProject(name)
      onSelectProject(project)
      await loadSidebarData({ silent: true, projectOverride: project })
      setDialogProject(undefined)
    } catch (err) {
      setProjectError(typeof err === 'string' ? err : (err as Error).message || '项目保存失败')
    } finally {
      setProjectSaving(false)
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
    if (conversations.length === 0) return
    const scope = selectedProject ? `项目「${selectedProject.name}」中的` : '全部'
    if (!window.confirm(`确定删除${scope} ${conversations.length} 个对话？此操作无法撤销。`)) return
    try {
      await Promise.all(conversations.map((conv) => chatApi.deleteConversation(conv.id)))
      if (currentConversationId) {
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
      if (selectedProject && item.folder !== selectedProject.name) return false
      return generatingConversationIds.has(item.id) || !realConversationIds.has(item.id)
    })
    if (visibleOptimisticConversations.length === 0) return conversations
    const optimisticIds = new Set(visibleOptimisticConversations.map((item) => item.id))
    return [
      ...visibleOptimisticConversations,
      ...conversations.filter((item) => !optimisticIds.has(item.id)),
    ]
  }, [conversations, generatingConversationIds, optimisticConversations, selectedProject])

  const filteredConversations = useMemo(() => {
    const normalizedSearchQuery = searchQuery.trim().toLowerCase()
    if (!normalizedSearchQuery) return visibleConversations
    return visibleConversations.filter(
      (c) =>
        c.title.toLowerCase().includes(normalizedSearchQuery) ||
        c.preview.toLowerCase().includes(normalizedSearchQuery),
    )
  }, [searchQuery, visibleConversations])

  const menuProject = projectMenuState
    ? projects.find((project) => project.id === projectMenuState.projectId)
    : undefined

  if (collapsed) {
    return null
  }

  return (
    <aside className="flex h-full w-[260px] shrink-0 flex-col border-r border-neutral-200/80 bg-[#f7f7f8] dark:border-neutral-800 dark:bg-[#1c1c1e]">
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

      <nav className="shrink-0 space-y-0.5 px-2 pb-2" data-tauri-drag-region="false">
        <NavRow
          icon={<SquarePen size={17} strokeWidth={1.75} />}
          label="新建聊天"
          shortcut={`${modLabel}N`}
          onClick={onNewConversation}
          iconMotion="group-hover:-rotate-6 group-hover:scale-110"
        />
        <NavRow
          icon={<FolderPlus size={17} strokeWidth={1.75} />}
          label="新建项目"
          shortcut={`${modLabel}P`}
          onClick={openCreateProjectDialog}
          iconMotion="group-hover:-translate-y-px group-hover:scale-110"
        />
        <ExtensionsNav
          activeItem={extensionsActive}
          onSelectItem={onOpenExtensionsItem}
        />
      </nav>

      <div className="mx-3 border-t border-neutral-200/90 dark:border-neutral-800" />

      <div className="custom-scrollbar flex min-h-0 flex-1 flex-col overflow-y-auto" data-tauri-drag-region="false">
        {projects.length > 0 && (
          <>
            <div className="px-2 pt-2 pb-1">
              <div className="px-3 py-1 text-[12px] font-medium text-neutral-400 dark:text-neutral-500">
                项目
              </div>
              <div className="space-y-0.5">
                {projects.map((project, index) => {
                  const active = selectedProject?.id === project.id
                  return (
                    <div
                      key={project.id}
                      className={`chat-motion-row group flex min-w-0 items-center rounded-lg ${
                        active
                          ? 'bg-black/[0.06] dark:bg-white/[0.1]'
                          : 'hover:bg-black/[0.04] dark:hover:bg-white/[0.06]'
                      }`}
                      style={{
                        ['--chat-motion-delay' as string]: `${Math.min(index, 12) * 18}ms`,
                      }}
                    >
                      <button
                        type="button"
                        onClick={() => onSelectProject(project)}
                        className={`min-w-0 flex-1 truncate px-3 py-1.5 text-left text-[13px] ${
                          active
                            ? 'font-medium text-neutral-900 dark:text-neutral-100'
                            : 'text-neutral-700 dark:text-neutral-300'
                        }`}
                        title={project.name}
                      >
                        {project.name}
                      </button>
                      <button
                        type="button"
                        onClick={(e) => {
                          e.stopPropagation()
                          openProjectMenu(project.id, e.currentTarget)
                        }}
                        className={`mr-1 shrink-0 rounded-md p-1 text-neutral-400 transition-opacity hover:bg-black/[0.06] hover:text-neutral-600 dark:hover:bg-white/[0.1] dark:hover:text-neutral-200 ${
                          projectMenuState?.projectId === project.id
                            ? 'opacity-100'
                            : 'opacity-0 group-hover:opacity-100'
                        }`}
                        aria-label="项目操作"
                      >
                        <MoreHorizontal size={16} />
                      </button>
                    </div>
                  )
                })}
              </div>
            </div>
            <div className="mx-3 mt-1 border-t border-neutral-200/90 dark:border-neutral-800" />
          </>
        )}

        <div className={`flex min-h-0 flex-col ${projects.length > 0 ? 'pt-2' : 'pt-1'}`}>
          <div className="flex min-w-0 items-center rounded-lg px-2 pb-1">
            <div className="flex min-w-0 flex-1 items-center gap-0.5 px-3 py-2">
              {selectedProject && (
                <button
                  type="button"
                  onClick={() => onSelectProject(null)}
                  className="-ml-1 shrink-0 rounded-md p-0.5 text-neutral-400 transition-colors hover:bg-black/[0.06] hover:text-neutral-600 dark:hover:bg-white/[0.1] dark:hover:text-neutral-200"
                  title="返回最近"
                  aria-label="返回最近"
                >
                  <ChevronLeft size={15} strokeWidth={2} />
                </button>
              )}
              <span className="min-w-0 truncate text-[13px] font-medium text-neutral-500 dark:text-neutral-400">
                {selectedProject ? selectedProject.name : '最近'}
              </span>
            </div>
            <button
              type="button"
              onClick={() => onSearchOpenChange(!searchOpen)}
              className={`shrink-0 rounded-md p-1 text-neutral-400 transition-colors hover:bg-black/[0.06] hover:text-neutral-600 dark:hover:bg-white/[0.1] dark:hover:text-neutral-200 ${
                searchOpen
                  ? 'bg-black/[0.06] text-neutral-600 dark:bg-white/[0.1] dark:text-neutral-200'
                  : ''
              }`}
              title={`搜索 (${modLabel}K)`}
              aria-label="搜索对话"
              aria-pressed={searchOpen}
            >
              <Search size={16} strokeWidth={1.75} />
            </button>
            <button
              ref={sectionMenuButtonRef}
              type="button"
              onClick={openSectionMenu}
              className={`mr-1 shrink-0 rounded-md p-1 text-neutral-400 transition-colors hover:bg-black/[0.06] hover:text-neutral-600 dark:hover:bg-white/[0.1] dark:hover:text-neutral-200 ${
                sectionMenuAnchor
                  ? 'bg-black/[0.06] text-neutral-600 dark:bg-white/[0.1] dark:text-neutral-200'
                  : ''
              }`}
              aria-label="最近列表操作"
              aria-haspopup="menu"
              aria-expanded={sectionMenuAnchor !== null}
            >
              <MoreHorizontal size={16} />
            </button>
          </div>

          {sectionMenuAnchor && (
            <ChatSectionMenu
              anchor={sectionMenuAnchor}
              hasConversations={conversations.length > 0}
              onNewConversation={onNewConversation}
              onOpenSearch={() => onSearchOpenChange(true)}
              onClearAll={() => void handleClearAllConversations()}
              onClose={() => setSectionMenuAnchor(null)}
            />
          )}

          {searchOpen && (
            <div className="chat-motion-search-reveal px-3 pb-2">
              <div className="relative">
                <Search
                  size={15}
                  className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-neutral-400"
                />
                <input
                  ref={searchInputRef}
                  type="text"
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Escape') {
                      onSearchOpenChange(false)
                      setSearchQuery('')
                    }
                  }}
                  placeholder={selectedProject ? '搜索项目聊天' : '搜索对话'}
                  className="w-full rounded-lg border border-neutral-200/90 bg-white py-2 pl-8 pr-3 text-[13px] text-neutral-900 outline-none ring-0 placeholder:text-neutral-400 focus:border-neutral-300 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100"
                />
              </div>
            </div>
          )}

          <div className="px-2 pb-3">
            {loading ? (
              <div className="px-3 py-6 text-center text-[13px] text-neutral-400">加载中…</div>
            ) : (
              <ConversationList
                conversations={filteredConversations}
                currentConversationId={currentConversationId}
                generatingConversationIds={generatingConversationIds}
                projectFolders={projectFolders}
                emptyLabel={selectedProject ? '项目里还没有对话' : '暂无对话'}
                onSelectConversation={onSelectConversation}
                onRenameConversation={handleRenameConversation}
                onDeleteConversation={handleDeleteConversation}
                onMoveConversationToFolder={handleMoveConversationToFolder}
              />
            )}
          </div>
        </div>
      </div>

      <SidebarUserFooter
        profile={userProfile}
        settingsActive={settingsActive}
        onOpenSettings={onOpenSettings}
      />

      {projectMenuState && menuProject && (
        <ProjectContextMenu
          anchor={projectMenuState.anchor}
          onRename={() => {
            setDialogProject(menuProject)
            setProjectError('')
          }}
          onDelete={() => void handleDeleteProject(menuProject)}
          onClose={() => setProjectMenuState(null)}
        />
      )}

      {dialogProject !== undefined && (
        <ProjectDialog
          project={dialogProject}
          saving={projectSaving}
          error={projectError}
          onSave={(name) => void handleSaveProject(name)}
          onClose={() => setDialogProject(undefined)}
        />
      )}
    </aside>
  )
})
