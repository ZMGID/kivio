import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { ClipboardPaste, Copy, Redo2, Scissors, TextSelect, Undo2 } from 'lucide-react'
import { useT } from '../components/i18n'

export type TextEditAnchor = {
  left: number
  top: number
}

export function TextEditContextMenu({
  anchor,
  hasSelection,
  onCut,
  onCopy,
  onPaste,
  onSelectAll,
  onClose,
  readOnly = false,
  onUndo,
  onRedo,
  canUndo = false,
  canRedo = false,
}: {
  anchor: TextEditAnchor
  hasSelection: boolean
  onCut: () => void
  onCopy: () => void
  onPaste: () => void
  onSelectAll: () => void
  onClose: () => void
  readOnly?: boolean
  onUndo?: () => void
  onRedo?: () => void
  canUndo?: boolean
  canRedo?: boolean
}) {
  const t = useT()
  const menuRef = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState(anchor)

  useLayoutEffect(() => {
    const el = menuRef.current
    if (!el) return
    const rect = el.getBoundingClientRect()
    const margin = 8
    setPos({
      left: Math.max(margin, Math.min(anchor.left, window.innerWidth - rect.width - margin)),
      top: Math.max(margin, Math.min(anchor.top, window.innerHeight - rect.height - margin)),
    })
    el.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus({ preventScroll: true })
  }, [anchor.left, anchor.top])

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

  const run = (action: () => void) => {
    onClose()
    action()
  }

  return createPortal(
    <div
      ref={menuRef}
      className="kv-menu chat-motion-popover fixed z-[1000] min-w-[168px]"
      style={{ left: pos.left, top: pos.top }}
      role="menu"
      data-tauri-drag-region="false"
      onContextMenu={(event) => { event.preventDefault(); event.stopPropagation() }}
      onKeyDown={(event) => {
        if (event.key === 'Escape') {
          event.preventDefault()
          event.stopPropagation()
          onClose()
          return
        }
        if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return
        event.preventDefault()
        const items = Array.from(menuRef.current?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)') ?? [])
        const index = items.indexOf(document.activeElement as HTMLButtonElement)
        const next = event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1
          : (index + (event.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length
        items[next]?.focus()
      }}
    >
      <button
        type="button"
        role="menuitem"
        className="kv-menu-item"
        onClick={() => run(onPaste)}
        disabled={readOnly}
      >
        <ClipboardPaste strokeWidth={1.75} />
        {t.editPaste}
      </button>
      <button
        type="button"
        role="menuitem"
        className="kv-menu-item"
        onClick={() => run(onSelectAll)}
      >
        <TextSelect strokeWidth={1.75} />
        {t.editSelectAll}
      </button>
      {onUndo && <button type="button" role="menuitem" className="kv-menu-item" disabled={readOnly || !canUndo} onClick={() => run(onUndo)}>
        <Undo2 strokeWidth={1.75} />{t.editUndo}
      </button>}
      {onRedo && <button type="button" role="menuitem" className="kv-menu-item" disabled={readOnly || !canRedo} onClick={() => run(onRedo)}>
        <Redo2 strokeWidth={1.75} />{t.editRedo}
      </button>}
      <button
        type="button"
        role="menuitem"
        className="kv-menu-item"
        disabled={readOnly || !hasSelection}
        onClick={() => run(onCut)}
      >
        <Scissors strokeWidth={1.75} />
        {t.editCut}
      </button>
      <button
        type="button"
        role="menuitem"
        className="kv-menu-item"
        disabled={!hasSelection}
        onClick={() => run(onCopy)}
      >
        <Copy strokeWidth={1.75} />
        {t.editCopy}
      </button>
    </div>,
    document.body,
  )
}
