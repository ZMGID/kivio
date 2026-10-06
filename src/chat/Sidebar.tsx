import { memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'
import { createPortal } from 'react-dom'
import { save } from '@tauri-apps/plugin-dialog'
import {
  ChevronRight,
  Folder,
  FolderPlus,
  Layers,
  MoreHorizontal,
  Plus,
  Search,
  Settings,
  SquarePen,
} from 'lucide-react'
import type { ChatAssistant, ChatProject, ChatSet, ConversationListItem, ConversationSearchHit } from './types'
import { HighlightText } from './searchHighlight'
import { AgentIcon, ComposeIcon, ExtensionsIcon, KnowledgeIcon, MediaIcon, NotesIcon, PluginIcon, PortfolioIcon, TasksIcon, SearchNavIcon } from '../settings/public/icons'
import { ConversationList } from './ConversationList'
import { useScheduledTasks } from './scheduledTasks/useScheduledTasks'
import { ChatSectionMenu } from './ChatSectionMenu'
import { ProjectContextMenu } from './ProjectContextMenu'
import { ProjectDialog } from './ProjectDialog'
import { CliImportDialog } from './CliImportDialog'
import { SetContextMenu } from './SetContextMenu'
import { SetDialog } from './SetDialog'
import { SidebarAccountMenu } from './SidebarAccountMenu'
import { getSettingsCached } from '../api/settingsCache'
import { IconButton } from '../components/Button'
import { Select } from '../settings/public/controls'
import { chatApi } from './api'
import { applyIdOrder, moveIdToIndex } from '../utils/pointerReorder'
import { useInsertionReorder } from '../utils/insertionReorder'
import { applyConversationPins, withPinAt, type ConversationPin } from './conversationPins'
import { ChatTitlebarActions } from './ChatTitlebarActions'
import { chatTitlebarMacInsetClass, isMac, usesNativeTitlebar } from './platform'
import { clampSidebarWidth, SIDEBAR_DEFAULT_WIDTH, getRememberedSidebarView, rememberSidebarView } from './persistence'
import { useChatPerfRenderProbe } from './chatPerformanceProbe'
import type { ConversationMenuAnchor } from './ConversationContextMenu'
import type { ChatUserProfile } from './types'
import { UserAvatar } from './UserAvatar'
import { i18n, useT, type I18n, type Lang } from '../components/i18n'
import { conversationMarkdownFilename } from './conversationExport'
import { displayConversationTitle, isPlaceholderTitle, isProvisionalTitle } from './conversationTitle'
import { SwapTitle } from './SwapTitle'
import { ProjectIcon } from './ProjectIcon'
import { alertDialog, confirmDialog } from '../components/dialogQueue'

function resolveChatUserProfile(
  chat?: { userDisplayName?: string; userAvatar?: string } | null,
): ChatUserProfile {
  return {
    displayName: chat?.userDisplayName?.trim() || '',
    avatarUrl: chat?.userAvatar?.trim() || '',
  }
}

const modLabel = isMac ? '⌘' : 'Ctrl'

export type ExtensionsNavItem = import('./chatRoutes').ChatExtensionsNavItem

/**
 * 点击会话时要同步切换的侧栏导航上下文。
 *
 * 必须和会话选择合并成一次动作：若先调用 onSelectProject/onSelectSet，它们会先把路由
 * 写成空会话 #chat，hashchange 随后会取消正在加载的目标会话，最终停在新对话页。
 */
export interface ConversationSelectionScope {
  project: ChatProject | null
  set: ChatSet | null
}

const extensionSubItems: Array<{
  id: ExtensionsNavItem
  label: (t: I18n) => string
  icon: (props: { size?: number; strokeWidth?: number; className?: string }) => React.JSX.Element
}> = [
  { id: 'assistants', label: (t) => t.chatNavAssistants, icon: AgentIcon },
  { id: 'knowledge', label: (t) => t.chatNavKnowledge, icon: KnowledgeIcon },
  { id: 'media', label: (t) => t.chatNavMedia, icon: MediaIcon },
  { id: 'notes', label: (t) => t.chatNavNotes, icon: NotesIcon },
]

const PROJECT_PREVIEW_LIMIT = 5

/** 滚动期间给容器挂 .is-scrolling（停止 1s 后摘除），配合 .kv-scrollbar-autohide
 *  实现"滚动时才出现"的滚动条。刻意不用 :hover 驱动——hover 的出现/消失时机不可预期。 */
function useScrollingFlag(ref: React.RefObject<HTMLElement | null>) {
  useEffect(() => {
    const el = ref.current
    if (!el) return
    let timer: number | undefined
    const onScroll = () => {
      el.classList.add('is-scrolling')
      window.clearTimeout(timer)
      timer = window.setTimeout(() => el.classList.remove('is-scrolling'), 1000)
    }
    el.addEventListener('scroll', onScroll, { passive: true })
    return () => {
      el.removeEventListener('scroll', onScroll)
      window.clearTimeout(timer)
      el.classList.remove('is-scrolling')
    }
  }, [ref])
}

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

/** 置顶会话排前，同组内保持原有相对顺序（时间序 / 拖拽钉子）。 */
function partitionPinnedFirst(
  conversations: ConversationListItem[],
): ConversationListItem[] {
  const pinned: ConversationListItem[] = []
  const rest: ConversationListItem[] = []
  for (const item of conversations) {
    if (item.pinned) pinned.push(item)
    else rest.push(item)
  }
  return pinned.length === 0 ? conversations : [...pinned, ...rest]
}

function conversationMatchesSearch(conversation: ConversationListItem, query: string): boolean {
  if (!query) return true
  return (
    displayConversationTitle(conversation.title, conversation.preview).toLowerCase().includes(query) ||
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

/**
 * 生成中乐观行会盖住真实行。置顶是用户手势，必须以真实列表为准，
 * 否则要点到 run 结束、乐观行卸掉才看得见。
 */
function overlayOptimisticConversation(
  optimistic: ConversationListItem,
  real: ConversationListItem | undefined,
): ConversationListItem {
  if (!real || Boolean(real.pinned) === Boolean(optimistic.pinned)) return optimistic
  return { ...optimistic, pinned: Boolean(real.pinned) }
}

function applyPinOverrides(
  items: ConversationListItem[],
  overrides: Record<string, boolean>,
): ConversationListItem[] {
  if (Object.keys(overrides).length === 0) return items
  return items.map((item) => {
    const pinned = overrides[item.id]
    if (pinned === undefined || Boolean(item.pinned) === pinned) return item
    return { ...item, pinned }
  })
}

export interface SidebarProps {
  lang: Lang
  currentConversationId?: string
  generatingConversationIds?: ReadonlySet<string>
  optimisticConversations?: ConversationListItem[]
  /**
   * 当前打开的对话。后端 list 排除归档，所以归档后仍开着的对话不会出现在
   * `conversations` 里；生成中的乐观行能看见它，结束后就被剪掉。
   */
  openConversation?: ConversationListItem | null
  selectedProject?: ChatProject | null
  onSelectProject: (project: ChatProject | null) => void
  selectedSet?: ChatSet | null
  onSelectSet: (set: ChatSet | null) => void
  onSelectConversation: (
    id: string,
    conversation?: ConversationSearchHit | ConversationListItem,
    scope?: ConversationSelectionScope,
  ) => void
  onNewConversation: () => void
  onOpenInPopout?: (conversationId: string) => void | Promise<void>
  onConversationDeleted?: (id: string) => void
  onForceDropConversation?: (id: string) => void
  /** 真实会话列表 refetch 落地后回调（父组件据此剪枝乐观条目，见 visibleConversations 注释）。 */
  onConversationsLoaded?: () => void
  onOpenSettings: () => void
  onOpenExtensionsItem: (item: ExtensionsNavItem) => void
  onSelectLang: (lang: Lang) => void
  onOpenUsage: () => void
  settingsActive?: boolean
  extensionsActive?: ExtensionsNavItem | null
  collapsed: boolean
  onToggleCollapsed: () => void
  /** 展开态宽度。拖拽过程只写 CSS 变量，松手才回传。 */
  width?: number
  onWidthChange?: (width: number) => void
  refreshKey: number
  profileRefreshKey?: number
  searchOpen: boolean
  onSearchOpenChange: (open: boolean) => void
}

function SidebarUserFooter({
  profile,
  lang,
  settingsActive,
  onOpenSettings,
  onSelectLang,
  onOpenUsage,
}: {
  profile: ChatUserProfile
  lang: Lang
  settingsActive: boolean
  onOpenSettings: () => void
  onSelectLang: (lang: Lang) => void
  onOpenUsage: () => void
}) {
  const [menuRect, setMenuRect] = useState<{ left: number; top: number; width: number } | null>(null)
  const rowRef = useRef<HTMLDivElement>(null)
  const t = i18n[lang]

  const toggleMenu = () => {
    if (menuRect) {
      setMenuRect(null)
      return
    }
    const rect = rowRef.current?.getBoundingClientRect()
    if (!rect) return
    setMenuRect({ left: rect.left, top: rect.top, width: rect.width })
  }

  return (
    <div
      className="shrink-0 border-t border-neutral-200/60 p-1.5"
      data-tauri-drag-region="false"
    >
      <div
        ref={rowRef}
        className={`flex w-full items-center gap-1 rounded-lg px-1.5 py-1 transition-colors ${
          menuRect || settingsActive
            ? 'bg-neutral-900/[0.06]'
            : 'hover:bg-neutral-900/[0.04]'
        }`}
      >
        <button
          type="button"
          onClick={toggleMenu}
          className="flex min-w-0 flex-1 items-center gap-2 text-left"
          aria-haspopup="menu"
          aria-expanded={menuRect !== null}
        >
          <UserAvatar profile={profile} size={22} />
          <span
            className="min-w-0 flex-1 truncate text-[12.5px] text-neutral-700"
            title={profile.displayName || undefined}
          >
            {profile.displayName || 'Kivio'}
          </span>
        </button>
        <IconButton
          size="xs"
          label={`${t.settings} (${isMac ? '⌘,' : 'Ctrl+,'})`}
          onClick={() => {
            setMenuRect(null)
            onOpenSettings()
          }}
        >
          <Settings strokeWidth={1.75} />
        </IconButton>
      </div>

      {menuRect && (
        <SidebarAccountMenu
          triggerRect={menuRect}
          lang={lang}
          onSelectLang={onSelectLang}
          onOpenUsage={onOpenUsage}
          onClose={() => setMenuRect(null)}
        />
      )}
    </div>
  )
}

interface NavRowProps {
  icon: React.ReactNode
  label: string
  onClick?: () => void
  disabled?: boolean
  active?: boolean
}

function NavRow({ icon, label, onClick, disabled, active }: NavRowProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={`kv-nav-motion group flex w-full items-center gap-2.5 rounded-lg px-3 py-1.5 text-left text-[13px] transition-colors disabled:cursor-default disabled:opacity-40 ${
        active
          ? 'bg-neutral-900/[0.06] font-medium text-neutral-900'
          : 'text-neutral-800 hover:bg-neutral-900/[0.04]'
      }`}
    >
      <span
        className={`flex h-5 w-5 shrink-0 items-center justify-center text-neutral-600 transition duration-300 ease-out group-hover:text-neutral-800 dark:text-neutral-400`}
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
  const t = useT()
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
        className={`kv-nav-motion group flex w-full items-center gap-2.5 rounded-lg px-3 py-1.5 text-left text-[13px] font-medium transition-colors ${
          highlighted
            ? 'bg-neutral-900/[0.06] text-neutral-900'
            : 'text-neutral-800 hover:bg-neutral-900/[0.04]'
        }`}
        aria-expanded={expanded}
      >
        <span className="flex h-5 w-5 shrink-0 items-center justify-center text-neutral-600 transition duration-300 ease-out group-hover:text-neutral-800 dark:text-neutral-400">
          <ExtensionsIcon size={18} strokeWidth={1.75} />
        </span>
        <span className="min-w-0 flex-1 truncate">{t.chatNavExtensions}</span>
        <ChevronRight
          size={14}
          strokeWidth={2}
          className={`shrink-0 text-neutral-400 transition-transform duration-[var(--kv-dur-fast)] ease-[var(--kv-ease-standard)] dark:text-neutral-500 ${
            expanded ? 'rotate-90' : ''
          }`}
        />
      </button>
      {expanded && (
        <div className="ml-[12px] mt-0.5 grid grid-cols-2 gap-x-1">
          {extensionSubItems.map((item) => {
            const active = activeItem === item.id
            const Icon = item.icon
            return (
              <button
                key={item.id}
                type="button"
                onClick={() => onSelectItem(item.id)}
                className={`flex items-center gap-2 rounded-md py-1.5 pl-2 pr-1 text-left text-[13px] transition-colors ${
                  active
                    ? 'font-medium text-neutral-900'
                    : 'text-neutral-700 hover:bg-neutral-900/[0.04] hover:text-neutral-900'
                }`}
              >
                <span className={`flex h-4 w-4 shrink-0 items-center justify-center ${
                  active ? 'text-neutral-700' : 'text-neutral-400 dark:text-neutral-500'
                }`}>
                  <Icon size={15} strokeWidth={1.75} />
                </span>
                <span className="min-w-0 flex-1 truncate">{item.label(t)}</span>
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
  generatingConversationIds = new Set(),
  projects,
  sets,
  onQueryChange,
  onSelectConversation,
  onClose,
}: {
  query: string
  results: ConversationSearchHit[]
  currentConversationId?: string
  generatingConversationIds?: ReadonlySet<string>
  projects: ChatProject[]
  sets: ChatSet[]
  onQueryChange: (query: string) => void
  onSelectConversation: (conversation: ConversationSearchHit) => void
  onClose: () => void
}) {
  const t = useT()
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
        className="chat-motion-popover flex max-h-[62vh] w-full max-w-[560px] flex-col overflow-hidden rounded-xl border border-neutral-200 bg-neutral-50 shadow-2xl shadow-black/25"
        role="dialog"
        aria-modal="true"
        aria-label={t.chatSearchConversations}
      >
        <div className="flex items-center gap-2 border-b border-neutral-200/80 px-3 py-2">
          <Search size={15} strokeWidth={1.75} className="shrink-0 text-neutral-400" />
          <input
            ref={inputRef}
            type="text"
            value={query}
            autoCapitalize="off"
            autoCorrect="off"
            autoComplete="off"
            spellCheck={false}
            onChange={(event) => onQueryChange(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Enter' && results[0]) {
                if (event.nativeEvent.isComposing || event.keyCode === 229) return
                event.preventDefault()
                onSelectConversation(results[0])
              }
            }}
            placeholder={t.chatSearchConversations}
            className="min-w-0 flex-1 bg-transparent text-[14px] font-medium text-neutral-900 outline-none placeholder:text-neutral-400 dark:placeholder:text-neutral-500"
          />
        </div>

        <div className="px-3 pb-1 pt-2 text-[11px] font-semibold uppercase tracking-wide text-neutral-400 dark:text-neutral-500">
          {normalizedQuery ? t.chatSearchResults : t.chatRecentConversations}
        </div>

        <div className="custom-scrollbar min-h-0 overflow-y-auto px-1.5 pb-1.5">
          {results.length > 0 ? (
            results.map((conversation, index) => {
              const active = conversation.id === currentConversationId
              const listedTitle =
                displayConversationTitle(conversation.title, conversation.preview)
                || t.chatLibUntitled
              const projectLabel = conversationProjectLabel(conversation, projects)
              const setId = conversation.set_id ?? conversation.setId ?? null
              const setLabel = setId ? sets.find((s) => s.id === setId)?.name ?? '' : ''
              const snippet = (conversation.match_snippet ?? conversation.matchSnippet ?? '').trim()
              const showSnippet = Boolean(
                normalizedQuery
                && snippet
                && snippet !== listedTitle
                && !isPlaceholderTitle(snippet),
              )
              return (
                <button
                  key={conversation.id}
                  type="button"
                  onClick={() => onSelectConversation(conversation)}
                  style={{
                    ['--chat-motion-delay' as string]: `${Math.min(index, 12) * 18}ms`,
                  }}
                  className={`chat-motion-row group/search-result flex w-full min-w-0 items-start gap-2 rounded-md px-2.5 py-1.5 text-left transition-colors ${
                    active
                      ? 'bg-neutral-900/[0.07]'
                      : 'hover:bg-neutral-900/[0.04]'
                  }`}
                >
                  <div className="min-w-0 flex-1">
                    <div className="flex min-w-0 items-center gap-2">
                      {normalizedQuery ? (
                        <HighlightText
                          text={listedTitle}
                          query={normalizedQuery}
                          className={`min-w-0 flex-1 truncate text-[13px] font-medium ${
                            active
                              ? 'text-neutral-950'
                              : 'text-neutral-800'
                          }${
                            generatingConversationIds.has(conversation.id)
                            && isProvisionalTitle(listedTitle, conversation.preview)
                              ? ' kv-title-provisional'
                              : ''
                          }`}
                        />
                      ) : (
                        <SwapTitle
                          text={listedTitle}
                          title={listedTitle}
                          className={`min-w-0 flex-1 truncate text-[13px] font-medium ${
                            active
                              ? 'text-neutral-950'
                              : 'text-neutral-800'
                          }${
                            generatingConversationIds.has(conversation.id)
                            && isProvisionalTitle(listedTitle, conversation.preview)
                              ? ' kv-title-provisional'
                              : ''
                          }`}
                        />
                      )}
                      {setLabel && (
                        <span className="max-w-[100px] shrink-0 truncate text-[12px] text-neutral-400 dark:text-neutral-500">
                          {t.chatSetPrefix} · {setLabel}
                        </span>
                      )}
                      {!setLabel && projectLabel && (
                        <span className="max-w-[100px] shrink-0 truncate text-[12px] text-neutral-400 dark:text-neutral-500">
                          {projectLabel}
                        </span>
                      )}
                    </div>
                    {showSnippet && (
                      <p className="mt-0.5 line-clamp-2 text-[12px] leading-snug text-neutral-500 dark:text-neutral-400">
                        <HighlightText text={snippet} query={normalizedQuery} />
                      </p>
                    )}
                  </div>
                </button>
              )
            })
          ) : (
            <div className="px-3 py-6 text-center text-[13px] text-neutral-400 dark:text-neutral-500">
              {t.chatNoMatchingConversations}
            </div>
          )}
        </div>
      </div>
    </div>,
    document.body,
  )
}

function applySidebarWidthCss(aside: HTMLElement | null, nextWidth: number) {
  const px = `${nextWidth}px`
  aside?.style.setProperty('--chat-sidebar-width', px)
  const shell = aside?.closest('.chat-window-shell')
  if (shell instanceof HTMLElement) {
    shell.style.setProperty('--chat-sidebar-width', px)
  }
}

function setSidebarResizing(aside: HTMLElement | null, resizing: boolean) {
  const shell = aside?.closest('.chat-window-shell')
  if (shell instanceof HTMLElement) {
    shell.classList.toggle('is-sidebar-resizing', resizing)
  }
}

export const Sidebar = memo(function Sidebar({
  lang,
  currentConversationId,
  generatingConversationIds = new Set(),
  optimisticConversations = [],
  openConversation = null,
  selectedProject = null,
  onSelectProject,
  selectedSet = null,
  onSelectSet,
  onSelectConversation,
  onNewConversation,
  onOpenInPopout,
  onConversationDeleted,
  onForceDropConversation,
  onConversationsLoaded,
  onOpenSettings,
  onOpenExtensionsItem,
  onSelectLang,
  onOpenUsage,
  settingsActive = false,
  extensionsActive = null,
  collapsed,
  onToggleCollapsed,
  width = SIDEBAR_DEFAULT_WIDTH,
  onWidthChange,
  refreshKey,
  profileRefreshKey = 0,
  searchOpen,
  onSearchOpenChange,
}: SidebarProps) {
  const t = i18n[lang]
  const asideRef = useRef<HTMLElement>(null)
  const dragStateRef = useRef<{ startX: number; startWidth: number; width: number; raf: number } | null>(null)
  // 折叠后侧栏仍挂载（用于滑出动画），用 inert 让其退出 tab 序 / 不可点击 / 不进 a11y 树。
  // useLayoutEffect：在绘制前与 JSX 里的 aria-hidden 原子地一起生效，避免短暂可聚焦窗口。
  useLayoutEffect(() => {
    const el = asideRef.current
    if (el) el.inert = collapsed
  }, [collapsed])
  useLayoutEffect(() => {
    applySidebarWidthCss(asideRef.current, width)
  }, [width])
  const handleResizeStart = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      if (event.button !== 0) return
      event.preventDefault()
      event.stopPropagation()
      const aside = asideRef.current
      const measured = aside?.getBoundingClientRect().width ?? 0
      const startWidth = measured > 0 ? measured : width
      dragStateRef.current = { startX: event.clientX, startWidth, width: startWidth, raf: 0 }
      setSidebarResizing(aside, true)

      const onMove = (moveEvent: PointerEvent) => {
        const state = dragStateRef.current
        if (!state) return
        const nextWidth = clampSidebarWidth(state.startWidth + (moveEvent.clientX - state.startX), window.innerWidth)
        state.width = nextWidth
        if (!state.raf) {
          state.raf = window.requestAnimationFrame(() => {
            state.raf = 0
            applySidebarWidthCss(asideRef.current, state.width)
          })
        }
      }
      const onUp = () => {
        window.removeEventListener('pointermove', onMove)
        window.removeEventListener('pointerup', onUp)
        const state = dragStateRef.current
        dragStateRef.current = null
        setSidebarResizing(asideRef.current, false)
        if (!state) return
        if (state.raf) window.cancelAnimationFrame(state.raf)
        applySidebarWidthCss(asideRef.current, state.width)
        if (state.width !== Math.round(startWidth)) onWidthChange?.(state.width)
      }
      window.addEventListener('pointermove', onMove)
      window.addEventListener('pointerup', onUp)
    },
    [onWidthChange, width],
  )
  const [conversations, setConversations] = useState<ConversationListItem[]>([])
  const { tasks: scheduledTasks } = useScheduledTasks()
  const scheduledTaskNamesByConversation = useMemo(() => {
    const names = new Map<string, string[]>()
    for (const task of scheduledTasks ?? []) {
      const id = task.conversationId
      if (!task.enabled || task.status !== 'active') continue
      const existing = names.get(id)
      if (existing) existing.push(task.name)
      else names.set(id, [task.name])
    }
    return names
  }, [scheduledTasks])
  const [projects, setProjects] = useState<ChatProject[]>([])
  const [sets, setSets] = useState<ChatSet[]>([])
  // 集/项目里对话的钉住位置：group_id → 钉子表。底座仍是时间序，见 conversationPins.ts。
  const [conversationPins, setConversationPins] = useState<Record<string, ConversationPin[]>>({})
  // 置顶手势的进行中覆盖：生成中乐观行 / 列表 refetch 都不得把刚点的 PIN 冲掉。
  const [pinOverrides, setPinOverrides] = useState<Record<string, boolean>>({})
  /** 归档已乐观摘掉、但 persist/refetch 尚未落地：挡住过期 list 把条目写回来（其余行会跟着上下抽）。 */
  const [suppressedConversationIds, setSuppressedConversationIds] = useState<ReadonlySet<string>>(
    () => new Set(),
  )
  const [titleGeneratingIds, setTitleGeneratingIds] = useState<Set<string>>(() => new Set())
  const [assistants, setAssistants] = useState<ChatAssistant[]>([])
  const [searchQuery, setSearchQuery] = useState('')
  // 后端全量索引搜索结果（覆盖所有对话，不止已加载的前 80）；空查询/非 Tauri 时为空，回退客户端过滤。
  const [fullSearchResults, setFullSearchResults] = useState<ConversationSearchHit[]>([])
  // 侧栏三块改为横排标签页：同一时刻只显示一块（对话/集/项目）。
  const [activeTab, setActiveTab] = useState<'conversations' | 'sets' | 'projects'>('conversations')
  const [sidebarView, setSidebarView] = useState(getRememberedSidebarView)
  const flatView = sidebarView === 'flat'
  const [flatScope, setFlatScope] = useState('all')
  const toggleSidebarView = () => {
    const next = flatView ? 'classic' : 'flat'
    setSidebarView(next)
    rememberSidebarView(next)
  }

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
  const [loading, setLoading] = useState(true)
  const [sectionMenuAnchor, setSectionMenuAnchor] = useState<ConversationMenuAnchor | null>(null)
  const [projectMenuState, setProjectMenuState] = useState<{
    projectId: string
    anchor: ConversationMenuAnchor
  } | null>(null)
  const [dialogProject, setDialogProject] = useState<ChatProject | null | undefined>(undefined)
  const [importProject, setImportProject] = useState<ChatProject | null>(null)
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
  const [userProfile, setUserProfile] = useState(() => resolveChatUserProfile())
  useChatPerfRenderProbe('Sidebar', {
    collapsed,
    settingsActive,
    activeTab,
    conversations: conversations.length,
  })

  useEffect(() => {
    let cancelled = false
    void getSettingsCached().then((settings) => {
      if (!cancelled) setUserProfile(resolveChatUserProfile(settings.chat))
    }).catch((err) => {
      console.error('Failed to load chat user profile:', err)
    })
    return () => {
      cancelled = true
    }
  }, [profileRefreshKey])

  // Selection/callbacks are navigation state, not catalog invalidations. Read the
  // latest committed props when a request finishes without restarting it.
  const sidebarInputsRef = useRef({ selectedProject, selectedSet, onSelectProject, onSelectSet, onConversationsLoaded })
  useLayoutEffect(() => {
    sidebarInputsRef.current = { selectedProject, selectedSet, onSelectProject, onSelectSet, onConversationsLoaded }
  })
  type LoadOptions = { silent?: boolean }
  const sidebarRequestRef = useRef<{ promise: Promise<void>; queued?: LoadOptions } | null>(null)
  const sidebarMountedRef = useRef(false)
  useEffect(() => {
    sidebarMountedRef.current = true
    return () => {
      sidebarMountedRef.current = false
      sidebarRequestRef.current = null
    }
  }, [])

  const loadSidebarData = useCallback((options: LoadOptions = {}): Promise<void> => {
    if (!sidebarMountedRef.current) return Promise.resolve()
    const pending = sidebarRequestRef.current
    if (pending) {
      // Mutations arriving during a read need one fresh snapshot afterwards,
      // not an unbounded number of concurrent IPC requests.
      pending.queued = options
      return pending.promise
    }
    const request: { promise: Promise<void>; queued?: LoadOptions } = { promise: Promise.resolve() }
    sidebarRequestRef.current = request
    const isCurrent = () => sidebarRequestRef.current === request && sidebarMountedRef.current
    request.promise = (async () => {
      let next: LoadOptions | undefined = options
      while (next && isCurrent()) {
        const silent = next.silent ?? false
        request.queued = undefined
        if (!silent) setLoading(true)
        try {
          const results = await Promise.allSettled([
            chatApi.getConversations(0, 80).then(conversationData => {
              if (!isCurrent() || request.queued) return
              setConversations(conversationData)
              // Keep optimistic-row handoff in the same React batch.
              sidebarInputsRef.current.onConversationsLoaded?.()
              setLoading(false)
            }),
            Promise.allSettled([
              chatApi.getProjects(), chatApi.getSets(),
              chatApi.getAssistants(), chatApi.getConversationPins(),
            ]).then(([projectResult, setResult, assistantResult, pinResult]) => {
              if (!isCurrent() || request.queued) return
              // A failed read must not release the batch while its siblings
              // are still running, or retries can pile up behind them.
              if (projectResult.status === 'rejected') throw projectResult.reason
              if (setResult.status === 'rejected') throw setResult.reason
              if (assistantResult.status === 'rejected') throw assistantResult.reason
              if (pinResult.status === 'rejected') throw pinResult.reason
              setProjects(projectResult.value)
              setSets(setResult.value)
              setConversationPins(pinResult.value)
              setAssistants(assistantResult.value)
              // A late response must never clear the group the user has since
              // selected using a closure from a previous navigation.
              const inputs = sidebarInputsRef.current
              if (inputs.selectedProject && !projectResult.value.some(project => project.id === inputs.selectedProject?.id)) {
                inputs.onSelectProject(null)
              }
              if (inputs.selectedSet && !setResult.value.some(set => set.id === inputs.selectedSet?.id)) {
                inputs.onSelectSet(null)
              }
            }),
          ])
          for (const result of results) if (result.status === 'rejected') throw result.reason
        } catch (err) {
          console.error('Failed to load chat sidebar data:', err)
        } finally {
          if (isCurrent()) setLoading(false)
        }
        next = request.queued
      }
    })().finally(() => {
      if (sidebarRequestRef.current === request) sidebarRequestRef.current = null
    })
    return request.promise
  }, [])

  useEffect(() => {
    // Initial state already shows the skeleton. Subsequent invalidations keep
    // the current rows visible, including after a StrictMode effect restart.
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

  const handleRegenerateConversationTitle = async (id: string) => {
    setTitleGeneratingIds((previous) => {
      if (previous.has(id)) return previous
      const next = new Set(previous)
      next.add(id)
      return next
    })
    try {
      const updated = await chatApi.regenerateConversationTitle(id)
      setTitleGeneratingIds((previous) => {
        if (!previous.has(id)) return previous
        const next = new Set(previous)
        next.delete(id)
        return next
      })
      setConversations((items) =>
        items.map((item) => (item.id === id ? { ...item, title: updated.title } : item)),
      )
      await loadSidebarData({ silent: true })
    } catch (err) {
      console.error('Failed to regenerate conversation title:', err)
      void alertDialog(
        t.chatRegenerateTitleFailed + (err instanceof Error ? err.message : String(err)),
      )
    } finally {
      setTitleGeneratingIds((previous) => {
        if (!previous.has(id)) return previous
        const next = new Set(previous)
        next.delete(id)
        return next
      })
    }
  }

  const handleTogglePinConversation = async (id: string, pinned: boolean) => {
    // 乐观更新：侧栏立刻重排，避免等磁盘写回才跳动。
    setPinOverrides((previous) => (previous[id] === pinned ? previous : { ...previous, [id]: pinned }))
    setConversations((items) =>
      items.map((item) => (item.id === id ? { ...item, pinned } : item)),
    )
    try {
      await chatApi.updateConversation(id, { pinned })
      await loadSidebarData({ silent: true })
    } catch (err) {
      console.error('Failed to pin conversation:', err)
      await loadSidebarData({ silent: true })
    } finally {
      setPinOverrides((previous) => {
        if (!(id in previous)) return previous
        const next = { ...previous }
        delete next[id]
        return next
      })
    }
  }

  /** 归档：侧栏「最近」不再显示；只在对话库「归档」书架可见。 */
  const handleArchiveConversation = async (id: string) => {
    setSuppressedConversationIds((prev) => {
      if (prev.has(id)) return prev
      const next = new Set(prev)
      next.add(id)
      return next
    })
    // 1) 立刻从侧栏真实列表摘掉
    setConversations((items) => items.filter((item) => item.id !== id))
    // 2) 清父组件乐观条目 / in-flight，否则 visibleConversations 会因「真实列表没有」又把乐观项并回来
    onForceDropConversation?.(id)
    // 3) 当前打开的就是这条 → 主区清空（与删除一致）
    if (currentConversationId === id) {
      onConversationDeleted?.(id)
    }
    try {
      await chatApi.updateConversation(id, { archived: true })
      await loadSidebarData({ silent: true })
    } catch (err) {
      console.error('Failed to archive conversation:', err)
      await loadSidebarData({ silent: true })
    } finally {
      setSuppressedConversationIds((prev) => {
        if (!prev.has(id)) return prev
        const next = new Set(prev)
        next.delete(id)
        return next
      })
    }
  }

  const handleDeleteConversation = async (id: string) => {
    if (!(await confirmDialog({ message: t.chatDeleteConversationConfirm, confirmLabel: t.dialogDelete, danger: true }))) return
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
      const warnings = await chatApi.deleteConversation(id)
      // 对话本身已删掉，只是副产物没清干净（典型：工作区里还有进程占着目录）。
      // 以前这类情况整个删除会中止、对话又冒回来，现在只提示一句。
      if (warnings.length > 0) {
        void alertDialog(t.chatDeleteConversationPartial + warnings.join('\n'))
      }
    } catch (err) {
      console.error('Failed to delete conversation:', err)
      const message = err instanceof Error ? err.message : String(err)
      void alertDialog(t.chatDeleteConversationFailed + message)
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

  const handleExportConversation = async (id: string, title: string) => {
    try {
      const path = await save({
        defaultPath: conversationMarkdownFilename(title),
        filters: [{ name: 'Markdown', extensions: ['md'] }],
      })
      if (!path) return
      await chatApi.exportConversationMarkdown(id, path, lang)
    } catch (err) {
      const prefix = t.chatExportFailed
      const message = err instanceof Error ? err.message : String(err)
      void alertDialog(`${prefix}${message}`)
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
      // 当前打开的对话被移出当前选中集，则从主视图移除（与项目逻辑一致）。
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
    color: string | null,
  ) => {
    setSetDialogSaving(true)
    setSetDialogError('')
    try {
      const set = dialogSet
        ? await chatApi.updateSet(dialogSet.id, { name, systemPrompt, defaultAssistantId, color })
        : await chatApi.createSet(name, systemPrompt, defaultAssistantId, color)
      onSelectSet(set)
      await loadSidebarData({ silent: true })
      setDialogSet(undefined)
    } catch (err) {
      setSetDialogError(typeof err === 'string' ? err : (err as Error).message || t.chatSetSaveFailed)
    } finally {
      setSetDialogSaving(false)
    }
  }

  const handleDeleteSet = async (set: ChatSet) => {
    if (!(await confirmDialog({ message: t.chatDeleteSetConfirm.replace('{name}', () => set.name), confirmLabel: t.dialogDelete, danger: true }))) {
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
    // toggle：开着就关，关着就按位置打开。
    if (sectionMenuAnchor) {
      setSectionMenuAnchor(null)
      return
    }
    const button = sectionMenuButtonRef.current
    if (!button) return
    const rect = button.getBoundingClientRect()
    // 菜单右对齐到按钮，但窄侧栏里会顶到窗口左缘 → 夹住至少 8px 边距。
    setSectionMenuAnchor({ left: Math.max(8, rect.right - 200), top: rect.bottom + 4 })
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
      await loadSidebarData({ silent: true })
      if (flatView) setFlatScope(`project:${project.id}`)
      setDialogProject(undefined)
    } catch (err) {
      setProjectError(typeof err === 'string' ? err : (err as Error).message || t.chatProjectSaveFailed)
    } finally {
      setProjectSaving(false)
    }
  }

  const handleOpenProjectFolder = async (project: ChatProject) => {
    try {
      await chatApi.openProjectFolder(project.id)
    } catch (err) {
      void alertDialog(typeof err === 'string' ? err : (err as Error).message || t.chatOpenProjectFolderFailed)
    }
  }

  const handleDeleteProject = async (project: ChatProject) => {
    if (!(await confirmDialog({ message: t.chatDeleteProjectConfirm.replace('{name}', () => project.name), confirmLabel: t.dialogDelete, danger: true }))) {
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
    const targetConversations = flatView ? flatConversations : selectedProject
      ? conversations.filter((conv) => conversationBelongsToProject(conv, selectedProject))
      : conversations
    if (targetConversations.length === 0) return
    const confirmText = flatView
      ? t.chatSidebarClearConfirm
      : selectedProject
      ? t.chatDeleteAllInProjectConfirm.replace('{name}', () => selectedProject.name)
      : t.chatDeleteAllConfirm
    if (!(await confirmDialog({ message: confirmText.replace('{count}', String(targetConversations.length)), confirmLabel: t.dialogDelete, danger: true }))) return
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
    // 侧栏永不展示已归档（后端 list 也会滤；这里再兜一层防脏数据/旧索引）
    const active = conversations.filter(
      (item) => !item.archived && !suppressedConversationIds.has(item.id),
    )
    // 当前打开的对话即使已被归档 / 不在最近 80 里，也要留在侧栏，否则用户对着正文找不到入口。
    // 刚从侧栏归档的除外：那条在 suppressed 里，不能又并回来。
    const withOpen = (() => {
      if (!openConversation || suppressedConversationIds.has(openConversation.id)) return active
      if (active.some((item) => item.id === openConversation.id)) return active
      return [{ ...openConversation, archived: false }, ...active]
    })()
    if (optimisticConversations.length === 0) return applyPinOverrides(withOpen, pinOverrides)
    const realById = new Map(withOpen.map((item) => [item.id, item]))
    const visibleOptimisticConversations = optimisticConversations.filter((item) => {
      if (item.archived || suppressedConversationIds.has(item.id)) return false
      const real = realById.get(item.id)
      // 真实列表里没有 → 保留。新会话从创建到首次 refetch 之间只存在于乐观列表；run 刚结束
      // 的那次 commit 里 generating 已清、refetch 还没落地，此刻若按 generating 判丢弃，
      // 行会卸载一帧、refetch 后再以最终标题重建 —— SwapTitle 首挂不播，标题打字机整个失效
      // （这正是「首轮生成完标题打出来」的主场景）。归档/删除的幽灵不再靠这里拦：
      // 删除/归档路径走 onForceDropConversation 即时摘除，其余由 refetch 落地时父组件的
      // onConversationsLoaded 剪枝（只留仍在 generating 的乐观项）兜底。
      if (!real) return true
      if (generatingConversationIds.has(item.id)) return true
      // 真实条目仍是占位标题「新对话」说明它落后于乐观条目（乐观条目持有刚持久化的最新数据）：
      // 首轮完成后、refetch 落地前，真实列表里这条还是旧快照 —— 直接切过去会让标题动效
      // 先倒退成「新对话」再跳成生成标题，且行实例销毁重建导致 SwapTitle 过渡不触发。
      return isPlaceholderTitle(real.title)
    }).map((item) => overlayOptimisticConversation(item, realById.get(item.id)))
    if (visibleOptimisticConversations.length === 0) return applyPinOverrides(withOpen, pinOverrides)
    const optimisticIds = new Set(visibleOptimisticConversations.map((item) => item.id))
    return applyPinOverrides([
      ...visibleOptimisticConversations,
      ...withOpen.filter((item) => !optimisticIds.has(item.id)),
    ], pinOverrides)
  }, [conversations, generatingConversationIds, openConversation, optimisticConversations, pinOverrides, suppressedConversationIds])

  const normalizedSearchQuery = searchQuery.trim().toLowerCase()

  const projectConversationMap = useMemo(() => {
    const map = new Map<string, ConversationListItem[]>()
    projects.forEach((project) => {
      const inProject = visibleConversations.filter((conversation) =>
        conversationBelongsToProject(conversation, project),
      )
      // 手动拖拽顺序之上，置顶会话始终压在分组顶部。
      map.set(
        project.id,
        partitionPinnedFirst(
          applyConversationPins(inProject, conversationPins[project.id] ?? []),
        ),
      )
    })
    return map
  }, [conversationPins, projects, visibleConversations])

  const visibleProjects = projects

  // ── 侧栏分组的手动顺序（docs/adr/0004）。数组顺序就是显示顺序，时间不参与。
  // 插入线式拖拽：只有被拖那行浮起，其余行不动，目标位置画线 —— 所以不要求行高相等，
  // 可展开的分组行直接能拖，不需要「拖拽时先全折叠」。
  const groupScrollRef = useRef<HTMLDivElement>(null)
  // 侧栏会话列表滚动条：滚动时才出现（.kv-scrollbar-autohide + .is-scrolling）
  useScrollingFlag(groupScrollRef)
  const projectIds = useMemo(() => visibleProjects.map((project) => project.id), [visibleProjects])
  const setIds = useMemo(() => sets.map((set) => set.id), [sets])

  const projectDrag = useInsertionReorder({
    listRef: groupScrollRef,
    rowSelector: '.kv-sidebar-group',
    onDrop: (id, toIndex) => {
      const nextIds = moveIdToIndex(projectIds, id, toIndex)
      setProjects((previous) => applyIdOrder(previous, nextIds))
      void chatApi.reorderProjects(nextIds).catch((err) => {
        console.error('Failed to reorder projects:', err)
        void loadSidebarData({ silent: true })
      })
    },
  })

  // 集/项目里的对话：拖到哪一行就钉在那一行，其余仍按更新时间填空位。
  // 一个 hook 实例服务所有分组 —— 取样范围由把手最近的 [data-reorder-scope] 决定，
  // 所以多个分组同时展开时不会互串。「最近」平铺列表不传 reorder，因此不可拖。
  const conversationDrag = useInsertionReorder({
    listRef: groupScrollRef,
    rowSelector: '.kv-conv-row',
    onDrop: (id, toIndex, groupId) => {
      if (!groupId) return
      const nextPins = withPinAt(conversationPins[groupId] ?? [], id, toIndex)
      setConversationPins((previous) => ({ ...previous, [groupId]: nextPins }))
      void chatApi.setConversationPins(groupId, nextPins).catch((err) => {
        console.error('Failed to save conversation pins:', err)
        void loadSidebarData({ silent: true })
      })
    },
  })

  const conversationReorderFor = useCallback(
    (groupId: string) => ({
      scopeId: groupId,
      draggingId: conversationDrag.draggingId,
      startDrag: conversationDrag.startDrag,
    }),
    [conversationDrag.draggingId, conversationDrag.startDrag],
  )

  const setDrag = useInsertionReorder({
    listRef: groupScrollRef,
    rowSelector: '.kv-sidebar-group',
    onDrop: (id, toIndex) => {
      const nextIds = moveIdToIndex(setIds, id, toIndex)
      setSets((previous) => applyIdOrder(previous, nextIds))
      void chatApi.reorderSets(nextIds).catch((err) => {
        console.error('Failed to reorder sets:', err)
        void loadSidebarData({ silent: true })
      })
    },
  })

  // 跟着指针走的浮起卡片：源行留在原位变淡作占位，这张卡表示「正在搬的是这个」。
  const dragGhost = useMemo(() => {
    const active = projectDrag.draggingId
      ? { drag: projectDrag, label: projects.find((p) => p.id === projectDrag.draggingId)?.name }
      : setDrag.draggingId
        ? { drag: setDrag, label: sets.find((s) => s.id === setDrag.draggingId)?.name }
        : conversationDrag.draggingId
          ? {
              drag: conversationDrag,
              label: (() => {
                const conv = visibleConversations.find((c) => c.id === conversationDrag.draggingId)
                if (!conv) return undefined
                return displayConversationTitle(conv.title, conv.preview) || t.chatLibUntitled
              })(),
            }
          : null
    if (!active?.label || !active.drag.ghostPos) return null
    return { label: active.label, ...active.drag.ghostPos }
  }, [conversationDrag, projectDrag, projects, setDrag, sets, t.chatLibUntitled, visibleConversations])

  const setConversationMap = useMemo(() => {
    const map = new Map<string, ConversationListItem[]>()
    sets.forEach((set) => {
      map.set(
        set.id,
        partitionPinnedFirst(
          applyConversationPins(
            visibleConversations.filter(
              (conversation) => (conversation.set_id ?? conversation.setId) === set.id,
            ),
            conversationPins[set.id] ?? [],
          ),
        ),
      )
    })
    return map
  }, [conversationPins, sets, visibleConversations])

  // 「最近」标签：跨集/项目的全部对话，置顶在前、再按更新时间倒序。
  const recentConversations = useMemo(
    () =>
      partitionPinnedFirst(
        [...visibleConversations].sort(
          (a, b) => (b.updated_at ?? 0) - (a.updated_at ?? 0),
        ),
      ),
    [visibleConversations],
  )

  const flatScopeOptions = [
    { value: 'all', label: t.chatSidebarAll },
    { value: 'ungrouped', label: t.chatSidebarUngrouped },
    ...projects.map(project => ({ value: `project:${project.id}`, label: `${t.chatTabProjects} · ${project.name}` })),
    ...sets.map(set => ({ value: `set:${set.id}`, label: `${t.chatSetPrefix} · ${set.name}` })),
  ]
  // A removed group falls back to the full list, without changing navigation.
  const effectiveFlatScope = flatScopeOptions.some(option => option.value === flatScope) ? flatScope : 'all'
  const flatProject = projects.find(project => effectiveFlatScope === `project:${project.id}`)
  const flatSet = sets.find(set => effectiveFlatScope === `set:${set.id}`)
  const flatScopeLabel = flatProject?.name ?? flatSet?.name
    ?? (effectiveFlatScope === 'ungrouped' ? t.chatSidebarUngrouped : t.chatSidebarAll)
  const flatConversations = recentConversations.filter(conversation => {
    const projectId = conversationProjectId(conversation)
      ?? projects.find(project => project.name === conversation.folder)?.id
    const setId = conversation.set_id ?? conversation.setId
    if (effectiveFlatScope === 'all') return true
    if (effectiveFlatScope === 'ungrouped') return !projectId && !setId && !conversation.folder
    return effectiveFlatScope === (setId ? `set:${setId}` : `project:${projectId}`)
  })
  const listedConversations = flatView ? flatConversations : recentConversations
  const openFlatScope = (scope: string) => {
    setFlatScope(scope)
    const project = projects.find(item => scope === `project:${item.id}`)
    const set = sets.find(item => scope === `set:${item.id}`)
    if (project) onSelectProject(project)
    else if (set) onSelectSet(set)
    else if (scope === 'ungrouped') onSelectProject(null)
  }
  const newListedConversation = () => {
    if (flatView && effectiveFlatScope !== 'all') {
      const project = projects.find(item => effectiveFlatScope === `project:${item.id}`)
      const set = sets.find(item => effectiveFlatScope === `set:${item.id}`)
      if (set) onSelectSet(set)
      else onSelectProject(project ?? null)
      return
    }
    onNewConversation()
  }

  // 查询变化时去后端全量索引搜（debounce 180ms）。覆盖掉出"最近 80"的老对话。
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
    // Tauri：后端全量结果优先；为空/mock 时回退到已加载列表的客户端过滤（也覆盖后端结果到达前的瞬间）。
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

  const clearableConversationCount = flatView ? flatConversations.length : selectedProject
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

  const handleSelectSearchConversation = useCallback((conversation: ConversationSearchHit) => {
    const project = findConversationProject(conversation, projects)
    onSelectConversation(conversation.id, conversation, {
      project: project ?? null,
      set: null,
    })
    closeSearch()
  }, [closeSearch, onSelectConversation, projects])

  const menuProject = projectMenuState
    ? projects.find((project) => project.id === projectMenuState.projectId)
    : undefined

  return (
    <>
      <aside
        ref={asideRef}
        className={`chat-sidebar-shell relative flex shrink-0 flex-col overflow-hidden${
          collapsed ? ' is-collapsed' : ''
        }${settingsActive ? ' is-settings-cover' : ''}`}
        aria-hidden={collapsed}
      >
        {!collapsed && (
          <div
            className="chat-sidebar-resize"
            data-tauri-drag-region="false"
            onPointerDown={handleResizeStart}
          />
        )}
        {/* 侧栏内顶栏行只在 macOS 存在：那两枚按钮要贴着系统交通灯排。
            Windows / Linux 已把它们常驻到全宽标题栏带（见 ChatTitlebar），此处渲染会重复。 */}
        {usesNativeTitlebar && (
          <div
            className={`chat-titlebar-row chat-sidebar-titlebar-row flex shrink-0 gap-2 ${chatTitlebarMacInsetClass} pr-3`}
            data-tauri-drag-region
          >
            <ChatTitlebarActions
              sidebarExpanded
              onToggleSidebar={onToggleCollapsed}
              onNewConversation={newListedConversation}
            />
            <div className="min-w-0 flex-1" data-tauri-drag-region />
          </div>
        )}

      <nav
        className={`shrink-0 space-y-0.5 px-2 pb-2 ${usesNativeTitlebar ? '' : 'pt-2'}`}
        data-tauri-drag-region="false"
      >
        <div className="flex items-center gap-1 pr-1">
          <div className="min-w-0 flex-1">
            <NavRow
              icon={<ComposeIcon size={18} strokeWidth={1.75} />}
              label={t.chatNewChat}
              onClick={newListedConversation}
            />
          </div>
          <IconButton
            size="md"
            variant="ghost"
            className="kv-nav-motion"
            label={t.chatSearch}
            aria-expanded={searchOpen}
            onClick={() => onSearchOpenChange(true)}
          >
            <SearchNavIcon size={18} strokeWidth={1.75} />
          </IconButton>
        </div>
        <NavRow
          icon={<PluginIcon size={18} strokeWidth={1.75} />}
          label={t.chatNavPlugins}
          onClick={() => onOpenExtensionsItem('plugins')}
          active={extensionsActive === 'plugins'}
        />
        <NavRow
          icon={<TasksIcon size={18} strokeWidth={1.75} />}
          label={t.chatNavTasks}
          onClick={() => onOpenExtensionsItem('tasks')}
          active={extensionsActive === 'tasks'}
        />
        <NavRow
          icon={<PortfolioIcon size={18} strokeWidth={1.75} />}
          label={t.chatNavArtifacts}
          onClick={() => onOpenExtensionsItem('artifacts')}
          active={extensionsActive === 'artifacts'}
        />
        <ExtensionsNav
          activeItem={extensionsActive === 'artifacts' || extensionsActive === 'plugins' || extensionsActive === 'tasks' ? null : extensionsActive}
          onSelectItem={onOpenExtensionsItem}
        />
      </nav>

      <div className="mx-2 border-t border-neutral-200/90" />

      <div className="flex min-h-0 flex-1 flex-col" data-tauri-drag-region="false">
        {loading ? (
          <div className="space-y-2 px-2 py-3" aria-label={t.chatLoading} aria-busy="true">
            {[0, 1, 2, 3, 4, 5].map((i) => (
              <div key={i} className="kv-skeleton h-7 rounded-lg" />
            ))}
          </div>
        ) : (
          <>
            <div className={flatView ? 'flex items-center justify-between px-2 py-1' : 'flex flex-wrap items-center justify-between gap-y-1 px-2 pb-1 pt-3'}>
              {flatView ? (
                <Select
                  className="min-w-0 flex-1"
                  ariaLabel={t.chatSidebarScope}
                  value={effectiveFlatScope}
                  onChange={openFlatScope}
                  options={flatScopeOptions}
                  triggerLabel={flatScopeLabel}
                  triggerIcon={flatProject
                    ? <ProjectIcon workdir={flatProject.root_path ?? flatProject.rootPath} color={flatProject.color} />
                    : flatSet
                      ? <Layers size={16} style={flatSet.color ? { color: flatSet.color } : undefined} />
                      : undefined}
                />
              ) : (
              <div className="flex shrink-0 items-center gap-1 whitespace-nowrap text-[13px] font-semibold">
                {([
                  ['conversations', t.chatTabRecent],
                  ['sets', t.chatTabSets],
                  ['projects', t.chatTabProjects],
                ] as const).flatMap(([tab, label], i) => {
                  const button = (
                    <button
                      key={tab}
                      type="button"
                      onClick={() => setActiveTab(tab)}
                      className={`shrink-0 rounded-md px-1 py-0.5 transition-colors ${
                        activeTab === tab
                          ? 'text-neutral-900'
                          : 'text-neutral-400 hover:text-neutral-600 dark:text-neutral-500'
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
              )}
              <div className="ml-auto flex shrink-0 items-center gap-1">
                {flatView && (
                  <>
                    <IconButton size="sm" label={t.chatNewProject} onClick={openCreateProjectDialog}>
                      <FolderPlus size={16} />
                    </IconButton>
                    <IconButton size="sm" label={t.chatSidebarNewInScope} onClick={newListedConversation}>
                      <SquarePen size={16} />
                    </IconButton>
                  </>
                )}
                {(flatView || activeTab === 'conversations') && (
                  <IconButton
                    ref={sectionMenuButtonRef}
                    size="sm"
                    onClick={openSectionMenu}
                    className={sectionMenuAnchor ? 'bg-neutral-900/[0.06] text-neutral-600' : ''}
                    label={t.chatConversationListActions}
                    aria-haspopup="menu"
                    aria-expanded={sectionMenuAnchor !== null}
                  >
                    <MoreHorizontal size={15} />
                  </IconButton>
                )}
                {!flatView && activeTab === 'sets' && (
                  <>
                    <IconButton
                      size="sm"
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
                      label={allVisibleSetsCollapsed ? t.chatExpandAllSets : t.chatCollapseAllSets}
                    >
                      <MoreHorizontal size={15} />
                    </IconButton>
                    <IconButton
                      size="sm"
                      onClick={openCreateSetDialog}
                      label={t.chatNewSet}
                    >
                      <Plus size={15} strokeWidth={2} />
                    </IconButton>
                  </>
                )}
                {!flatView && activeTab === 'projects' && (
                  <>
                    <IconButton
                      size="sm"
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
                      label={allVisibleProjectsCollapsed ? t.chatExpandAllProjects : t.chatCollapseAllProjects}
                    >
                      <MoreHorizontal size={15} />
                    </IconButton>
                    <IconButton
                      size="sm"
                      onClick={openCreateProjectDialog}
                      label={t.chatNewProject}
                      title={`${t.chatNewProject} (${modLabel}P)`}
                    >
                      <FolderPlus size={15} strokeWidth={1.75} />
                    </IconButton>
                  </>
                )}
              </div>
            </div>

            <div
              ref={groupScrollRef}
              className="kv-sidebar-groups custom-scrollbar kv-scrollbar-autohide relative min-h-0 flex-1 overflow-y-auto"
              data-tauri-drag-region="false"
            >
            {(projectDrag.lineTop ?? setDrag.lineTop ?? conversationDrag.lineTop) !== null && (
              <div
                className="kv-reorder-line"
                style={{
                  top: `${projectDrag.lineTop ?? setDrag.lineTop ?? conversationDrag.lineTop ?? 0}px`,
                }}
              />
            )}
            {dragGhost &&
              createPortal(
                <div
                  className="kv-reorder-ghost"
                  style={{ left: `${dragGhost.x}px`, top: `${dragGhost.y}px` }}
                >
                  {dragGhost.label}
                </div>,
                document.body,
              )}
            {!flatView && activeTab === 'projects' && (
            <section key="projects" className="chat-motion-tab-in group/projects px-2 pb-2 pt-1">
                <div className="mt-1.5 space-y-1">
                  {visibleProjects.map((project) => {
                    const active = selectedProject?.id === project.id
                    const projectConversations = projectConversationMap.get(project.id) ?? []
                    const collapsedProject = collapsedProjectIds.has(project.id)
                    const expanded = expandedProjectConversationIds.has(project.id)
                    const previewConversations = expanded
                      ? projectConversations
                      : projectConversations.slice(0, PROJECT_PREVIEW_LIMIT)
                    const isDragging = projectDrag.draggingId === project.id
                    return (
                      <div
                        key={project.id}
                        data-reorder-id={project.id}
                        onPointerDown={(e) => projectDrag.startDrag(e, project.id)}
                        className={`kv-sidebar-group${isDragging ? ' is-dragging' : ''}`}
                      >
                        <div
                          className={`kv-sidebar-group-row group flex min-w-0 items-center rounded-lg ${
                            active
                              ? 'bg-neutral-900/[0.04]'
                              : 'hover:bg-neutral-900/[0.035]'
                          }`}
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
                                ? 'font-semibold text-neutral-900'
                                : 'font-medium text-neutral-600'
                            }`}
                            title={(collapsedProject ? t.chatExpandNamed : t.chatCollapseNamed).replace('{name}', project.name)}
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
                          <IconButton
                            size="sm"
                            data-no-drag
                            onClick={(e) => {
                              e.stopPropagation()
                              openProjectMenu(project.id, e.currentTarget)
                            }}
                            className={`shrink-0 transition-opacity ${
                              projectMenuState?.projectId === project.id
                                ? 'opacity-100'
                                : 'opacity-0 group-hover:opacity-100'
                            }`}
                            label={t.chatProjectActions}
                          >
                            <MoreHorizontal size={15} />
                          </IconButton>
                          <IconButton
                            size="sm"
                            onClick={(e) => {
                              e.stopPropagation()
                              setCollapsedProjectIds((previous) => {
                                const next = new Set(previous)
                                next.delete(project.id)
                                return next
                              })
                              onSelectProject(project)
                            }}
                            className="mr-1 shrink-0 opacity-0 transition-opacity group-hover:opacity-100"
                            label={t.chatNewChat}
                          >
                            <SquarePen size={15} strokeWidth={1.75} />
                          </IconButton>
                        </div>

                      {!collapsedProject && previewConversations.length > 0 && (
                        <ConversationList
                          conversations={previewConversations}
                          reorder={conversationReorderFor(project.id)}
                          currentConversationId={currentConversationId}
                          generatingConversationIds={generatingConversationIds}
                          titleGeneratingConversationIds={titleGeneratingIds}
                          scheduledTaskNamesByConversation={scheduledTaskNamesByConversation}
                          projects={projects}
                          sets={sets}
                          lang={lang}
                          compact
                          indent
                          showAssistantName={false}
                          onSelectConversation={(id, conversation) => {
                            onSelectConversation(id, conversation, { project, set: null })
                          }}
                          onOpenInPopout={onOpenInPopout}
                          onRenameConversation={handleRenameConversation}
                          onRegenerateConversationTitle={handleRegenerateConversationTitle}
                          onTogglePinConversation={handleTogglePinConversation}
                          onArchiveConversation={handleArchiveConversation}
                          onExportConversation={handleExportConversation}
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
                          className="ml-8 rounded-md px-2.5 py-0.5 text-left text-[13px] font-medium text-neutral-400 transition-colors hover:bg-neutral-900/[0.035] hover:text-neutral-600 dark:text-neutral-500"
                        >
                          {expanded ? t.chatShowLess : t.chatShowMore}
                        </button>
                      )}
                      </div>
                    )
                  })}
                </div>
            </section>
            )}

            {!flatView && activeTab === 'sets' && (
            <section key="sets" className="chat-motion-tab-in group/sets px-2 pb-2 pt-1">
                <div className="mt-1.5 space-y-1">
                  {sets.length === 0 ? (
                    <button
                      type="button"
                      onClick={openCreateSetDialog}
                      className="flex w-full items-center gap-1.5 rounded-lg px-2 py-1 text-left text-[13px] text-neutral-400 transition-colors hover:bg-neutral-900/[0.035] hover:text-neutral-600 dark:text-neutral-500"
                    >
                      <Plus size={14} strokeWidth={2} className="shrink-0" />
                      {t.chatNewSetHint}
                    </button>
                  ) : (
                    sets.map((set) => {
                      const active = selectedSet?.id === set.id
                      const setConversations = setConversationMap.get(set.id) ?? []
                      const collapsedSet = collapsedSetIds.has(set.id)
                      const expanded = expandedSetConversationIds.has(set.id)
                      const previewConversations = expanded
                        ? setConversations
                        : setConversations.slice(0, PROJECT_PREVIEW_LIMIT)
                      const isDragging = setDrag.draggingId === set.id
                      return (
                        <div
                          key={set.id}
                          data-reorder-id={set.id}
                          onPointerDown={(e) => setDrag.startDrag(e, set.id)}
                          className={`kv-sidebar-group${isDragging ? ' is-dragging' : ''}`}
                        >
                          <div
                            className={`kv-sidebar-group-row group flex min-w-0 items-center rounded-lg ${
                              active
                                ? 'bg-neutral-900/[0.04]'
                                : 'hover:bg-neutral-900/[0.035]'
                            }`}
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
                                  ? 'font-semibold text-neutral-900'
                                  : 'font-medium text-neutral-600'
                              }`}
                              title={(collapsedSet ? t.chatExpandNamed : t.chatCollapseNamed).replace('{name}', set.name)}
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
                                className={`shrink-0 ${set.color ? '' : 'text-neutral-500 dark:text-neutral-400'}`}
                                style={set.color ? { color: set.color } : undefined}
                              />
                              <span className="min-w-0 truncate">{set.name}</span>
                            </button>
                            <IconButton
                              size="sm"
                              data-no-drag
                              onClick={(e) => {
                                e.stopPropagation()
                                openSetMenu(set.id, e.currentTarget)
                              }}
                              className={`shrink-0 transition-opacity ${
                                setMenuState?.setId === set.id ? 'opacity-100' : 'opacity-0 group-hover:opacity-100'
                              }`}
                              label={t.chatSetActions}
                            >
                              <MoreHorizontal size={15} />
                            </IconButton>
                            <IconButton
                              size="sm"
                              onClick={(e) => {
                                e.stopPropagation()
                                setCollapsedSetIds((previous) => {
                                  const next = new Set(previous)
                                  next.delete(set.id)
                                  return next
                                })
                                onSelectSet(set)
                              }}
                              className="mr-1 shrink-0 opacity-0 transition-opacity group-hover:opacity-100"
                              label={t.chatNewChatInSet}
                            >
                              <SquarePen size={15} strokeWidth={1.75} />
                            </IconButton>
                          </div>

                          {!collapsedSet && previewConversations.length > 0 && (
                            <ConversationList
                              conversations={previewConversations}
                              reorder={conversationReorderFor(set.id)}
                              currentConversationId={currentConversationId}
                              generatingConversationIds={generatingConversationIds}
                              titleGeneratingConversationIds={titleGeneratingIds}
                              scheduledTaskNamesByConversation={scheduledTaskNamesByConversation}
                              projects={projects}
                              sets={sets}
                              lang={lang}
                              compact
                              indent
                              showAssistantName={false}
                              onSelectConversation={(id, conversation) => {
                                onSelectConversation(id, conversation, { project: null, set })
                              }}
                              onOpenInPopout={onOpenInPopout}
                              onRenameConversation={handleRenameConversation}
                              onRegenerateConversationTitle={handleRegenerateConversationTitle}
                              onTogglePinConversation={handleTogglePinConversation}
                              onArchiveConversation={handleArchiveConversation}
                              onExportConversation={handleExportConversation}
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
                              className="ml-8 rounded-md px-2.5 py-0.5 text-left text-[13px] font-medium text-neutral-400 transition-colors hover:bg-neutral-900/[0.035] hover:text-neutral-600 dark:text-neutral-500"
                            >
                              {expanded ? t.chatShowLess : t.chatShowMore}
                            </button>
                          )}
                        </div>
                      )
                    })
                  )}
                </div>
            </section>
            )}

            {(flatView || activeTab === 'conversations') && (
            <section key="conversations" className="chat-motion-tab-in group/conversations px-2 pb-5 pt-1">
              {sectionMenuAnchor && (
                <ChatSectionMenu
                  anchor={sectionMenuAnchor}
                  hasConversations={clearableConversationCount > 0}
                  onNewConversation={newListedConversation}
                  onOpenSearch={() => onSearchOpenChange(true)}
                  flatView={flatView}
                  onToggleView={toggleSidebarView}
                  onClearAll={() => void handleClearAllConversations()}
                  onClose={() => setSectionMenuAnchor(null)}
                  triggerRef={sectionMenuButtonRef}
                />
              )}

              {listedConversations.length > 0 ? (
                <div className="mt-1.5">
                    <ConversationList
                      conversations={listedConversations}
                      cardLayout={flatView}
                      currentConversationId={currentConversationId}
                      generatingConversationIds={generatingConversationIds}
                      titleGeneratingConversationIds={titleGeneratingIds}
                      scheduledTaskNamesByConversation={scheduledTaskNamesByConversation}
                      projects={projects}
                      sets={sets}
                      lang={lang}
                      compact
                      showAssistantName={false}
                      showFolderLabel
                      onSelectConversation={(id, conversation) => {
                        const setId = conversation?.set_id ?? conversation?.setId
                        onSelectConversation(id, conversation, flatView && conversation ? {
                          project: findConversationProject(conversation, projects) ?? null,
                          set: sets.find(set => set.id === setId) ?? null,
                        } : { project: null, set: null })
                      }}
                      onOpenInPopout={onOpenInPopout}
                      onRenameConversation={handleRenameConversation}
                      onRegenerateConversationTitle={handleRegenerateConversationTitle}
                      onTogglePinConversation={handleTogglePinConversation}
                      onArchiveConversation={handleArchiveConversation}
                      onExportConversation={handleExportConversation}
                      onDeleteConversation={handleDeleteConversation}
                      onMoveConversationToProject={handleMoveConversationToProject}
                      onMoveConversationToSet={handleMoveConversationToSet}
                    />
                </div>
              ) : flatView ? (
                <p className="px-2.5 py-4 text-xs text-neutral-400" role="status">{t.chatSidebarEmpty}</p>
              ) : null}
            </section>
            )}
            </div>
          </>
        )}
      </div>

      <SidebarUserFooter
        profile={userProfile}
        lang={lang}
        settingsActive={settingsActive}
        onOpenSettings={onOpenSettings}
        onSelectLang={onSelectLang}
        onOpenUsage={onOpenUsage}
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
          onImportFromCli={() => setImportProject(menuProject)}
          onDelete={() => void handleDeleteProject(menuProject)}
          onClose={() => setProjectMenuState(null)}
        />
      )}

      {importProject && (
        <CliImportDialog
          project={importProject}
          onClose={() => setImportProject(null)}
          onOpenConversation={(conversationId) => onSelectConversation(conversationId)}
          onImported={(conversationIds) => {
            void loadSidebarData({ silent: true })
            // 导入多条时跳到第一条——总得落到某一条上，第一条是列表里最新的那个。
            if (conversationIds[0]) onSelectConversation(conversationIds[0])
          }}
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
          onSave={(name, systemPrompt, defaultAssistantId, color) =>
            void handleSaveSet(name, systemPrompt, defaultAssistantId, color)
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
        generatingConversationIds={generatingConversationIds}
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
