import { forwardRef, useImperativeHandle, useLayoutEffect, useRef, useState, type AnimationEventHandler } from 'react'
import { createPortal } from 'react-dom'
import { Schema, type Node as ProseNode } from '@milkdown/prose/model'
import { EditorState, Plugin, TextSelection } from '@milkdown/prose/state'
import { EditorView } from '@milkdown/prose/view'
import { history, undo, redo, undoDepth, redoDepth, closeHistory } from '@milkdown/prose/history'
import { keymap } from '@milkdown/prose/keymap'
import { selectAll } from '@milkdown/prose/commands'
import { findComposerCommands, type SlashCommandDefinition } from './slashCommands'
import { SlashCommandIcon } from './SlashCommandIcon'

const schema = new Schema({ nodes: {
  doc: { content: 'inline*', whitespace: 'pre' },
  text: { group: 'inline' },
  command: {
    inline: true, group: 'inline', atom: true, selectable: true,
    attrs: { token: {}, id: {} },
    leafText: node => node.attrs.token,
    toDOM: node => ['span', { 'data-command': node.attrs.id }, node.attrs.token],
  },
} })

function plainText(doc: ProseNode): string { return doc.textContent }

function documentFromText(value: string, commands: readonly SlashCommandDefinition[], editingAt?: number, existing?: ProseNode) {
  const nodes: ProseNode[] = []
  let from = 0
  const existingEnds = new Set<number>()
  if (existing) {
    let offset = 0
    existing.forEach(node => {
      offset += node.isText ? node.nodeSize : node.attrs.token.length
      if (!node.isText) existingEnds.add(offset)
    })
  }
  for (const match of findComposerCommands(value, commands)) {
    // Keep an unfinished query editable, even if it already equals a short command.
    if (match.end === editingAt && !existingEnds.has(match.end)) continue
    if (match.start > from) nodes.push(schema.text(value.slice(from, match.start)))
    nodes.push(schema.nodes.command.create({ id: match.command.id, token: value.slice(match.start, match.end) }))
    from = match.end
  }
  if (from < value.length) nodes.push(schema.text(value.slice(from)))
  return schema.nodes.doc.create(null, nodes)
}

function textOffset(doc: ProseNode, position: number): number {
  let offset = 0
  doc.forEach((node, pos) => {
    if (pos >= position) return
    offset += node.isText ? Math.min(node.nodeSize, position - pos) : node.attrs.token.length
  })
  return offset
}

function documentPosition(doc: ProseNode, offset: number, bias = 1): number {
  let textPos = 0
  let result = doc.content.size
  let found = false
  doc.forEach((node, pos) => {
    if (found) return
    const length = node.isText ? node.nodeSize : node.attrs.token.length
    if (offset <= textPos + length) {
      result = node.isText ? pos + Math.max(0, offset - textPos)
        : offset <= textPos ? pos : offset >= textPos + length || bias > 0 ? pos + 1 : pos
      found = true
    }
    textPos += length
  })
  return result
}

export interface ComposerEditorHandle {
  readonly dom: HTMLElement
  readonly value: string
  selectionStart: number
  selectionEnd: number
  focus: (options?: FocusOptions) => void
  setSelectionRange: (start: number, end: number) => void
  select: () => void
  replaceText: (start: number, end: number, text: string) => void
  undo: () => void
  redo: () => void
  readonly canUndo: boolean
  readonly canRedo: boolean
  readonly atCommand: boolean
}

type Props = {
  value: string
  scopeKey: string
  commands: readonly SlashCommandDefinition[]
  readOnly?: boolean
  busy?: boolean
  placeholder: string
  className?: string
  onChange: (value: string, cursor: number) => void
  onSelect: () => void
  onKeyDown: (event: KeyboardEvent) => void
  onPaste: (event: ClipboardEvent) => void
  onContextMenu: (event: MouseEvent) => void
  onAnimationEnd?: AnimationEventHandler<HTMLDivElement>
}

/** Owns rich editing/selection/history; InputBar still owns the serializable draft. */
export const ComposerEditor = forwardRef<ComposerEditorHandle, Props>(function ComposerEditor(props, ref) {
  const mount = useRef<HTMLDivElement>(null)
  const viewRef = useRef<EditorView | null>(null)
  const current = useRef(props)
  current.current = props
  const scope = useRef(props.scopeKey)
  const [iconMounts, setIconMounts] = useState<{ key: number; commandId: string; dom: HTMLElement }[]>([])

  useImperativeHandle(ref, () => {
    const selection = (start: number, end: number) => {
      const view = viewRef.current
      if (!view) return
      const from = documentPosition(view.state.doc, start, start === end ? 1 : -1)
      const to = documentPosition(view.state.doc, end)
      view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, from, to)))
    }
    return {
      get dom() { return viewRef.current!.dom },
      get value() { return viewRef.current ? plainText(viewRef.current.state.doc) : current.current.value },
      get selectionStart() { const v = viewRef.current; return v ? textOffset(v.state.doc, v.state.selection.from) : 0 },
      get selectionEnd() { const v = viewRef.current; return v ? textOffset(v.state.doc, v.state.selection.to) : 0 },
      set selectionStart(start: number) { selection(start, Math.max(start, this.selectionEnd)) },
      set selectionEnd(end: number) { selection(Math.min(this.selectionStart, end), end) },
      setSelectionRange: selection,
      focus(options) { viewRef.current?.dom.focus(options) },
      select() { const v = viewRef.current; if (v) selectAll(v.state, v.dispatch) },
      replaceText(start, end, text) {
        const view = viewRef.current
        if (!view || current.current.readOnly) return
        const from = documentPosition(view.state.doc, start, -1)
        const to = documentPosition(view.state.doc, end)
        const content = documentFromText(text, current.current.commands).content
        const tr = view.state.tr.replaceWith(from, to, content)
        tr.setSelection(TextSelection.create(tr.doc, from + content.size))
        view.dispatch(closeHistory(tr).scrollIntoView())
        view.dispatch(closeHistory(view.state.tr))
      },
      undo() { const v = viewRef.current; if (v && !current.current.readOnly) undo(v.state, v.dispatch) },
      redo() { const v = viewRef.current; if (v && !current.current.readOnly) redo(v.state, v.dispatch) },
      get canUndo() { return !!viewRef.current && undoDepth(viewRef.current.state) > 0 },
      get canRedo() { return !!viewRef.current && redoDepth(viewRef.current.state) > 0 },
      get atCommand() {
        const v = viewRef.current
        return !!v && (v.state.selection.$from.nodeBefore?.type === schema.nodes.command || !v.state.selection.empty)
      },
    }
  }, [])

  useLayoutEffect(() => {
    if (!mount.current) return
    let active = true
    let iconKey = 0
    setIconMounts([])
    const normalize = new Plugin({
      appendTransaction(transactions, _old, state) {
        if (viewRef.current?.composing || !transactions.some(tr => tr.docChanged || tr.getMeta('normalize'))) return null
        const start = textOffset(state.doc, state.selection.from)
        const end = textOffset(state.doc, state.selection.to)
        const doc = documentFromText(plainText(state.doc), current.current.commands, end, state.doc)
        if (doc.eq(state.doc)) return null
        const tr = state.tr.replaceWith(0, state.doc.content.size, doc.content)
        return tr.setSelection(TextSelection.create(tr.doc, documentPosition(tr.doc, start), documentPosition(tr.doc, end)))
      },
    })
    const createState = (value: string) => EditorState.create({
      doc: documentFromText(value, current.current.commands),
      plugins: [history(), normalize, keymap({
        'Mod-z': undo, 'Mod-Shift-z': redo, 'Mod-y': redo, 'Mod-a': selectAll,
        'Shift-Enter': (state, dispatch) => { dispatch?.(state.tr.insertText('\n')); return true },
      })],
    })
    const view = new EditorView(mount.current, {
      state: createState(current.current.value),
      editable: () => !current.current.readOnly,
      attributes: { role: 'textbox', 'aria-multiline': 'true', spellcheck: 'false' },
      clipboardTextSerializer: slice => slice.content.textBetween(0, slice.content.size),
      handlePaste(view, event) {
        if (current.current.readOnly) return true
        current.current.onPaste(event)
        if (event.defaultPrevented) return true
        const text = event.clipboardData?.getData('text/plain')
        if (text === undefined || !text) return false
        view.dispatch(view.state.tr.insertText(text).scrollIntoView())
        return true
      },
      handleKeyDown(_view, event) {
        current.current.onKeyDown(event)
        return event.defaultPrevented
      },
      handleDOMEvents: {
        contextmenu(_view, event) { current.current.onContextMenu(event); return event.defaultPrevented },
        compositionend() {
          queueMicrotask(() => { if (!view.isDestroyed) view.dispatch(view.state.tr.setMeta('normalize', true)) })
          return false
        },
      },
      nodeViews: {
        command(node) {
          const dom = document.createElement('span')
          dom.className = 'chat-composer-command'
          dom.contentEditable = 'false'
          dom.dataset.command = node.attrs.id
          const command = current.current.commands.find(item => item.id === node.attrs.id)
          dom.title = command?.description ?? node.attrs.token
          const icon = document.createElement('span')
          icon.className = 'chat-composer-command-icon'
          icon.setAttribute('aria-hidden', 'true')
          const key = iconKey++
          setIconMounts(items => [...items, { key, commandId: node.attrs.id, dom: icon }])
          const label = document.createElement('span')
          label.textContent = node.attrs.token
          dom.append(icon, label)
          return {
            dom,
            // React owns the icon subtree; editor parsing must ignore its updates.
            ignoreMutation: mutation => mutation.type !== 'selection',
            destroy() { if (active) setIconMounts(items => items.filter(item => item.key !== key)) },
          }
        },
      },
      dispatchTransaction(tr) {
        const old = view.state
        const next = old.applyTransaction(tr).state
        view.updateState(next)
        if (!next.doc.eq(old.doc)) current.current.onChange(plainText(next.doc), textOffset(next.doc, next.selection.from))
        else if (!next.selection.eq(old.selection)) current.current.onSelect()
      },
    })
    viewRef.current = view
    return () => { active = false; viewRef.current = null; view.destroy() }
  }, [])

  useLayoutEffect(() => {
    const view = viewRef.current
    if (!view) return
    view.setProps({ attributes: {
      role: 'textbox', 'aria-multiline': 'true', 'aria-label': props.placeholder,
      'data-placeholder': props.placeholder, 'aria-busy': String(!!props.busy),
      'aria-readonly': String(!!props.readOnly), spellcheck: 'false',
      class: `chat-composer-editor custom-scrollbar ${props.className ?? ''}`,
    } })
    if (scope.current !== props.scopeKey) {
      scope.current = props.scopeKey
      view.updateState(EditorState.create({ doc: documentFromText(props.value, props.commands), plugins: view.state.plugins }))
    } else if (props.value !== plainText(view.state.doc)) {
      const doc = documentFromText(props.value, props.commands)
      // Programmatic draft replacement (send, navigation, history recall) starts
      // a fresh undo scope, so undo cannot resurrect a sent/other-chat message.
      view.updateState(EditorState.create({ doc, plugins: view.state.plugins,
        selection: TextSelection.create(doc, doc.content.size) }))
    } else if (!view.composing) {
      const end = textOffset(view.state.doc, view.state.selection.to)
      const doc = documentFromText(props.value, props.commands, end, view.state.doc)
      if (!doc.eq(view.state.doc)) {
        const tr = view.state.tr.replaceWith(0, view.state.doc.content.size, doc.content).setMeta('addToHistory', false)
        tr.setSelection(TextSelection.create(tr.doc, documentPosition(tr.doc, end)))
        view.updateState(view.state.apply(tr))
      }
    }
    view.dom.dataset.empty = String(view.state.doc.content.size === 0)
  })

  return <>
    <div ref={mount} onAnimationEnd={props.onAnimationEnd} />
    {iconMounts.map(({ key, commandId, dom }) => {
      const command = props.commands.find(item => item.id === commandId)
      return command ? createPortal(<SlashCommandIcon command={command} size={14} />, dom, key) : null
    })}
  </>
})
