import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import {
  ChevronRight,
  Folder,
  FolderPlus,
  Layers,
  LayoutGrid,
  MoreHorizontal,
  Plus,
  Search,
  Settings as SettingsIcon,
  SquarePen,
} from 'lucide-react'
import type { ChatAssistant, ChatProject, ChatSet, ConversationListItem } from './types'
import { ConversationList } from './ConversationList'
import { ChatSectionMenu } from './ChatSectionMenu'
import { ProjectContextMenu } from './ProjectContextMenu'
import { ProjectDialog } from './ProjectDialog'
import { SetContextMenu } from './SetContextMenu'
import { SetDialog } from './SetDialog'
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
  { id: 'mcp', label: '連接器' },
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
  selectedSet?: ChatSet | null
  onSelectSet: (set: ChatSet | null) => void
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
          title="設定"
          aria-label="設定"
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
  /** 圖示在 hover 時的微動效（group-hover transform 工具類） */
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
        <span className="min-w-0 flex-1 truncate">外掛</span>
        <ChevronRight
          size={14}
          strokeWidth={2}
          className={`shrink-0 text-neutral-400 transition-transform duration-200 dark:text-neutral-500 ${
            expanded ? 'rotate-90' : ''
          }`}
        />
      </button>
      {expanded && (
        <div className="relative ml-[34px] mt-0.5 border-l border-neutral-300 pl-2 dark:border-neutral-600">
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
  sets,
  onQueryChange,
  onSelectConversation,
  onClose,
}: {
  query: string
  results: ConversationListItem[]
  currentConversationId?: string
  projects: ChatProject[]
  sets: ChatSet[]
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
        aria-label="搜尋對話"
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
            placeholder="搜尋對話"
            className="min-w-0 flex-1 bg-transparent text-[14px] font-medium text-neutral-900 outline-none placeholder:text-neutral-400 dark:text-neutral-100 dark:placeholder:text-neutral-500"
          />
        </div>

        <div className="px-3 pb-1 pt-2 text-[11px] font-semibold uppercase tracking-wide text-neutral-400 dark:text-neutral-500">
          {normalizedQuery ? '搜尋結果' : '近期對話'}
        </div>

        <div className="custom-scrollbar min-h-0 overflow-y-auto px-1.5 pb-1.5">
          {results.length > 0 ? (
            results.map((conversation) => {
              const active = conversation.id === currentConversationId
              const projectLabel = conversationProjectLabel(conversation, projects)
              const setId = conversation.set_id ?? conversation.setId ?? null
              const setLabel = setId ? sets.find((s) => s.id === setId)?.name ?? '' : ''
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
                  {setLabel && (
                    <span className="max-w-[100px] shrink-0 truncate text-[12px] text-neutral-400 dark:text-neutral-500">
                      集 · {setLabel}
                    </span>
                  )}
                  {!setLabel && projectLabel && (
                    <span className="max-w-[100px] shrink-0 truncate text-[12px] text-neutral-400 dark:text-neutral-500">
                      {projectLabel}
                    </span>
                  )}
                </button>
              )
            })
          ) : (
            <div className="px-3 py-6 text-center text-[13px] text-neutral-400 dark:text-neutral-500">
              沒有匹配的對話
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
  selectedSet = null,
  onSelectSet,
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
  // 摺疊後側欄仍掛載（用於滑出動畫），用 inert 讓其退出 tab 序 / 不可點選 / 不進 a11y 樹。
  // useLayoutEffect：在繪製前與 JSX 裡的 aria-hidden 原子地一起生效，避免短暫可聚焦視窗。
  useLayoutEffect(() => {
    const el = asideRef.current
    if (el) el.inert = collapsed
  }, [collapsed])
  const [conversations, setConversations] = useState<ConversationListItem[]>([])
  const [projects, setProjects] = useState<ChatProject[]>([])
  const [sets, setSets] = useState<ChatSet[]>([])
  const [assistants, setAssistants] = useState<ChatAssistant[]>([])
  const [searchQuery, setSearchQuery] = useState('')
  // 後端全量索引搜尋結果（覆蓋所有對話，不止已載入的前 80）；空查詢/非 Tauri 時為空，回退客戶端過濾。
  const [fullSearchResults, setFullSearchResults] = useState<ConversationListItem[]>([])
  // 側欄三塊改為橫排標籤頁：同一時刻只顯示一塊（對話/集/專案）。
  const [activeTab, setActiveTab] = useState<'conversations' | 'sets' | 'projects'>('conversations')
  const [collapsedProjectIds, setCollapsedProjectIds] = useState<Set<string>>(
    () => new Set(),
  )
  const [expandedProjectConversationIds, setExpandedProjectConversationIds] = useState<Set<string>>(
    () => new Set(),
  )
  const [collapsedSetIds, setCollapsedSetIds] = useState<Set<string>>(() => new Set())
  const [expandedSetConversationIds, setExpandedSetConversationIds] = useState<Set<string>>(
    () => new Set(),
  )
  const [loading, setLoading] = useState(false)
  const [sectionMenuAnchor, setSectionMenuAnchor] = useState<ConversationMenuAnchor | null>(null)
  const [projectMenuState, setProjectMenuState] = useState<{
    projectId: string
    anchor: ConversationMenuAnchor
  } | null>(null)
  const [dialogProject, setDialogProject] = useState<ChatProject | null | undefined>(undefined)
  const [projectSaving, setProjectSaving] = useState(false)
  const [projectError, setProjectError] = useState('')
  const [setMenuState, setSetMenuState] = useState<{
    setId: string
    anchor: ConversationMenuAnchor
  } | null>(null)
  const [dialogSet, setDialogSet] = useState<ChatSet | null | undefined>(undefined)
  const [setDialogSaving, setSetDialogSaving] = useState(false)
  const [setDialogError, setSetDialogError] = useState('')
  const sectionMenuButtonRef = useRef<HTMLButtonElement>(null)
  const sidebarLoadedRef = useRef(false)
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

  const loadSidebarData = useCallback(async (options?: { silent?: boolean; projectOverride?: ChatProject | null; setOverride?: ChatSet | null }) => {
    const projectForLoad = options?.projectOverride === undefined ? selectedProject : options.projectOverride
    const setForLoad = options?.setOverride === undefined ? selectedSet : options.setOverride
    const silent = options?.silent ?? false
    if (!silent) setLoading(true)
    try {
      const [projectData, setData, assistantData, conversationData] = await Promise.all([
        chatApi.getProjects(),
        chatApi.getSets(),
        chatApi.getAssistants(),
        chatApi.getConversations(0, 80),
      ])
      setProjects(projectData)
      setSets(setData)
      setAssistants(assistantData)
      setConversations(conversationData)
      if (projectForLoad && !projectData.some((project) => project.id === projectForLoad.id)) {
        onSelectProject(null)
      }
      if (setForLoad && !setData.some((set) => set.id === setForLoad.id)) {
        onSelectSet(null)
      }
    } catch (err) {
      console.error('Failed to load chat sidebar data:', err)
    } finally {
      if (!silent) setLoading(false)
    }
  }, [onSelectProject, onSelectSet, selectedProject, selectedSet])

  useEffect(() => {
    // 側欄資料與 selectedProject 無關（loadSidebarData 始終拉全部專案+對話，僅用 selectedProject
    // 判斷專案是否被刪）。切專案時拉到的是相同資料，不該進 loading 態白閃一下；首次載入非靜默
    // 顯 loading，之後（含跨專案切換）一律靜默後臺重新整理，消除切換對話時的側欄閃爍。
    void loadSidebarData({ silent: sidebarLoadedRef.current })
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
    if (!window.confirm('確定刪除此對話？此操作無法撤銷。')) return
    // B3：刪"generating"會話先強制清父元件 in-flight/樂觀狀態，
    // 讓樂觀合併（visibleConversations）不再保留它。
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
      // 無論後端刪除成功或拋錯，都本機剔除該 id 並重新整理側欄，確保 ghost 立即消失。
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

  const handleMoveConversationToSet = async (id: string, setId: string | undefined) => {
    try {
      const conversation = await chatApi.updateConversation(id, { setId: setId ?? null })
      const conversationSetId = conversation.set_id ?? conversation.setId ?? null
      // 當前開啟的對話被移出當前選中集，則從主檢視移除（與專案邏輯一致）。
      if (currentConversationId === id && selectedSet && conversationSetId !== selectedSet.id) {
        onConversationDeleted?.(id)
      }
      await loadSidebarData({ silent: true })
    } catch (err) {
      console.error('Failed to move conversation to set:', err)
    }
  }

  const openCreateSetDialog = () => {
    setDialogSet(null)
    setSetDialogError('')
  }

  const openSetMenu = (setId: string, button: HTMLButtonElement) => {
    const rect = button.getBoundingClientRect()
    setSetMenuState({ setId, anchor: { left: rect.right - 180, top: rect.bottom + 4 } })
  }

  const handleSaveSet = async (
    name: string,
    systemPrompt: string,
    defaultAssistantId: string | null,
  ) => {
    setSetDialogSaving(true)
    setSetDialogError('')
    try {
      const set = dialogSet
        ? await chatApi.updateSet(dialogSet.id, { name, systemPrompt, defaultAssistantId })
        : await chatApi.createSet(name, systemPrompt, defaultAssistantId)
      onSelectSet(set)
      await loadSidebarData({ silent: true, setOverride: set })
      setDialogSet(undefined)
    } catch (err) {
      setSetDialogError(typeof err === 'string' ? err : (err as Error).message || '集儲存失敗')
    } finally {
      setSetDialogSaving(false)
    }
  }

  const handleDeleteSet = async (set: ChatSet) => {
    if (!window.confirm(`確定刪除集「${set.name}」？集內的對話會移出集，不會被刪除。`)) {
      return
    }
    try {
      await chatApi.deleteSet(set.id)
      if (selectedSet?.id === set.id) {
        onSelectSet(null)
        if (currentConversationId) onConversationDeleted?.(currentConversationId)
      }
      await loadSidebarData({ silent: true })
    } catch (err) {
      console.error('Failed to delete set:', err)
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
      setProjectError(typeof err === 'string' ? err : (err as Error).message || '專案儲存失敗')
    } finally {
      setProjectSaving(false)
    }
  }

  const handleOpenProjectFolder = async (project: ChatProject) => {
    try {
      await chatApi.openProjectFolder(project.id)
    } catch (err) {
      window.alert(typeof err === 'string' ? err : (err as Error).message || '開啟專案資料夾失敗')
    }
  }

  const handleDeleteProject = async (project: ChatProject) => {
    if (!window.confirm(`確定刪除專案「${project.name}」？專案內的聊天會移出專案，不會被刪除。`)) {
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
    const scope = selectedProject ? `專案「${selectedProject.name}」中的` : '全部'
    if (!window.confirm(`確定刪除${scope} ${targetConversations.length} 個對話？此操作無法撤銷。`)) return
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

  const setConversationMap = useMemo(() => {
    const map = new Map<string, ConversationListItem[]>()
    sets.forEach((set) => {
      map.set(
        set.id,
        visibleConversations.filter(
          (conversation) => (conversation.set_id ?? conversation.setId) === set.id,
        ),
      )
    })
    return map
  }, [sets, visibleConversations])

  // 「最近」標籤：跨集/專案的全部對話，置頂在前、再按更新時間倒序。
  const recentConversations = useMemo(
    () =>
      [...visibleConversations].sort((a, b) => {
        const pin = Number(Boolean(b.pinned)) - Number(Boolean(a.pinned))
        if (pin !== 0) return pin
        return (b.updated_at ?? 0) - (a.updated_at ?? 0)
      }),
    [visibleConversations],
  )

  // 查詢變化時去後端全量索引搜（debounce 180ms）。覆蓋掉出"最近 80"的老對話。
  useEffect(() => {
    if (!searchOpen || !normalizedSearchQuery) {
      setFullSearchResults([])
      return
    }
    let cancelled = false
    const handle = setTimeout(() => {
      void chatApi
        .searchConversations(searchQuery, 30)
        .then((items) => {
          if (!cancelled) setFullSearchResults(items)
        })
        .catch(() => {
          if (!cancelled) setFullSearchResults([])
        })
    }, 180)
    return () => {
      cancelled = true
      clearTimeout(handle)
    }
  }, [searchOpen, normalizedSearchQuery, searchQuery])

  const searchResults = useMemo(() => {
    if (!normalizedSearchQuery) {
      return visibleConversations.slice(0, 9)
    }
    // Tauri：後端全量結果優先；為空/mock 時回退到已載入列表的客戶端過濾（也覆蓋後端結果到達前的瞬間）。
    if (fullSearchResults.length > 0) {
      return fullSearchResults
    }
    return visibleConversations
      .filter((conversation) => {
        const project = findConversationProject(conversation, projects)
        return (
          conversationMatchesSearch(conversation, normalizedSearchQuery) ||
          (project ? projectMatchesSearch(project, normalizedSearchQuery) : false) ||
          (conversation.folder ?? '').toLowerCase().includes(normalizedSearchQuery)
        )
      })
      .slice(0, 9)
  }, [normalizedSearchQuery, projects, visibleConversations, fullSearchResults])

  const clearableConversationCount = selectedProject
    ? conversations.filter((conv) => conversationBelongsToProject(conv, selectedProject)).length
    : conversations.length

  const allVisibleProjectsCollapsed = visibleProjects.length > 0 &&
    visibleProjects.every((project) => collapsedProjectIds.has(project.id))

  const allVisibleSetsCollapsed =
    sets.length > 0 && sets.every((set) => collapsedSetIds.has(set.id))

  const menuSet = setMenuState ? sets.find((set) => set.id === setMenuState.setId) : undefined

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
          label="搜尋"
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

      <div className="flex min-h-0 flex-1 flex-col" data-tauri-drag-region="false">
        {loading ? (
          <div className="space-y-2 px-3 py-3" aria-label="載入中" aria-busy="true">
            {[0, 1, 2, 3, 4, 5].map((i) => (
              <div key={i} className="kv-skeleton h-7 rounded-lg" />
            ))}
          </div>
        ) : (
          <>
            <div className="flex items-center justify-between px-4 pb-1 pt-3">
              <div className="flex items-center gap-1.5 text-[13px] font-semibold">
                {([
                  ['conversations', '最近'],
                  ['sets', '集'],
                  ['projects', '專案'],
                ] as const).flatMap(([tab, label], i) => {
                  const button = (
                    <button
                      key={tab}
                      type="button"
                      onClick={() => setActiveTab(tab)}
                      className={`rounded-md px-1 py-0.5 transition-colors ${
                        activeTab === tab
                          ? 'text-neutral-900 dark:text-neutral-100'
                          : 'text-neutral-400 hover:text-neutral-600 dark:text-neutral-500 dark:hover:text-neutral-300'
                      }`}
                      aria-current={activeTab === tab}
                    >
                      {label}
                    </button>
                  )
                  return i === 0
                    ? [button]
                    : [
                        <span key={`sep-${tab}`} className="text-neutral-300 dark:text-neutral-700">
                          /
                        </span>,
                        button,
                      ]
                })}
              </div>
              <div className="flex shrink-0 items-center gap-1">
                {activeTab === 'conversations' && (
                  <>
                    <button
                      ref={sectionMenuButtonRef}
                      type="button"
                      onClick={openSectionMenu}
                      className={`rounded-md p-0.5 text-neutral-400 transition-colors hover:bg-black/[0.06] hover:text-neutral-600 dark:hover:bg-white/[0.1] dark:hover:text-neutral-200 ${
                        sectionMenuAnchor
                          ? 'bg-black/[0.06] text-neutral-600 dark:bg-white/[0.1] dark:text-neutral-200'
                          : ''
                      }`}
                      aria-label="對話列表操作"
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
                  </>
                )}
                {activeTab === 'sets' && (
                  <>
                    <button
                      type="button"
                      onClick={() => {
                        setCollapsedSetIds((previous) => {
                          const next = new Set(previous)
                          if (allVisibleSetsCollapsed) {
                            sets.forEach((set) => next.delete(set.id))
                          } else {
                            sets.forEach((set) => next.add(set.id))
                          }
                          return next
                        })
                      }}
                      className="rounded-md p-0.5 text-neutral-400 transition-colors hover:bg-black/[0.06] hover:text-neutral-600 dark:hover:bg-white/[0.1] dark:hover:text-neutral-200"
                      title={allVisibleSetsCollapsed ? '展開全部集' : '摺疊全部集'}
                      aria-label={allVisibleSetsCollapsed ? '展開全部集' : '摺疊全部集'}
                    >
                      <MoreHorizontal size={15} />
                    </button>
                    <button
                      type="button"
                      onClick={openCreateSetDialog}
                      className="rounded-md p-0.5 text-neutral-400 transition-colors hover:bg-black/[0.06] hover:text-neutral-600 dark:hover:bg-white/[0.1] dark:hover:text-neutral-200"
                      title="新建集"
                      aria-label="新建集"
                    >
                      <Plus size={15} strokeWidth={2} />
                    </button>
                  </>
                )}
                {activeTab === 'projects' && (
                  <>
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
                      title={allVisibleProjectsCollapsed ? '展開全部專案' : '摺疊全部專案'}
                      aria-label={allVisibleProjectsCollapsed ? '展開全部專案' : '摺疊全部專案'}
                    >
                      <MoreHorizontal size={15} />
                    </button>
                    <button
                      type="button"
                      onClick={openCreateProjectDialog}
                      className="rounded-md p-0.5 text-neutral-400 transition-colors hover:bg-black/[0.06] hover:text-neutral-600 dark:hover:bg-white/[0.1] dark:hover:text-neutral-200"
                      title={`新建專案 (${modLabel}P)`}
                      aria-label="新建專案"
                    >
                      <FolderPlus size={15} strokeWidth={1.75} />
                    </button>
                  </>
                )}
              </div>
            </div>

            <div className="custom-scrollbar min-h-0 flex-1 overflow-y-auto" data-tauri-drag-region="false">
            {activeTab === 'projects' && (
            <section className="group/projects px-3 pb-2 pt-1">
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
                            title={collapsedProject ? `展開 ${project.name}` : `摺疊 ${project.name}`}
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
                            aria-label="專案操作"
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
                          sets={sets}
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
                          onMoveConversationToSet={handleMoveConversationToSet}
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
                          {expanded ? '收起' : '展開顯示'}
                        </button>
                      )}
                      </div>
                    )
                  })}
                </div>
            </section>
            )}

            {activeTab === 'sets' && (
            <section className="group/sets px-3 pb-2 pt-1">
                <div className="mt-1.5 space-y-1">
                  {sets.length === 0 ? (
                    <button
                      type="button"
                      onClick={openCreateSetDialog}
                      className="flex w-full items-center gap-1.5 rounded-lg px-2 py-1 text-left text-[13px] text-neutral-400 transition-colors hover:bg-black/[0.035] hover:text-neutral-600 dark:text-neutral-500 dark:hover:bg-white/[0.06] dark:hover:text-neutral-300"
                    >
                      <Plus size={14} strokeWidth={2} className="shrink-0" />
                      新建一個集（系統提示詞 + 預設助手）
                    </button>
                  ) : (
                    sets.map((set, index) => {
                      const active = selectedSet?.id === set.id
                      const setConversations = setConversationMap.get(set.id) ?? []
                      const collapsedSet = collapsedSetIds.has(set.id)
                      const expanded = expandedSetConversationIds.has(set.id)
                      const previewConversations = expanded
                        ? setConversations
                        : setConversations.slice(0, PROJECT_PREVIEW_LIMIT)
                      return (
                        <div key={set.id}>
                          <div
                            className={`chat-motion-row group flex min-w-0 items-center rounded-lg ${
                              active
                                ? 'bg-black/[0.04] dark:bg-white/[0.08]'
                                : 'hover:bg-black/[0.035] dark:hover:bg-white/[0.06]'
                            }`}
                            style={{ ['--chat-motion-delay' as string]: `${Math.min(index, 12) * 18}ms` }}
                          >
                            <button
                              type="button"
                              onClick={() => {
                                setCollapsedSetIds((previous) => {
                                  const next = new Set(previous)
                                  if (next.has(set.id)) next.delete(set.id)
                                  else next.add(set.id)
                                  return next
                                })
                              }}
                              className={`flex min-w-0 flex-1 items-center gap-1.5 px-2 py-1 text-left text-[13px] ${
                                active
                                  ? 'font-semibold text-neutral-900 dark:text-neutral-100'
                                  : 'font-medium text-neutral-600 dark:text-neutral-300'
                              }`}
                              title={collapsedSet ? `展開 ${set.name}` : `摺疊 ${set.name}`}
                              aria-expanded={!collapsedSet}
                            >
                              <ChevronRight
                                size={13}
                                strokeWidth={2}
                                className={`shrink-0 text-neutral-400 transition-transform dark:text-neutral-500 ${
                                  collapsedSet ? '' : 'rotate-90'
                                }`}
                              />
                              <Layers
                                size={15}
                                strokeWidth={1.75}
                                className="shrink-0 text-neutral-500 dark:text-neutral-400"
                              />
                              <span className="min-w-0 truncate">{set.name}</span>
                            </button>
                            <button
                              type="button"
                              onClick={(e) => {
                                e.stopPropagation()
                                openSetMenu(set.id, e.currentTarget)
                              }}
                              className={`shrink-0 rounded-md p-0.5 text-neutral-400 transition-opacity hover:bg-black/[0.06] hover:text-neutral-600 dark:hover:bg-white/[0.1] dark:hover:text-neutral-200 ${
                                setMenuState?.setId === set.id ? 'opacity-100' : 'opacity-0 group-hover:opacity-100'
                              }`}
                              aria-label="集操作"
                            >
                              <MoreHorizontal size={15} />
                            </button>
                            <button
                              type="button"
                              onClick={(e) => {
                                e.stopPropagation()
                                setCollapsedSetIds((previous) => {
                                  const next = new Set(previous)
                                  next.delete(set.id)
                                  return next
                                })
                                onSelectSet(set)
                              }}
                              className="mr-1 shrink-0 rounded-md p-0.5 text-neutral-400 opacity-0 transition-opacity hover:bg-black/[0.06] hover:text-neutral-600 group-hover:opacity-100 dark:hover:bg-white/[0.1] dark:hover:text-neutral-200"
                              aria-label="在此集新建聊天"
                              title="在此集新建聊天"
                            >
                              <SquarePen size={15} strokeWidth={1.75} />
                            </button>
                          </div>

                          {!collapsedSet && previewConversations.length > 0 && (
                            <ConversationList
                              conversations={previewConversations}
                              currentConversationId={currentConversationId}
                              generatingConversationIds={generatingConversationIds}
                              projects={projects}
                              sets={sets}
                              compact
                              indent
                              showAssistantName={false}
                              onSelectConversation={(id) => {
                                if (selectedSet?.id !== set.id) onSelectSet(set)
                                onSelectConversation(id)
                              }}
                              onRenameConversation={handleRenameConversation}
                              onDeleteConversation={handleDeleteConversation}
                              onMoveConversationToProject={handleMoveConversationToProject}
                              onMoveConversationToSet={handleMoveConversationToSet}
                            />
                          )}

                          {!collapsedSet && setConversations.length > PROJECT_PREVIEW_LIMIT && (
                            <button
                              type="button"
                              onClick={() => {
                                setExpandedSetConversationIds((previous) => {
                                  const next = new Set(previous)
                                  if (next.has(set.id)) next.delete(set.id)
                                  else next.add(set.id)
                                  return next
                                })
                              }}
                              className="ml-8 rounded-md px-2.5 py-0.5 text-left text-[13px] font-medium text-neutral-400 transition-colors hover:bg-black/[0.035] hover:text-neutral-600 dark:text-neutral-500 dark:hover:bg-white/[0.06] dark:hover:text-neutral-300"
                            >
                              {expanded ? '收起' : '展開顯示'}
                            </button>
                          )}
                        </div>
                      )
                    })
                  )}
                </div>
            </section>
            )}

            {activeTab === 'conversations' && (
            <section className="group/conversations px-3 pb-5 pt-1">
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

              {recentConversations.length > 0 ? (
                <div className="mt-1.5">
                    <ConversationList
                      conversations={recentConversations}
                      currentConversationId={currentConversationId}
                      generatingConversationIds={generatingConversationIds}
                      projects={projects}
                      sets={sets}
                      compact
                      showAssistantName={false}
                      showFolderLabel
                      onSelectConversation={(id) => {
                        if (selectedProject) onSelectProject(null)
                        if (selectedSet) onSelectSet(null)
                        onSelectConversation(id)
                      }}
                      onRenameConversation={handleRenameConversation}
                      onDeleteConversation={handleDeleteConversation}
                      onMoveConversationToProject={handleMoveConversationToProject}
                      onMoveConversationToSet={handleMoveConversationToSet}
                    />
                </div>
              ) : null}
            </section>
            )}
            </div>
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

      {setMenuState && menuSet && (
        <SetContextMenu
          anchor={setMenuState.anchor}
          onRename={() => {
            setDialogSet(menuSet)
            setSetDialogError('')
          }}
          onDelete={() => void handleDeleteSet(menuSet)}
          onClose={() => setSetMenuState(null)}
        />
      )}

      {dialogSet !== undefined && (
        <SetDialog
          set={dialogSet}
          assistants={assistants}
          saving={setDialogSaving}
          error={setDialogError}
          onSave={(name, systemPrompt, defaultAssistantId) =>
            void handleSaveSet(name, systemPrompt, defaultAssistantId)
          }
          onClose={() => setDialogSet(undefined)}
        />
      )}
    </aside>

    {searchOpen && (
      <SearchDialog
        query={searchQuery}
        results={searchResults}
        currentConversationId={currentConversationId}
        projects={projects}
        sets={sets}
        onQueryChange={setSearchQuery}
        onSelectConversation={handleSelectSearchConversation}
        onClose={closeSearch}
      />
    )}
    </>
  )
})
