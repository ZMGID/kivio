import { useEffect, useRef, useState, type MouseEvent, type RefObject } from 'react'
import { TextEditContextMenu } from '../settings/public/textEditing'
import { copyToClipboard } from '../utils/clipboard'
import { api } from '../api/tauri'
import { isTauriRuntime } from './utils'

export type ComposerPasteTarget = {
  isCurrent: () => boolean
  insertText: (text: string) => void
}

export function useComposerContextMenu({ textareaRef, scopeKey, readOnly, onPaste, onError }: {
  textareaRef: RefObject<HTMLTextAreaElement>
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
    if (menu?.scopeKey === current.current.scopeKey) textareaRef.current?.focus({ preventScroll: true })
  }
  const isCurrent = () => !!menu && menu.scopeKey === current.current.scopeKey
    && !current.current.readOnly && textareaRef.current?.value === menu.value
  const restoreSelection = () => {
    const el = textareaRef.current
    if (!el || !menu || menu.scopeKey !== current.current.scopeKey) return null
    el.focus({ preventScroll: true })
    el.setSelectionRange(menu.start, menu.end)
    return el
  }
  const insertText = (text: string) => {
    if (!isCurrent()) return
    const el = restoreSelection()
    if (!el) return
    // Use the editor's undo stack; a state-only replacement loses native undo history.
    if (document.execCommand?.('insertText', false, text)) return
    const next = `${el.value.slice(0, menu!.start)}${text}${el.value.slice(menu!.end)}`
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set?.call(el, next)
    el.dispatchEvent(new Event('input', { bubbles: true }))
    el.setSelectionRange(menu!.start + text.length, menu!.start + text.length)
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
    restoreSelection()
    document.execCommand?.(command)
  }
  const onContextMenu = (event: MouseEvent<HTMLTextAreaElement>) => {
    event.preventDefault()
    event.stopPropagation()
    const el = event.currentTarget
    el.focus({ preventScroll: true })
    setMenu({ left: event.clientX, top: event.clientY, start: el.selectionStart, end: el.selectionEnd,
      value: el.value, scopeKey, canUndo: document.queryCommandEnabled?.('undo') ?? false,
      canRedo: document.queryCommandEnabled?.('redo') ?? false })
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
      onSelectAll={() => { const el = textareaRef.current; el?.focus(); el?.select() }}
      onClose={close}
    /> : null,
  }
}
