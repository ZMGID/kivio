import { memo, useEffect, useRef, useState } from 'react'
import { MoreHorizontal } from 'lucide-react'
import type { ConversationListItem } from './types'
import {
  ConversationContextMenu,
  type ConversationMenuAnchor,
} from './ConversationContextMenu'

interface ConversationListProps {
  conversations: ConversationListItem[]
  currentConversationId?: string
  projectFolders: string[]
  emptyLabel?: string
  onSelectConversation: (id: string) => void
  onRenameConversation: (id: string, title: string) => Promise<void>
  onDeleteConversation: (id: string) => Promise<void>
  onMoveConversationToFolder: (id: string, folder: string | undefined) => Promise<void>
}

export const ConversationList = memo(function ConversationList({
  conversations,
  currentConversationId,
  projectFolders,
  emptyLabel = '暂无对话',
  onSelectConversation,
  onRenameConversation,
  onDeleteConversation,
  onMoveConversationToFolder,
}: ConversationListProps) {
  const [menuState, setMenuState] = useState<{
    conversationId: string
    anchor: ConversationMenuAnchor
  } | null>(null)
  const [renamingId, setRenamingId] = useState<string | null>(null)
  const [renameDraft, setRenameDraft] = useState('')
  const renameInputRef = useRef<HTMLInputElement>(null)

  const menuConversation = menuState
    ? conversations.find((c) => c.id === menuState.conversationId)
    : undefined

  useEffect(() => {
    if (renamingId) {
      renameInputRef.current?.focus()
      renameInputRef.current?.select()
    }
  }, [renamingId])

  const openMenu = (conversationId: string, button: HTMLButtonElement) => {
    const rect = button.getBoundingClientRect()
    setMenuState({
      conversationId,
      anchor: { left: rect.right - 200, top: rect.bottom + 4 },
    })
  }

  const startRename = (conv: ConversationListItem) => {
    setRenamingId(conv.id)
    setRenameDraft(conv.title)
    setMenuState(null)
  }

  const commitRename = async (conversationId: string) => {
    const nextTitle = renameDraft.trim()
    setRenamingId(null)
    if (!nextTitle) return
    const conv = conversations.find((c) => c.id === conversationId)
    if (!conv || conv.title === nextTitle) return
    await onRenameConversation(conversationId, nextTitle)
  }

  if (conversations.length === 0) {
    return (
      <div className="px-3 py-10 text-center text-[13px] text-neutral-400 dark:text-neutral-500">
        {emptyLabel}
      </div>
    )
  }

  return (
    <>
      <div className="space-y-0.5 py-1">
        {conversations.map((conv) => {
          const active = currentConversationId === conv.id
          const isRenaming = renamingId === conv.id

          if (isRenaming) {
            return (
              <div key={conv.id} className="px-1 py-0.5">
                <input
                  ref={renameInputRef}
                  type="text"
                  value={renameDraft}
                  onChange={(e) => setRenameDraft(e.target.value)}
                  onBlur={() => void commitRename(conv.id)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') {
                      e.preventDefault()
                      void commitRename(conv.id)
                    }
                    if (e.key === 'Escape') {
                      setRenamingId(null)
                    }
                  }}
                  className="w-full rounded-lg border border-neutral-300 bg-white px-3 py-2 text-[13px] text-neutral-900 outline-none ring-0 focus:border-neutral-400 dark:border-neutral-600 dark:bg-neutral-900 dark:text-neutral-100"
                />
              </div>
            )
          }

          return (
            <div
              key={conv.id}
              className={`group relative flex min-w-0 items-center rounded-lg ${
                active
                  ? 'bg-black/[0.06] dark:bg-white/[0.1]'
                  : 'hover:bg-black/[0.04] dark:hover:bg-white/[0.06]'
              }`}
            >
              <button
                type="button"
                onClick={() => onSelectConversation(conv.id)}
                className={`min-w-0 flex-1 px-3 py-2 text-left text-[13px] transition-colors ${
                  active
                    ? 'font-medium text-neutral-900 dark:text-neutral-100'
                    : 'text-neutral-700 dark:text-neutral-300'
                }`}
                title={conv.title}
              >
                <span className="block truncate">{conv.title}</span>
                {(conv.assistant_name ?? conv.assistantName) && (
                  <span className="mt-0.5 block truncate text-[11px] font-normal text-neutral-400 dark:text-neutral-500">
                    {(conv.assistant_name ?? conv.assistantName)}
                  </span>
                )}
              </button>
              <button
                type="button"
                onClick={(e) => {
                  e.stopPropagation()
                  openMenu(conv.id, e.currentTarget)
                }}
                className={`mr-1 shrink-0 rounded-md p-1 text-neutral-400 transition-opacity hover:bg-black/[0.06] hover:text-neutral-600 dark:hover:bg-white/[0.1] dark:hover:text-neutral-200 ${
                  menuState?.conversationId === conv.id
                    ? 'opacity-100'
                    : 'opacity-0 group-hover:opacity-100'
                }`}
                aria-label="对话操作"
              >
                <MoreHorizontal size={16} />
              </button>
            </div>
          )
        })}
      </div>

      {menuState && menuConversation && (
        <ConversationContextMenu
          anchor={menuState.anchor}
          conversationTitle={menuConversation.title}
          conversationFolder={menuConversation.folder}
          projectFolders={projectFolders}
          onRename={() => startRename(menuConversation)}
          onMoveToFolder={(folder) => void onMoveConversationToFolder(menuConversation.id, folder)}
          onDelete={() => void onDeleteConversation(menuConversation.id)}
          onClose={() => setMenuState(null)}
        />
      )}
    </>
  )
})
