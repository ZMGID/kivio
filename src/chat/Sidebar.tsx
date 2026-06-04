import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
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
import { WindowControls } from './WindowControls'
import { chatApi } from './api'
import { ChatTitlebarActions } from './ChatTitlebarActions'
import { chatTitlebarMacInsetClass, chatTitlebarRowClass, isMac } from './platform'
import type { ConversationMenuAnchor } from './ConversationContextMenu'

const modLabel = isMac ? '⌘' : 'Ctrl'

interface SidebarProps {
  currentConversationId?: string
  selectedProject?: ChatProject | null
  onSelectProject: (project: ChatProject | null) => void
  onSelectConversation: (id: string) => void
  onNewConversation: () => void
  onConversationDeleted?: (id: string) => void
  onOpenSettings: () => void
  onOpenAssistantCenter: () => void
  settingsActive?: boolean
  assistantCenterActive?: boolean
  collapsed: boolean
  onToggleCollapsed: () => void
  refreshKey: number
  searchOpen: boolean
  onSearchOpenChange: (open: boolean) => void
}

interface NavRowProps {
  icon: React.ReactNode
  label: string
  shortcut?: string
  onClick?: () => void
  disabled?: boolean
  active?: boolean
}

function NavRow({ icon, label, shortcut, onClick, disabled, active }: NavRowProps) {
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
      <span className="flex h-5 w-5 shrink-0 items-center justify-center text-neutral-600 dark:text-neutral-400">
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

export const Sidebar = memo(function Sidebar({
  currentConversationId,
  selectedProject = null,
  onSelectProject,
  onSelectConversation,
  onNewConversation,
  onConversationDeleted,
  onOpenSettings,
  onOpenAssistantCenter,
  settingsActive = false,
  assistantCenterActive = false,
  collapsed,
  onToggleCollapsed,
  refreshKey,
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

  const loadSidebarData = useCallback(async (projectOverride?: ChatProject | null) => {
    const projectForLoad = projectOverride === undefined ? selectedProject : projectOverride
    setLoading(true)
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
      setLoading(false)
    }
  }, [onSelectProject, selectedProject])

  useEffect(() => {
    void loadSidebarData()
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
      await loadSidebarData()
    } catch (err) {
      console.error('Failed to rename conversation:', err)
    }
  }

  const handleDeleteConversation = async (id: string) => {
    if (!window.confirm('确定删除此对话？此操作无法撤销。')) return
    try {
      await chatApi.deleteConversation(id)
      if (currentConversationId === id) {
        onConversationDeleted?.(id)
      }
      await loadSidebarData()
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
      await loadSidebarData()
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
      await loadSidebarData(project)
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
      await loadSidebarData()
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
      await loadSidebarData()
    } catch (err) {
      console.error('Failed to clear conversations:', err)
    }
  }

  const filteredConversations = useMemo(() => {
    const normalizedSearchQuery = searchQuery.trim().toLowerCase()
    if (!normalizedSearchQuery) return conversations
    return conversations.filter(
      (c) =>
        c.title.toLowerCase().includes(normalizedSearchQuery) ||
        c.preview.toLowerCase().includes(normalizedSearchQuery),
    )
  }, [conversations, searchQuery])

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
        {!isMac && <WindowControls />}
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
        />
        <NavRow
          icon={<FolderPlus size={17} strokeWidth={1.75} />}
          label="新建项目"
          shortcut={`${modLabel}P`}
          onClick={openCreateProjectDialog}
        />
        <NavRow
          icon={<Search size={17} strokeWidth={1.75} />}
          label="搜索"
          shortcut={`${modLabel}K`}
          onClick={() => onSearchOpenChange(!searchOpen)}
        />
        <NavRow
          icon={<LayoutGrid size={17} strokeWidth={1.75} />}
          label="助手中心"
          active={assistantCenterActive}
          onClick={onOpenAssistantCenter}
        />
        <NavRow
          icon={<SettingsIcon size={17} strokeWidth={1.75} />}
          label="设置"
          active={settingsActive}
          onClick={onOpenSettings}
        />
      </nav>

      <div className="mx-3 border-t border-neutral-200/90 dark:border-neutral-800" />

      <div className="custom-scrollbar flex min-h-0 flex-1 flex-col overflow-y-auto" data-tauri-drag-region="false">
        <div className="px-2 pt-2">
          <div className="px-3 py-2 text-[13px] font-medium text-neutral-500 dark:text-neutral-400">
            项目
          </div>
          <button
            type="button"
            onClick={() => onSelectProject(null)}
            className={`flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left text-[13px] transition-colors ${
              !selectedProject
                ? 'bg-black/[0.06] font-medium text-neutral-900 dark:bg-white/[0.1] dark:text-neutral-100'
                : 'text-neutral-700 hover:bg-black/[0.04] dark:text-neutral-300 dark:hover:bg-white/[0.06]'
            }`}
          >
            <Folder size={16} strokeWidth={1.75} className="shrink-0 text-neutral-500" />
            <span className="min-w-0 flex-1 truncate">全部聊天</span>
          </button>

          <div className="mt-1 space-y-0.5">
            {projects.map((project) => {
              const active = selectedProject?.id === project.id
              return (
                <div
                  key={project.id}
                  className={`group flex min-w-0 items-center rounded-lg ${
                    active
                      ? 'bg-black/[0.06] dark:bg-white/[0.1]'
                      : 'hover:bg-black/[0.04] dark:hover:bg-white/[0.06]'
                  }`}
                >
                  <button
                    type="button"
                    onClick={() => onSelectProject(project)}
                    className={`min-w-0 flex-1 truncate px-3 py-2 text-left text-[13px] ${
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
            {projects.length === 0 && (
              <div className="px-3 py-2 text-[13px] text-neutral-400 dark:text-neutral-500">
                暂无项目
              </div>
            )}
          </div>
        </div>

        <div className="mx-3 mt-3 border-t border-neutral-200/90 dark:border-neutral-800" />

        <div className="flex min-h-0 flex-col pt-2">
          <div className="flex min-w-0 items-center rounded-lg px-2 pb-1">
            <span className="min-w-0 flex-1 px-3 py-2 text-[13px] font-medium text-neutral-500 dark:text-neutral-400">
              {selectedProject ? selectedProject.name : '聊天'}
            </span>
            <button
              ref={sectionMenuButtonRef}
              type="button"
              onClick={openSectionMenu}
              className={`mr-1 shrink-0 rounded-md p-1 text-neutral-400 transition-colors hover:bg-black/[0.06] hover:text-neutral-600 dark:hover:bg-white/[0.1] dark:hover:text-neutral-200 ${
                sectionMenuAnchor
                  ? 'bg-black/[0.06] text-neutral-600 dark:bg-white/[0.1] dark:text-neutral-200'
                  : ''
              }`}
              aria-label="聊天列表操作"
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
            <div className="px-3 pb-2">
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
