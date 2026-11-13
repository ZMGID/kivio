import { FolderIcon as Folder } from '../components/FileIcon'
import { useEffect, useRef, useState, type ReactNode, type RefObject } from 'react'
import { createPortal } from 'react-dom'
import { ChevronRight, Copy, Download, Hash, Layers, RotateCcw, SquareArrowOutUpRight, Trash2 } from 'lucide-react'
import { i18n, type Lang } from '../components/i18n'
import type { ChatProject, ChatSet } from './types'
import { useCloseAnimation } from './useCloseAnimation'
import { useClampedMenuPosition } from './useClampedMenuPosition'

export interface ConversationMenuAnchor {
  left: number
  top: number
}

interface ConversationContextMenuProps {
  anchor: ConversationMenuAnchor
  conversationFolder?: string
  conversationProjectId?: string | null
  conversationSetId?: string | null
  projects: ChatProject[]
  sets: ChatSet[]
  lang: Lang
  canRegenerateTitle?: boolean
  regeneratingTitle?: boolean
  showNativeSession?: boolean
  nativeSessionId?: string | null
  nativeSessionLoading?: boolean
  onRegenerateTitle: () => void
  onExport: () => void
  onMoveToProject: (projectId: string | undefined) => void
  onMoveToSet: (setId: string | undefined) => void
  onOpenInPopout?: () => void
  onDelete: () => void
  onClose: () => void
}

function ConversationSubmenu({ anchor, label, menuRef, children }: {
  anchor: ConversationMenuAnchor
  label: string
  menuRef: RefObject<HTMLDivElement>
  children: ReactNode
}) {
  const pos = useClampedMenuPosition(menuRef, anchor)
  return createPortal(
    <div ref={menuRef} role="menu" aria-label={label}
      className="kv-menu fixed z-[201] min-w-[168px] max-w-[calc(100vw-16px)] max-h-[min(70vh,420px)] overflow-y-auto custom-scrollbar"
      style={{ left: pos.left, top: pos.top }}>
      {children}
    </div>,
    document.body,
  )
}

function shortNativeId(id: string): string {
  if (id.length <= 22) return id
  return `${id.slice(0, 8)}…${id.slice(-6)}`
}

export function ConversationContextMenu({
  anchor,
  conversationFolder,
  conversationProjectId,
  conversationSetId,
  projects,
  sets,
  lang,
  canRegenerateTitle = true,
  regeneratingTitle = false,
  showNativeSession = false,
  nativeSessionId = null,
  nativeSessionLoading = false,
  onRegenerateTitle,
  onExport,
  onMoveToProject,
  onMoveToSet,
  onOpenInPopout,
  onDelete,
  onClose: onCloseProp,
}: ConversationContextMenuProps) {
  const t = i18n[lang]
  const menuRef = useRef<HTMLDivElement>(null)
  const pos = useClampedMenuPosition(menuRef, anchor)
  const submenuRef = useRef<HTMLDivElement>(null)
  const [submenu, setSubmenu] = useState<{ kind: 'project' | 'set'; anchor: ConversationMenuAnchor } | null>(null)
  const openSubmenu = (kind: 'project' | 'set', button: HTMLButtonElement) => {
    const rect = button.getBoundingClientRect()
    setSubmenu({ kind, anchor: { left: rect.right + 4, top: rect.top } })
  }
  const { closing, startClose, onAnimationEnd } = useCloseAnimation(onCloseProp)
  // 所有内部关闭触发（菜单项动作后 / 外部点击 / Esc）走 startClose，先播退场再卸载
  const onClose = startClose

  useEffect(() => {
    const onPointerDown = (e: MouseEvent) => {
      const target = e.target
      if (target instanceof Node && (menuRef.current?.contains(target) || submenuRef.current?.contains(target))) return
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
      className={`kv-menu ${closing ? 'chat-motion-popover-out' : 'chat-motion-popover chat-motion-menu-cascade'} fixed z-[200] min-w-[176px]`}
      style={{ left: pos.left, top: pos.top }}
      role="menu"
      onAnimationEnd={onAnimationEnd}
    >
      <button
        type="button"
        role="menuitem"
        className="kv-menu-item"
        disabled={!canRegenerateTitle || regeneratingTitle}
        onClick={() => {
          onRegenerateTitle()
          onClose()
        }}
      >
        <RotateCcw strokeWidth={1.75} />
        {t.chatRegenerateTitle}
      </button>

      {onOpenInPopout && (
        <button
          type="button"
          role="menuitem"
          className="kv-menu-item"
          onClick={() => {
            onOpenInPopout()
            onClose()
          }}
        >
          <SquareArrowOutUpRight strokeWidth={1.75} />
          {t.chatOpenInNewWindow}
        </button>
      )}

      <div>
        <button
          type="button"
          role="menuitem"
          className="kv-menu-item"
          aria-haspopup="menu"
          aria-expanded={submenu?.kind === 'project'}
          onMouseEnter={(event) => openSubmenu('project', event.currentTarget)}
          onClick={(event) => openSubmenu('project', event.currentTarget)}
        >
          <Folder />
          <span className="min-w-0 flex-1">{t.chatAddToProject}</span>
          <ChevronRight size={16} className="shrink-0 text-neutral-400" />
        </button>

        {!closing && submenu?.kind === 'project' && (
          <ConversationSubmenu anchor={submenu.anchor} label={t.chatAddToProject} menuRef={submenuRef}>
            {projects.length === 0 ? (
              <div className="kv-menu-item">{t.chatNoProjects}</div>
            ) : (
              projects.map((project) => {
                const active = conversationProjectId
                  ? conversationProjectId === project.id
                  : conversationFolder === project.name
                return (
                <button
                  key={project.id}
                  type="button"
                  className={`kv-menu-item ${
                    active
                      ? 'font-medium text-neutral-900'
                      : 'text-neutral-800'
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
                <div className="my-1 border-t border-neutral-200/80" />
                <button
                  type="button"
                  className="kv-menu-item"
                  onClick={() => {
                    onMoveToProject(undefined)
                    onClose()
                  }}
                >
                  {t.chatRemoveFromProject}
                </button>
              </>
            )}
          </ConversationSubmenu>
        )}
      </div>

      <div>
        <button
          type="button"
          role="menuitem"
          className="kv-menu-item"
          aria-haspopup="menu"
          aria-expanded={submenu?.kind === 'set'}
          onMouseEnter={(event) => openSubmenu('set', event.currentTarget)}
          onClick={(event) => openSubmenu('set', event.currentTarget)}
        >
          <Layers strokeWidth={1.75} />
          <span className="min-w-0 flex-1">{t.chatMoveToSet}</span>
          <ChevronRight size={16} className="shrink-0 text-neutral-400" />
        </button>

        {!closing && submenu?.kind === 'set' && (
          <ConversationSubmenu anchor={submenu.anchor} label={t.chatMoveToSet} menuRef={submenuRef}>
            {sets.length === 0 ? (
              <div className="kv-menu-item">{t.chatNoSets}</div>
            ) : (
              sets.map((set) => {
                const active = conversationSetId === set.id
                return (
                  <button
                    key={set.id}
                    type="button"
                    className={`kv-menu-item ${
                      active
                        ? 'font-medium text-neutral-900'
                        : 'text-neutral-800'
                    }`}
                    onClick={() => {
                      onMoveToSet(set.id)
                      onClose()
                    }}
                  >
                    {set.name}
                  </button>
                )
              })
            )}
            {conversationSetId && (
              <>
                <div className="my-1 border-t border-neutral-200/80" />
                <button
                  type="button"
                  className="kv-menu-item"
                  onClick={() => {
                    onMoveToSet(undefined)
                    onClose()
                  }}
                >
                  {t.chatRemoveFromSet}
                </button>
              </>
            )}
          </ConversationSubmenu>
        )}
      </div>

      <button
        type="button"
        role="menuitem"
        className="kv-menu-item"
        onClick={() => {
          onExport()
          onClose()
        }}
      >
        <Download strokeWidth={1.75} />
        {t.chatExport}
      </button>

      <div className="my-1 border-t border-neutral-200/80" />

      <button
        type="button"
        role="menuitem"
        className="kv-menu-item kv-menu-item--danger"
        onClick={() => {
          onDelete()
          onClose()
        }}
      >
        <Trash2 strokeWidth={1.75} />
        {t.chatDelete}
      </button>

      {showNativeSession && (
        <>
          <div className="my-1 border-t border-neutral-200/80" />
          <button
            type="button"
            role="menuitem"
            className="kv-menu-item"
            disabled={!nativeSessionId}
            title={nativeSessionId ? t.chatNativeSessionCopy : undefined}
            onClick={() => {
              if (!nativeSessionId) return
              void navigator.clipboard.writeText(nativeSessionId)
              onClose()
            }}
          >
            {nativeSessionId ? <Copy strokeWidth={1.75} /> : <Hash strokeWidth={1.75} />}
            <span className="min-w-0 flex-1 truncate">
              {t.chatNativeSessionId}
              <span className="ml-1.5 font-mono text-[11px] text-neutral-500 dark:text-neutral-400">
                {nativeSessionLoading
                  ? '…'
                  : nativeSessionId
                    ? shortNativeId(nativeSessionId)
                    : t.chatNativeSessionUnbound}
              </span>
            </span>
          </button>
        </>
      )}
    </div>
  )

  return createPortal(menu, document.body)
}
