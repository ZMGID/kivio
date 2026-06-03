import { useEffect, useRef, useState } from 'react'
import {
  FolderPlus,
  LayoutGrid,
  MoreHorizontal,
  Search,
  Settings as SettingsIcon,
  SquarePen,
} from 'lucide-react'
import type { ConversationListItem } from './types'
import { ConversationList } from './ConversationList'
import { ChatSectionMenu } from './ChatSectionMenu'
import { WindowControls } from './WindowControls'
import { chatApi } from './api'
import { ChatTitlebarActions } from './ChatTitlebarActions'
import { chatTitlebarMacInsetClass, chatTitlebarRowClass, isMac } from './platform'
import type { ConversationMenuAnchor } from './ConversationContextMenu'

const modLabel = isMac ? '⌘' : 'Ctrl'

interface SidebarProps {
  currentConversationId?: string
  onSelectConversation: (id: string) => void
  onNewConversation: () => void
  onConversationDeleted?: (id: string) => void
  onOpenSettings: () => void
  settingsActive?: boolean
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
}

function NavRow({ icon, label, shortcut, onClick, disabled, active }: NavRowProps & { active?: boolean }) {
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

export function Sidebar({
  currentConversationId,
  onSelectConversation,
  onNewConversation,
  onConversationDeleted,
  onOpenSettings,
  settingsActive = false,
  collapsed,
  onToggleCollapsed,
  refreshKey,
  searchOpen,
  onSearchOpenChange,
}: SidebarProps) {
  const [conversations, setConversations] = useState<ConversationListItem[]>([])
  const [searchQuery, setSearchQuery] = useState('')
  const [loading, setLoading] = useState(false)
  const [sectionMenuAnchor, setSectionMenuAnchor] = useState<ConversationMenuAnchor | null>(null)
  const searchInputRef = useRef<HTMLInputElement>(null)
  const sectionMenuButtonRef = useRef<HTMLButtonElement>(null)

  useEffect(() => {
    loadConversations()
  }, [refreshKey])

  useEffect(() => {
    if (searchOpen) {
      searchInputRef.current?.focus()
    }
  }, [searchOpen])

  const loadConversations = async () => {
    setLoading(true)
    try {
      const data = await chatApi.getConversations(0, 50)
      setConversations(data)
    } catch (err) {
      console.error('Failed to load conversations:', err)
    } finally {
      setLoading(false)
    }
  }

  const handleRenameConversation = async (id: string, title: string) => {
    try {
      await chatApi.updateConversation(id, { title })
      await loadConversations()
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
      await loadConversations()
    } catch (err) {
      console.error('Failed to delete conversation:', err)
    }
  }

  const handleMoveConversationToFolder = async (id: string, folder: string | undefined) => {
    try {
      await chatApi.updateConversation(id, { folder })
      await loadConversations()
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

  const handleClearAllConversations = async () => {
    if (conversations.length === 0) return
    if (!window.confirm(`确定删除全部 ${conversations.length} 个对话？此操作无法撤销。`)) return
    try {
      await Promise.all(conversations.map((conv) => chatApi.deleteConversation(conv.id)))
      if (currentConversationId) {
        onConversationDeleted?.(currentConversationId)
      }
      await loadConversations()
    } catch (err) {
      console.error('Failed to clear conversations:', err)
    }
  }

  const normalizedSearchQuery = searchQuery.trim().toLowerCase()
  const filteredConversations = normalizedSearchQuery
    ? conversations.filter(
        (c) =>
          c.title.toLowerCase().includes(normalizedSearchQuery) ||
          c.preview.toLowerCase().includes(normalizedSearchQuery)
      )
    : conversations

  if (collapsed) {
    return null
  }

  return (
    <aside className="flex h-full w-[260px] shrink-0 flex-col border-r border-neutral-200/80 bg-[#f7f7f8] dark:border-neutral-800 dark:bg-[#1c1c1e]">
      {/* 顶栏：交通灯 + 拖拽区 + 侧栏操作 */}
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
          disabled
        />
        <NavRow
          icon={<Search size={17} strokeWidth={1.75} />}
          label="搜索"
          shortcut={`${modLabel}K`}
          onClick={() => onSearchOpenChange(!searchOpen)}
        />
        <NavRow icon={<LayoutGrid size={17} strokeWidth={1.75} />} label="中心" disabled />
        <NavRow
          icon={<SettingsIcon size={17} strokeWidth={1.75} />}
          label="设置"
          active={settingsActive}
          onClick={onOpenSettings}
        />
      </nav>

      <div className="mx-3 border-t border-neutral-200/90 dark:border-neutral-800" />

      <div className="flex min-h-0 flex-1 flex-col pt-2" data-tauri-drag-region="false">
        <div className="flex items-center justify-between px-4 pb-1">
          <span className="text-[13px] font-medium text-neutral-500 dark:text-neutral-400">聊天</span>
          <button
            ref={sectionMenuButtonRef}
            type="button"
            onClick={openSectionMenu}
            className={`rounded-md p-1 text-neutral-400 transition-colors hover:bg-black/[0.05] hover:text-neutral-600 dark:hover:bg-white/[0.08] ${
              sectionMenuAnchor ? 'bg-black/[0.05] text-neutral-600 dark:bg-white/[0.08]' : ''
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
                placeholder="搜索对话"
                className="w-full rounded-lg border border-neutral-200/90 bg-white py-2 pl-8 pr-3 text-[13px] text-neutral-900 outline-none ring-0 placeholder:text-neutral-400 focus:border-neutral-300 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100"
              />
            </div>
          </div>
        )}

        <div className="custom-scrollbar flex-1 overflow-y-auto px-2 pb-3">
          {loading ? (
            <div className="px-3 py-6 text-center text-[13px] text-neutral-400">加载中…</div>
          ) : (
            <ConversationList
              conversations={filteredConversations}
              currentConversationId={currentConversationId}
              onSelectConversation={onSelectConversation}
              onRenameConversation={handleRenameConversation}
              onDeleteConversation={handleDeleteConversation}
              onMoveConversationToFolder={handleMoveConversationToFolder}
            />
          )}
        </div>
      </div>
    </aside>
  )
}
