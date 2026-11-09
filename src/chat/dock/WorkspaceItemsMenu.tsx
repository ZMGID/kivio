// 工作区面板「选择显示的内容」，参照 T3 Code 的 ThreadDetailsItemsMenu：
// 右键面板打开，勾选项即时生效；全部隐藏时面板里留一行入口用来恢复。
import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { Check } from 'lucide-react'
import { i18n, type Lang } from '../../components/i18n'
import { PANEL_SCOPE_ATTR } from './WorkspacePanelParts'
import { WORKSPACE_ITEMS, type WorkspaceItemId } from './workspaceItems'

export function WorkspaceItemsMenu({ at, lang, visible, onToggle, onClose }: {
  at: { x: number; y: number }
  lang: Lang
  visible: Record<WorkspaceItemId, boolean>
  onToggle: (id: WorkspaceItemId) => void
  onClose: () => void
}) {
  const t = i18n[lang]
  const ref = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState(at)
  const labels: Record<WorkspaceItemId, string> = {
    project: t.workspaceItemProject,
    environment: t.workspaceItemEnvironment,
    openIn: t.workspaceItemOpenIn,
    branch: t.workspaceItemBranch,
    gitActions: t.workspaceItemGitActions,
    changes: t.workspaceChanges,
    history: t.workspaceHistoryBranches,
    tasks: t.dockTabTasks,
    subagents: t.workspaceSubagents,
    usage: t.workspaceItemUsage,
    context: t.workspaceItemContext,
  }

  // 贴在点击位置，超出窗口时往回收。
  useLayoutEffect(() => {
    const rect = ref.current?.getBoundingClientRect()
    if (!rect) return
    setPos({
      x: Math.max(8, Math.min(at.x, window.innerWidth - rect.width - 8)),
      y: Math.max(8, Math.min(at.y, window.innerHeight - rect.height - 8)),
    })
  }, [at])

  useEffect(() => {
    ref.current?.querySelector<HTMLElement>('button')?.focus({ preventScroll: true })
    const onDown = (event: PointerEvent) => {
      if (!ref.current?.contains(event.target as Node)) onClose()
    }
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return
      event.stopPropagation()
      onClose()
    }
    document.addEventListener('pointerdown', onDown, true)
    window.addEventListener('keydown', onKey, true)
    return () => {
      document.removeEventListener('pointerdown', onDown, true)
      window.removeEventListener('keydown', onKey, true)
    }
  }, [onClose])

  return createPortal(
    <div
      ref={ref}
      role="menu"
      aria-label={t.workspaceCustomize}
      {...{ [PANEL_SCOPE_ATTR]: '' }}
      data-tauri-drag-region="false"
      className="kv-menu chat-motion-popover chat-motion-menu-cascade fixed z-[60] w-52"
      style={{ left: pos.x, top: pos.y }}
    >
      <div className="kv-menu-label px-2 pb-1 pt-0.5 text-[11px] text-neutral-400">{t.workspaceCustomize}</div>
      {WORKSPACE_ITEMS.map((id) => {
        const shown = visible[id]
        return (
          <button
            key={id}
            type="button"
            role="menuitemcheckbox"
            aria-checked={shown}
            className="kv-menu-item"
            onClick={() => onToggle(id)}
          >
            <span className="flex size-3.5 shrink-0 items-center justify-center">
              {shown && <Check size={13} strokeWidth={2} />}
            </span>
            <span className="min-w-0 flex-1 truncate">{labels[id]}</span>
          </button>
        )
      })}
    </div>,
    document.body,
  )
}
