import { useEffect, useRef } from 'react'
import { createPortal } from 'react-dom'
import { ChevronRight, Folder, Pencil, Trash2 } from 'lucide-react'
import type { ChatProject } from './types'

export interface ConversationMenuAnchor {
  left: number
  top: number
}

interface ConversationContextMenuProps {
  anchor: ConversationMenuAnchor
  conversationTitle: string
  conversationFolder?: string
  conversationProjectId?: string | null
  projects: ChatProject[]
  onRename: () => void
  onMoveToProject: (projectId: string | undefined) => void
  onDelete: () => void
  onClose: () => void
}

export function ConversationContextMenu({
  anchor,
  conversationFolder,
  conversationProjectId,
  projects,
  onRename,
  onMoveToProject,
  onDelete,
  onClose,
}: ConversationContextMenuProps) {
  const menuRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const onPointerDown = (e: MouseEvent) => {
      const target = e.target as Node
      if (menuRef.current?.contains(target)) return
      onClose()
    }
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('mousedown', onPointerDown)
    window.addEventListener('keydown', onKeyDown)
    return () => {
      window.removeEventListener('mousedown', onPointerDown)
      window.removeEventListener('keydown', onKeyDown)
    }
  }, [onClose])

  const menu = (
    <div
      ref={menuRef}
      className="chat-motion-popover fixed z-[200] min-w-[200px] rounded-xl border border-neutral-200/90 bg-white py-1.5 shadow-lg dark:border-neutral-700 dark:bg-[#2a2a2c]"
      style={{ left: anchor.left, top: anchor.top }}
      role="menu"
    >
      <button
        type="button"
        role="menuitem"
        className="flex w-full items-center gap-3 px-3.5 py-2 text-left text-[13px] text-neutral-800 transition-colors hover:bg-black/[0.04] dark:text-neutral-100 dark:hover:bg-white/[0.06]"
        onClick={() => {
          onRename()
          onClose()
        }}
      >
        <Pencil size={16} strokeWidth={1.75} className="shrink-0 text-neutral-500" />
        重命名
      </button>

      <div className="group/sub relative">
        <button
          type="button"
          role="menuitem"
          className="flex w-full items-center gap-3 px-3.5 py-2 text-left text-[13px] text-neutral-800 transition-colors hover:bg-black/[0.04] dark:text-neutral-100 dark:hover:bg-white/[0.06]"
        >
          <Folder size={16} strokeWidth={1.75} className="shrink-0 text-neutral-500" />
          <span className="min-w-0 flex-1">添加到项目</span>
          <ChevronRight size={16} className="shrink-0 text-neutral-400" />
        </button>

        <div className="pointer-events-none absolute left-full top-0 z-[201] min-w-[168px] pl-1 opacity-0 transition-opacity group-hover/sub:pointer-events-auto group-hover/sub:opacity-100">
          <div className="rounded-xl border border-neutral-200/90 bg-white py-1.5 shadow-lg dark:border-neutral-700 dark:bg-[#2a2a2c]">
            {projects.length === 0 ? (
              <div className="px-3.5 py-2 text-[13px] text-neutral-400">暂无项目</div>
            ) : (
              projects.map((project) => {
                const active = conversationProjectId
                  ? conversationProjectId === project.id
                  : conversationFolder === project.name
                return (
                <button
                  key={project.id}
                  type="button"
                  className={`flex w-full px-3.5 py-2 text-left text-[13px] transition-colors hover:bg-black/[0.04] dark:hover:bg-white/[0.06] ${
                    active
                      ? 'font-medium text-neutral-900 dark:text-neutral-50'
                      : 'text-neutral-800 dark:text-neutral-100'
                  }`}
                  onClick={() => {
                    onMoveToProject(project.id)
                    onClose()
                  }}
                >
                  {project.name}
                </button>
                )
              })
            )}
            {(conversationProjectId || conversationFolder) && (
              <>
                <div className="my-1 border-t border-neutral-200/80 dark:border-neutral-700" />
                <button
                  type="button"
                  className="flex w-full px-3.5 py-2 text-left text-[13px] text-neutral-600 transition-colors hover:bg-black/[0.04] dark:text-neutral-300 dark:hover:bg-white/[0.06]"
                  onClick={() => {
                    onMoveToProject(undefined)
                    onClose()
                  }}
                >
                  移出项目
                </button>
              </>
            )}
          </div>
        </div>
      </div>

      <div className="my-1 border-t border-neutral-200/80 dark:border-neutral-700" />

      <button
        type="button"
        role="menuitem"
        className="flex w-full items-center gap-3 px-3.5 py-2 text-left text-[13px] text-red-600 transition-colors hover:bg-red-50 dark:text-red-400 dark:hover:bg-red-500/10"
        onClick={() => {
          onDelete()
          onClose()
        }}
      >
        <Trash2 size={16} strokeWidth={1.75} className="shrink-0" />
        删除
      </button>
    </div>
  )

  return createPortal(menu, document.body)
}
