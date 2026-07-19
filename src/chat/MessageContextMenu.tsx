import { useEffect, useRef } from 'react'
import { createPortal } from 'react-dom'
import { Copy, ClipboardCopy } from 'lucide-react'
import { useCloseAnimation } from './useCloseAnimation'

export interface MessageMenuAnchor {
  left: number
  top: number
}

interface MessageContextMenuProps {
  anchor: MessageMenuAnchor
  hasSelection: boolean
  canCopyMessage: boolean
  onCopySelection: () => void
  onCopyMessage: () => void
  onClose: () => void
}

// 消息区内置右键菜单：替代被屏蔽的原生菜单，提供「复制选中」/「复制整条消息」。
export function MessageContextMenu({
  anchor,
  hasSelection,
  canCopyMessage,
  onCopySelection,
  onCopyMessage,
  onClose: onCloseProp,
}: MessageContextMenuProps) {
  const menuRef = useRef<HTMLDivElement>(null)
  const { closing, startClose, onAnimationEnd } = useCloseAnimation(onCloseProp)
  const onClose = startClose

  useEffect(() => {
    const onPointerDown = (e: MouseEvent) => {
      if (menuRef.current?.contains(e.target as Node)) return
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

  const itemClass =
    'flex w-full items-center gap-3 px-3.5 py-2 text-left text-[13px] text-neutral-800 transition-colors hover:bg-black/[0.04] dark:text-neutral-100 dark:hover:bg-white/[0.06]'

  const menu = (
    <div
      ref={menuRef}
      className={`${closing ? 'chat-motion-popover-out' : 'chat-motion-popover chat-motion-menu-cascade'} fixed z-[200] min-w-[168px] rounded-xl border border-neutral-200/90 bg-white py-1.5 shadow-lg dark:border-neutral-700 dark:bg-[#2a2a2c]`}
      style={{ left: anchor.left, top: anchor.top }}
      role="menu"
      onAnimationEnd={onAnimationEnd}
    >
      {hasSelection && (
        <button
          type="button"
          role="menuitem"
          className={itemClass}
          onClick={() => {
            onCopySelection()
            onClose()
          }}
        >
          <Copy size={16} strokeWidth={1.75} className="shrink-0 text-neutral-500" />
          复制
        </button>
      )}
      {canCopyMessage && (
        <button
          type="button"
          role="menuitem"
          className={itemClass}
          onClick={() => {
            onCopyMessage()
            onClose()
          }}
        >
          <ClipboardCopy size={16} strokeWidth={1.75} className="shrink-0 text-neutral-500" />
          复制整条消息
        </button>
      )}
    </div>
  )

  return createPortal(menu, document.body)
}
