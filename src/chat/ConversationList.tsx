import { memo, useEffect, useRef, useState } from 'react'
import { MoreHorizontal } from 'lucide-react'
import type { ChatProject, ConversationListItem } from './types'
import {
  ConversationContextMenu,
  type ConversationMenuAnchor,
} from './ConversationContextMenu'

interface ConversationListProps {
  conversations: ConversationListItem[]
  currentConversationId?: string
  generatingConversationIds?: ReadonlySet<string>
  projects: ChatProject[]
  compact?: boolean
  emptyLabel?: string
  indent?: boolean
  showAssistantName?: boolean
  onSelectConversation: (id: string) => void
  onRenameConversation: (id: string, title: string) => Promise<void>
  onDeleteConversation: (id: string) => Promise<void>
  onMoveConversationToProject: (id: string, projectId: string | undefined) => Promise<void>
}

export const ConversationList = memo(function ConversationList({
  conversations,
  currentConversationId,
  generatingConversationIds = new Set(),
  projects,
  compact = false,
  emptyLabel = '暂无对话',
  indent = false,
  showAssistantName = true,
  onSelectConversation,
  onRenameConversation,
  onDeleteConversation,
  onMoveConversationToProject,
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
      <div className={compact ? 'space-y-0.5 py-0.5' : 'space-y-0.5 py-1'}>
        {conversations.map((conv) => {
          const active = currentConversationId === conv.id
          const isGenerating = generatingConversationIds.has(conv.id)
          const isRenaming = renamingId === conv.id

          if (isRenaming) {
            return (
              <div
                key={conv.id}
                className={`${indent ? 'pl-8 pr-1' : 'px-1'} py-0.5`}
              >
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
                  ? 'bg-black/[0.07] dark:bg-white/[0.11]'
                  : 'hover:bg-black/[0.04] dark:hover:bg-white/[0.06]'
              }`}
            >
              <button
                type="button"
                onClick={() => onSelectConversation(conv.id)}
                className={`min-w-0 flex-1 text-left transition-colors ${
                  compact
                    ? `${indent ? 'pl-8' : 'pl-2.5'} pr-2 py-1 text-[13px] leading-5`
                    : 'px-3 py-2 text-[13px]'
                } ${
                  active
                    ? 'font-semibold text-neutral-900 dark:text-neutral-100'
                    : compact
                      ? 'font-medium text-neutral-700 dark:text-neutral-300'
                      : 'text-neutral-700 dark:text-neutral-300'
                }`}
                title={isGenerating ? `${conv.title}（正在生成…）` : conv.title}
              >
                <span className="flex min-w-0 items-center gap-1.5">
                  <span className="block min-w-0 flex-1 truncate">{conv.title}</span>
                  {isGenerating && (
                    <span
                      className="inline-flex h-3.5 w-3.5 shrink-0 animate-spin rounded-full border-[1.5px] border-neutral-300 border-t-neutral-600 dark:border-neutral-600 dark:border-t-neutral-200"
                      aria-label="正在生成"
                    />
                  )}
                </span>
                {showAssistantName && (conv.assistant_name ?? conv.assistantName) && (
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
                className={`mr-1 shrink-0 rounded-md p-0.5 text-neutral-400 transition-opacity hover:bg-black/[0.06] hover:text-neutral-600 dark:hover:bg-white/[0.1] dark:hover:text-neutral-200 ${
                  menuState?.conversationId === conv.id
                    ? 'opacity-100'
                    : 'opacity-0 group-hover:opacity-100'
                }`}
                aria-label="对话操作"
              >
                <MoreHorizontal size={15} />
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
          conversationProjectId={menuConversation.project_id ?? menuConversation.projectId ?? null}
          projects={projects}
          onRename={() => startRename(menuConversation)}
          onMoveToProject={(projectId) => void onMoveConversationToProject(menuConversation.id, projectId)}
          onDelete={() => void onDeleteConversation(menuConversation.id)}
          onClose={() => setMenuState(null)}
        />
      )}
    </>
  )
})
