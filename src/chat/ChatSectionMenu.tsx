import { useEffect, useRef } from 'react'
import type { RefObject } from 'react'
import { createPortal } from 'react-dom'
import { Check, PanelsTopLeft, Trash2 } from 'lucide-react'
import { useT } from '../components/i18n'
import type { ConversationMenuAnchor } from './ConversationContextMenu'
import { useCloseAnimation } from './useCloseAnimation'

interface ChatSectionMenuProps {
  anchor: ConversationMenuAnchor
  hasConversations: boolean
  flatView: boolean
  onToggleView: () => void
  /** 仅经典视图显示：「最近」是否隐藏项目对话。 */
  recentHidesProjects: boolean
  onToggleRecentHidesProjects: () => void
  recentHidesSets: boolean
  onToggleRecentHidesSets: () => void
  onClearAll: () => void
  onClose: () => void
  triggerRef?: RefObject<HTMLElement | null>
}

export function ChatSectionMenu({
  anchor,
  hasConversations,
  flatView,
  onToggleView,
  recentHidesProjects,
  onToggleRecentHidesProjects,
  recentHidesSets,
  onToggleRecentHidesSets,
  onClearAll,
  onClose: onCloseProp,
  triggerRef,
}: ChatSectionMenuProps) {
  const t = useT()
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
      className={`kv-menu ${closing ? 'chat-motion-popover-out' : 'chat-motion-popover chat-motion-menu-cascade'} fixed z-[200] min-w-[176px]`}
      style={{ left: anchor.left, top: anchor.top }}
      role="menu"
      onAnimationEnd={onAnimationEnd}
    >
      <button
        type="button"
        role="menuitem"
        className="kv-menu-item"
        onClick={() => {
          onToggleView()
          onClose()
        }}
      >
        <PanelsTopLeft strokeWidth={1.75} />
        {flatView ? t.chatSidebarUseClassic : t.chatSidebarUseFlat}
      </button>

      {!flatView && (
        <button
          type="button"
          role="menuitemcheckbox"
          aria-checked={recentHidesProjects}
          className="kv-menu-item"
          onClick={() => {
            onToggleRecentHidesProjects()
            onClose()
          }}
        >
          <span className="flex size-3.5 shrink-0 items-center justify-center">
            {recentHidesProjects && <Check size={13} strokeWidth={2} />}
          </span>
          {t.chatSidebarRecentHideProjects}
        </button>
      )}

      {!flatView && (
        <button
          type="button"
          role="menuitemcheckbox"
          aria-checked={recentHidesSets}
          className="kv-menu-item"
          onClick={() => {
            onToggleRecentHidesSets()
            onClose()
          }}
        >
          <span className="flex size-3.5 shrink-0 items-center justify-center">
            {recentHidesSets && <Check size={13} strokeWidth={2} />}
          </span>
          {t.chatSidebarRecentHideSets}
        </button>
      )}

      <div className="my-1 border-t border-neutral-200/80" />

      <button
        type="button"
        role="menuitem"
        disabled={!hasConversations}
        className="kv-menu-item kv-menu-item--danger"
        onClick={() => {
          onClearAll()
          onClose()
        }}
      >
        <Trash2 strokeWidth={1.75} />
        {flatView ? t.chatSidebarClearList : t.chatClearAllConversations}
      </button>
    </div>
  )

  return createPortal(menu, document.body)
}
