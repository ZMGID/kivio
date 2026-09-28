import { useEffect, useRef, useState, type RefObject } from 'react'
import type { ComposerEditorHandle } from './ComposerEditor'
import { TextEditContextMenu } from '../settings/public/textEditing'
import { copyToClipboard } from '../utils/clipboard'
import { api } from '../api/tauri'
import { isTauriRuntime } from './utils'

export type ComposerPasteTarget = {
  isCurrent: () => boolean
  insertText: (text: string) => void
}

export function useComposerContextMenu({ editorRef, scopeKey, readOnly, onPaste, onError }: {
  editorRef: RefObject<ComposerEditorHandle>
  scopeKey: string
  readOnly: boolean
  onPaste: (target: ComposerPasteTarget) => Promise<void>
  onError: (message: string) => void
}) {
  const current = useRef({ scopeKey, readOnly })
  current.current = { scopeKey, readOnly }
  const [menu, setMenu] = useState<{
    left: number; top: number; start: number; end: number; value: string; scopeKey: string
    canUndo: boolean; canRedo: boolean
  } | null>(null)
  useEffect(() => { setMenu(null) }, [scopeKey])

  const close = () => {
    setMenu(null)
    if (menu?.scopeKey === current.current.scopeKey) editorRef.current?.focus({ preventScroll: true })
  }
  const isCurrent = () => !!menu && menu.scopeKey === current.current.scopeKey
    && !current.current.readOnly && editorRef.current?.value === menu.value
  const restoreSelection = () => {
    const el = editorRef.current
    if (!el || !menu || menu.scopeKey !== current.current.scopeKey) return null
    el.focus({ preventScroll: true })
    el.setSelectionRange(menu.start, menu.end)
    return el
  }
  const insertText = (text: string) => {
    if (!isCurrent()) return
    const el = restoreSelection()
    if (!el) return
    el.replaceText(menu!.start, menu!.end, text)
  }
  const copy = async (cut: boolean) => {
    if (!menu || menu.scopeKey !== current.current.scopeKey) return
    const selected = menu.value.slice(menu.start, menu.end)
    if (!selected) return
    try {
      if (isTauriRuntime()) await api.chatWriteClipboardText(selected)
      else if (!await copyToClipboard(selected)) throw new Error('复制失败')
      if (cut) insertText('')
    } catch {
      onError('无法写入剪贴板，请重试。')
    }
  }
  const history = (command: 'undo' | 'redo') => {
    if (!isCurrent()) return
    restoreSelection()?.[command]()
  }
  const onContextMenu = (event: MouseEvent) => {
    event.preventDefault()
    event.stopPropagation()
    const el = editorRef.current
    if (!el) return
    el.focus({ preventScroll: true })
    setMenu({ left: event.clientX, top: event.clientY, start: el.selectionStart, end: el.selectionEnd,
      value: el.value, scopeKey, canUndo: el.canUndo, canRedo: el.canRedo })
  }
  return {
    onContextMenu,
    menu: menu && menu.scopeKey === scopeKey ? <TextEditContextMenu
      anchor={menu} hasSelection={menu.end > menu.start} readOnly={readOnly}
      canUndo={menu.canUndo} canRedo={menu.canRedo}
      onUndo={() => history('undo')} onRedo={() => history('redo')}
      onCopy={() => { void copy(false) }} onCut={() => { void copy(true) }}
      onPaste={() => { void onPaste({ isCurrent, insertText }).catch(() => {
        if (isCurrent()) onError('无法读取剪贴板，请重试或使用 Ctrl+V。')
      }) }}
      onSelectAll={() => { const el = editorRef.current; el?.focus(); el?.select() }}
      onClose={close}
    /> : null,
  }
}
