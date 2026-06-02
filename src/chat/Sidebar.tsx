import { useState, useEffect } from 'react'
import { MessageSquare, Search, Plus, FolderOpen, Box, Settings as SettingsIcon } from 'lucide-react'
import type { ConversationListItem } from './types'
import { ConversationList } from './ConversationList'
import { chatApi } from './api'
import { groupConversationsByTime } from './utils'

interface SidebarProps {
  currentConversationId?: string
  onSelectConversation: (id: string) => void
  onNewConversation: () => void
  onOpenSettings: () => void
  collapsed: boolean
  refreshKey: number
}

export function Sidebar({
  currentConversationId,
  onSelectConversation,
  onNewConversation,
  onOpenSettings,
  collapsed,
  refreshKey,
}: SidebarProps) {
  const [conversations, setConversations] = useState<ConversationListItem[]>([])
  const [searchQuery, setSearchQuery] = useState('')
  const [loading, setLoading] = useState(false)

  // 加载对话列表
  useEffect(() => {
    loadConversations()
  }, [refreshKey])

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

  // 搜索过滤
  const normalizedSearchQuery = searchQuery.trim().toLowerCase()
  const filteredConversations = normalizedSearchQuery
    ? conversations.filter(
        (c) =>
          c.title.toLowerCase().includes(normalizedSearchQuery) ||
          c.preview.toLowerCase().includes(normalizedSearchQuery)
      )
    : conversations

  const groups = groupConversationsByTime(filteredConversations)

  if (collapsed) {
    return null
  }

  return (
    <div className="w-80 h-screen flex flex-col bg-neutral-50 dark:bg-neutral-900 border-r border-neutral-200 dark:border-neutral-800">
      {/* 顶部工具栏 */}
      <div className="p-3 border-b border-neutral-200 dark:border-neutral-800 space-y-2">
        <button
          onClick={onNewConversation}
          className="w-full flex items-center gap-2 px-3 py-2.5 rounded-lg bg-blue-500 hover:bg-blue-600 text-white transition-colors font-medium text-sm"
        >
          <Plus size={18} strokeWidth={2} />
          <span>新建聊天</span>
        </button>

        {/* 搜索框 */}
        <div className="relative">
          <Search
            size={16}
            className="absolute left-3 top-1/2 -translate-y-1/2 text-neutral-400"
          />
          <input
            type="text"
            placeholder="搜索"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            className="w-full pl-9 pr-3 py-2 bg-white dark:bg-neutral-800 border border-neutral-200 dark:border-neutral-700 rounded-lg text-sm focus:outline-none focus:ring-2 focus:ring-blue-500 dark:focus:ring-blue-400"
          />
        </div>
      </div>

      {/* 快捷入口 */}
      <div className="px-3 py-2 border-b border-neutral-200 dark:border-neutral-800">
        <button className="w-full flex items-center gap-2.5 px-3 py-2 rounded-lg hover:bg-neutral-100 dark:hover:bg-neutral-800 transition-colors text-neutral-700 dark:text-neutral-300 text-sm">
          <MessageSquare size={18} />
          <span>ChatGPT</span>
        </button>
        <button className="w-full flex items-center gap-2.5 px-3 py-2 rounded-lg hover:bg-neutral-100 dark:hover:bg-neutral-800 transition-colors text-neutral-700 dark:text-neutral-300 text-sm">
          <FolderOpen size={18} />
          <span>库</span>
        </button>
        <button className="w-full flex items-center gap-2.5 px-3 py-2 rounded-lg hover:bg-neutral-100 dark:hover:bg-neutral-800 transition-colors text-neutral-700 dark:text-neutral-300 text-sm">
          <Box size={18} />
          <span>GPT</span>
        </button>
      </div>

      {/* 对话列表 */}
      <div className="flex-1 overflow-y-auto">
        {loading ? (
          <div className="p-4 text-center text-sm text-neutral-500">加载中...</div>
        ) : (
          <ConversationList
            groups={groups}
            currentConversationId={currentConversationId}
            onSelectConversation={onSelectConversation}
          />
        )}
      </div>

      {/* 底部设置按钮 */}
      <div className="p-3 border-t border-neutral-200 dark:border-neutral-800">
        <button
          onClick={onOpenSettings}
          className="w-full flex items-center gap-2.5 px-3 py-2 rounded-lg hover:bg-neutral-100 dark:hover:bg-neutral-800 transition-colors text-neutral-700 dark:text-neutral-300 text-sm"
        >
          <SettingsIcon size={18} />
          <span>设置</span>
        </button>
      </div>
    </div>
  )
}
