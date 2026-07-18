import { useEffect, useRef } from 'react'
import type { RefObject } from 'react'
import { createPortal } from 'react-dom'
import { Search, SquarePen, Trash2 } from 'lucide-react'
import type { ConversationMenuAnchor } from './ConversationContextMenu'
import { useCloseAnimation } from './useCloseAnimation'

interface ChatSectionMenuProps {
  anchor: ConversationMenuAnchor
  hasConversations: boolean
  onNewConversation: () => void
  onOpenSearch: () => void
  onClearAll: () => void
  onClose: () => void
  triggerRef?: RefObject<HTMLElement | null>
}

export function ChatSectionMenu({
  anchor,
  hasConversations,
  onNewConversation,
  onOpenSearch,
  onClearAll,
  onClose: onCloseProp,
  triggerRef,
}: ChatSectionMenuProps) {
  const menuRef = useRef<HTMLDivElement>(null)
  const { closing, startClose, onAnimationEnd } = useCloseAnimation(onCloseProp)
  const onClose = startClose

  useEffect(() => {
    const onPointerDown = (e: MouseEvent) => {
      const target = e.target as Node
      if (menuRef.current?.contains(target)) return
      // 触发按钮自己处理 toggle，别让外部关闭抢先把菜单关掉再被点开。
      if (triggerRef?.current?.contains(target)) return
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
  }, [onClose, triggerRef])

  const menu = (
    <div
      ref={menuRef}
      className={`${closing ? 'chat-motion-popover-out' : 'chat-motion-popover'} fixed z-[200] min-w-[200px] rounded-xl border border-neutral-200/90 bg-white py-1.5 shadow-lg dark:border-neutral-700 dark:bg-[#2a2a2c]`}
      style={{ left: anchor.left, top: anchor.top }}
      role="menu"
      onAnimationEnd={onAnimationEnd}
    >
      <button
        type="button"
        role="menuitem"
        className="flex w-full items-center gap-3 px-3.5 py-2 text-left text-[13px] text-neutral-800 transition-colors hover:bg-black/[0.04] dark:text-neutral-100 dark:hover:bg-white/[0.06]"
        onClick={() => {
          onNewConversation()
          onClose()
        }}
      >
        <SquarePen size={16} strokeWidth={1.75} className="shrink-0 text-neutral-500" />
        新建聊天
      </button>
      <button
        type="button"
        role="menuitem"
        className="flex w-full items-center gap-3 px-3.5 py-2 text-left text-[13px] text-neutral-800 transition-colors hover:bg-black/[0.04] dark:text-neutral-100 dark:hover:bg-white/[0.06]"
        onClick={() => {
          onOpenSearch()
          onClose()
        }}
      >
        <Search size={16} strokeWidth={1.75} className="shrink-0 text-neutral-500" />
        搜索对话
      </button>

      <div className="my-1 border-t border-neutral-200/80 dark:border-neutral-700" />

      <button
        type="button"
        role="menuitem"
        disabled={!hasConversations}
        className="flex w-full items-center gap-3 px-3.5 py-2 text-left text-[13px] text-red-600 transition-colors hover:bg-red-50 disabled:cursor-default disabled:opacity-40 dark:text-red-400 dark:hover:bg-red-500/10"
        onClick={() => {
          onClearAll()
          onClose()
        }}
      >
        <Trash2 size={16} strokeWidth={1.75} className="shrink-0" />
        清空全部对话
      </button>
    </div>
  )

  return createPortal(menu, document.body)
}
