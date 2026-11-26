import { forwardRef, useEffect, useImperativeHandle, useLayoutEffect, useRef, useState, type AnimationEventHandler } from 'react'
import { Schema, type Node as ProseNode } from '@milkdown/prose/model'
import { EditorState, NodeSelection, Plugin, TextSelection, type Transaction } from '@milkdown/prose/state'
import { Decoration, DecorationSet, EditorView } from '@milkdown/prose/view'
import { history, undo, redo, undoDepth, redoDepth, closeHistory } from '@milkdown/prose/history'
import { keymap } from '@milkdown/prose/keymap'
import { deleteSelection, selectAll } from '@milkdown/prose/commands'
import { composerCaret } from './composerCaret'
import { findComposerCommands, type SlashCommandDefinition } from './slashCommands'
import { formatBytes } from '../utils/formatBytes'
import type { ComposerAttachment } from './composerDraft'
import type { PendingAttachment } from './types'

const schema = new Schema({ nodes: {
  doc: { content: 'inline*', whitespace: 'pre' },
  text: { group: 'inline' },
  command: {
    inline: true, group: 'inline', atom: true, selectable: true,
    attrs: { token: {}, id: {} },
    leafText: node => node.attrs.token,
    toDOM: node => ['span', { 'data-command': node.attrs.id }, node.attrs.token],
  },
  attachment: {
    inline: true, group: 'inline', atom: true, selectable: true, draggable: false,
    attrs: { attachment: {} },
    leafText: node => node.attrs.attachment.name,
    toDOM: node => ['span', node.attrs.attachment.name],
  },
} })

function attachmentIn(node: ProseNode): PendingAttachment | undefined {
  return node.type === schema.nodes.attachment ? node.attrs.attachment : undefined
}

function plainText(doc: ProseNode): string {
  return doc.textBetween(0, doc.content.size, '', node => attachmentIn(node) ? '' : node.attrs.token)
}

function textLength(node: ProseNode, countAttachments: boolean): number {
  if (node.isText) return node.nodeSize
  if (node.type === schema.nodes.attachment) return countAttachments ? 1 : 0
  return node.attrs.token.length
}

function documentAttachments(doc: ProseNode): ComposerAttachment[] {
  const attachments: ComposerAttachment[] = []
  let offset = 0
  doc.forEach(node => {
    const attachment = attachmentIn(node)
    if (attachment) attachments.push({ ...attachment, offset })
    offset += textLength(node, false)
  })
  return attachments
}

function documentFromText(
  value: string, commands: readonly SlashCommandDefinition[],
  attachments: readonly ComposerAttachment[] = [], editingAt?: number, existing?: ProseNode,
) {
  const nodes: ProseNode[] = []
  const placements = attachments.map(({ offset, ...attachment }) => ({
    offset: Math.min(offset ?? value.length, value.length), attachment,
  })).sort((a, b) => a.offset - b.offset)
  // A space at each chip boundary keeps a command from spanning an attachment.
  let source = ''
  let from = 0
  const slots = placements.map(({ offset, attachment }) => {
    source += value.slice(from, offset) + ' '
    from = offset
    return { start: source.length - 1, node: schema.nodes.attachment.create({ attachment }) }
  })
  source += value.slice(from)
  from = 0
  const existingEnds = new Set<number>()
  if (existing) {
    let offset = 0
    existing.forEach(node => {
      offset += textLength(node, true)
      if (node.type === schema.nodes.command) existingEnds.add(offset)
    })
  }
  for (const match of findComposerCommands(source, commands)) {
    // Keep an unfinished query editable, even if it already equals a short command.
    if (match.end === editingAt && !existingEnds.has(match.end)) continue
    slots.push({ start: match.start, node: schema.nodes.command.create({
      id: match.command.id, token: source.slice(match.start, match.end),
    }) })
  }
  for (const { start, node } of slots.sort((a, b) => a.start - b.start)) {
    if (start > from) nodes.push(schema.text(source.slice(from, start)))
    nodes.push(node)
    from = start + textLength(node, true)
  }
  if (from < source.length) nodes.push(schema.text(source.slice(from)))
  return schema.nodes.doc.create(null, nodes)
}

function textOffset(doc: ProseNode, position: number, countAttachments = false): number {
  let offset = 0
  doc.forEach((node, pos) => {
    if (pos >= position) return
    offset += node.isText ? Math.min(node.nodeSize, position - pos) : textLength(node, countAttachments)
  })
  return offset
}

function documentPosition(
  doc: ProseNode, offset: number,
  options: { bias?: -1 | 1; countAttachments?: boolean; attachmentBias?: -1 | 1 } = {},
): number {
  const { bias = 1, countAttachments = false, attachmentBias = bias } = options
  let textPos = 0
  let result = doc.content.size
  let found = false
  doc.forEach((node, pos) => {
    if (found) return
    const length = textLength(node, countAttachments)
    if (offset === textPos + length && attachmentBias > 0) {
      textPos += length
      return
    }
    if (offset <= textPos + length) {
      result = node.isText ? pos + Math.max(0, offset - textPos)
        : offset <= textPos ? pos : offset >= textPos + length || bias > 0 ? pos + 1 : pos
      found = true
    }
    textPos += length
  })
  return result
}

function updateDocument(tr: Transaction, doc: ProseNode) {
  const start = tr.doc.content.findDiffStart(doc.content)
  const end = tr.doc.content.findDiffEnd(doc.content)
  if (start === null || end === null) return null
  const overlap = Math.max(0, start - Math.min(end.a, end.b))
  return tr.replaceWith(start, end.a + overlap, doc.slice(start, end.b + overlap).content)
}

export type AttachmentInsertion = {
  insert: (attachments: readonly PendingAttachment[]) => boolean
  release: () => void
}

type AttachmentInsertionAnchor = { from: number; to: number }

const attachmentInsertionMeta = 'composerAttachmentInsertion'

function mapAttachmentInsertions(tr: Transaction, pending: ReadonlySet<AttachmentInsertionAnchor>) {
  const inserted: unknown = tr.getMeta(attachmentInsertionMeta)
  let followsInsertion = false
  for (const anchor of pending) {
    if (anchor === inserted) {
      followsInsertion = true
      continue
    }
    for (const map of tr.mapping.maps) {
      const { from, to } = anchor
      let replaceSelection = from < to
      // Keep later typing, including edits that reuse part of the selected text.
      if (replaceSelection) map.forEach((start, end) => {
        if (start <= to && end >= from) replaceSelection = false
      })
      // Stay before typing at the paste point and after earlier pastes there.
      anchor.from = map.map(from, from < to || followsInsertion ? 1 : -1)
      anchor.to = replaceSelection ? map.map(to, -1) : anchor.from
    }
  }
}

export interface ComposerEditorHandle {
  readonly dom: HTMLElement
  readonly value: string
  readonly attachments: ComposerAttachment[] | undefined
  readonly selectedText: string
  selectionStart: number
  selectionEnd: number
  focus: (options?: FocusOptions) => void
  setSelectionRange: (start: number, end: number) => void
  select: () => void
  replaceText: (start: number, end: number, text: string) => void
  captureAttachmentInsertion: (options?: { replaceSelection: boolean }) => AttachmentInsertion | undefined
  undo: () => void
  redo: () => void
  readonly canUndo: boolean
  readonly canRedo: boolean
  readonly atCommand: boolean
  atVerticalBoundary: (direction: 'up' | 'down') => boolean
}

type Props = {
  value: string
  scopeKey: string
  commands: readonly SlashCommandDefinition[]
  attachments?: readonly ComposerAttachment[]
  imageThumbnails?: ReadonlyMap<string, string>
  readOnly?: boolean
  busy?: boolean
  placeholder: string
  className?: string
  onChange: (value: string, cursor: number, attachments: ComposerAttachment[]) => void
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
  const pendingInsertions = useRef(new Set<AttachmentInsertionAnchor>())
  const [, finishComposition] = useState(0)

  useImperativeHandle(ref, () => {
    const selection = (start: number, end: number) => {
      const view = viewRef.current
      if (!view) return
      const from = documentPosition(view.state.doc, start, { bias: start === end ? 1 : -1 })
      const to = documentPosition(view.state.doc, end)
      view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, from, to)))
    }
    return {
      get dom() { return viewRef.current!.dom },
      get value() { return viewRef.current ? plainText(viewRef.current.state.doc) : current.current.value },
      get attachments() { const v = viewRef.current; return v ? documentAttachments(v.state.doc) : undefined },
      get selectedText() {
        const v = viewRef.current
        return v ? v.state.doc.textBetween(v.state.selection.from, v.state.selection.to) : ''
      },
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
        const { doc, selection } = view.state
        const matchesSelection = start === textOffset(doc, selection.from) && end === textOffset(doc, selection.to)
        const from = matchesSelection ? selection.from : documentPosition(doc, start, { bias: -1, attachmentBias: 1 })
        const to = matchesSelection ? selection.to : start === end ? from : documentPosition(doc, end, { attachmentBias: -1 })
        const content = documentFromText(text, current.current.commands).content
        const tr = view.state.tr.replaceWith(from, to, content)
        tr.setSelection(TextSelection.create(tr.doc, from + content.size))
        view.dispatch(closeHistory(tr).scrollIntoView())
        view.dispatch(closeHistory(view.state.tr))
      },
      captureAttachmentInsertion({ replaceSelection } = { replaceSelection: false }) {
        const view = viewRef.current
        if (!view) return
        const pending = pendingInsertions.current
        const { selection } = view.state
        const domSelection = view.dom.ownerDocument.getSelection()
        const focus = domSelection?.focusNode
        const start = domSelection?.anchorNode
        // A paste can follow a native caret move before ProseMirror observes it.
        let head = selection.head
        let tail = selection.anchor
        if (view.hasFocus() && domSelection && focus && start && view.dom.contains(focus) && view.dom.contains(start)) {
          head = view.posAtDOM(focus, domSelection.focusOffset)
          tail = view.posAtDOM(start, domSelection.anchorOffset)
        }
        const from = replaceSelection ? Math.min(head, tail) : head
        const to = replaceSelection ? Math.max(head, tail) : head
        const anchor = { from, to }
        pending.add(anchor)
        return {
          insert(attachments) {
            if (!pending.has(anchor) || view.isDestroyed || !view.editable || scope.current !== current.current.scopeKey) return false
            const nodes = attachments.map(attachment => schema.nodes.attachment.create({ attachment }))
            const { selection } = view.state
            const { from, to } = anchor
            const tr = view.state.tr.replaceWith(from, to, nodes).setMeta(attachmentInsertionMeta, anchor)
            if (selection.from === from && selection.to === to) {
              tr.setSelection(TextSelection.create(tr.doc, from + nodes.length))
            }
            view.dispatch(tr)
            return true
          },
          release() { pending.delete(anchor) },
        }
      },
      undo() { const v = viewRef.current; if (v && !current.current.readOnly) undo(v.state, v.dispatch) },
      redo() { const v = viewRef.current; if (v && !current.current.readOnly) redo(v.state, v.dispatch) },
      get canUndo() { return !!viewRef.current && undoDepth(viewRef.current.state) > 0 },
      get canRedo() { return !!viewRef.current && redoDepth(viewRef.current.state) > 0 },
      get atCommand() {
        const v = viewRef.current
        const before = v?.state.selection.$from.nodeBefore
        return !!v && ((!!before && !before.isText) || !v.state.selection.empty)
      },
      atVerticalBoundary(direction) {
        const view = viewRef.current
        if (!view) return false
        // A second key can arrive before ProseMirror observes the native selection change.
        const selection = view.dom.ownerDocument.getSelection()
        if (!selection?.isCollapsed || !selection.focusNode || !view.dom.contains(selection.focusNode)) return false
        const end = view.state.doc.content.size
        const caret = view.posAtDOM(selection.focusNode, selection.focusOffset)
        if (caret === (direction === 'up' ? 0 : end)) return true
        if (plainText(view.state.doc).includes('\n')) return false
        const first = view.coordsAtPos(0)
        const last = view.coordsAtPos(end, -1)
        return first.bottom > last.top
      },
    }
  }, [])

  useLayoutEffect(() => {
    if (!mount.current) return
    let compositionJustEnded = false
    let compositionTimer: ReturnType<typeof setTimeout> | undefined
    const normalize = new Plugin({
      appendTransaction(transactions, _old, state) {
        if (viewRef.current?.composing || !transactions.some(tr => tr.docChanged || tr.getMeta('normalize'))) return null
        const anchor = textOffset(state.doc, state.selection.anchor, true)
        const head = textOffset(state.doc, state.selection.head, true)
        const doc = documentFromText(plainText(state.doc), current.current.commands, documentAttachments(state.doc), head, state.doc)
        const tr = updateDocument(state.tr, doc)
        if (!tr) return null
        const from = documentPosition(tr.doc, anchor, { countAttachments: true })
        const to = documentPosition(tr.doc, head, { countAttachments: true })
        return tr.setSelection(TextSelection.create(tr.doc, from, to))
      },
    })
    const createState = (value: string) => EditorState.create({
      doc: documentFromText(value, current.current.commands, current.current.attachments),
      plugins: [history(), normalize, composerCaret, keymap({
        'Mod-z': undo, 'Mod-Shift-z': redo, 'Mod-y': redo, 'Mod-a': selectAll,
        // AllSelection cannot use the browser's native deletion fallback.
        'Backspace': deleteSelection, 'Delete': deleteSelection,
        'Shift-Enter': (state, dispatch) => { dispatch?.(state.tr.insertText('\n').scrollIntoView()); return true },
      })],
    })
    const view = new EditorView(mount.current, {
      state: createState(current.current.value),
      editable: () => !current.current.readOnly,
      attributes: { role: 'textbox', 'aria-multiline': 'true', spellcheck: 'false' },
      decorations(state) {
        const labels: Decoration[] = []
        state.doc.forEach((node, pos) => {
          const attachment = attachmentIn(node)
          if (!attachment) return
          const selected = state.selection.from <= pos && state.selection.to >= pos + node.nodeSize
          const thumbnail = current.current.imageThumbnails?.get(attachment.id)
          labels.push(Decoration.node(pos, pos + node.nodeSize, selected ? { class: 'is-selected' } : {}, { thumbnail }))
        })
        return DecorationSet.create(state.doc, labels)
      },
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
      handleDoubleClickOn(view, _pos, node, nodePos) {
        if (node.type !== schema.nodes.attachment) return false
        view.dispatch(view.state.tr.setSelection(NodeSelection.create(view.state.doc, nodePos)))
        return true
      },
      handleDOMEvents: {
        keydown(view, event) {
          // Handle send before ProseMirror's 500ms Safari composition heuristic.
          // Candidate confirmation must still stay with the native IME.
          if (event.key !== 'Enter' || event.shiftKey || current.current.readOnly
            || view.composing || event.isComposing || event.keyCode === 229 || compositionJustEnded) return false
          current.current.onKeyDown(event)
          return event.defaultPrevented
        },
        keyup() { compositionJustEnded = false; return false },
        contextmenu(_view, event) { current.current.onContextMenu(event); return event.defaultPrevented },
        compositionstart(view) {
          // Preedit text is visible before the draft updates; hide the overlay now.
          view.dom.dataset.empty = 'false'
          return false
        },
        compositionend() {
          compositionJustEnded = true
          clearTimeout(compositionTimer)
          // Let ProseMirror flush the final IME DOM mutation before normalization.
          compositionTimer = setTimeout(() => {
            compositionJustEnded = false
            if (view.isDestroyed) return
            view.dispatch(view.state.tr.setMeta('normalize', true))
            finishComposition(revision => revision + 1)
          }, 0)
          return false
        },
      },
      nodeViews: {
        attachment(node, _view, _getPos, decorations) {
          const dom = document.createElement('span')
          dom.className = 'chat-composer-attachment-label'
          dom.contentEditable = 'false'
          const type = attachmentIn(node)?.type
          const text = document.createTextNode('')
          const image = type === 'image' ? document.createElement('img') : null
          const size = image ? document.createElement('span') : null
          if (image && size) {
            const preview = document.createElement('span')
            preview.className = 'chat-composer-attachment-preview'
            const name = document.createElement('span')
            name.className = 'chat-composer-attachment-name'
            name.append(text)
            size.className = 'chat-composer-attachment-size'
            dom.classList.add('is-image')
            image.alt = ''
            image.draggable = false
            image.hidden = true
            image.onload = () => { image.hidden = false }
            preview.append(image)
            dom.append(preview, name, size)
          } else {
            dom.append(text)
          }
          const update = (next: ProseNode, labels: readonly Decoration[]) => {
            const attachment = attachmentIn(next)
            if (!attachment || attachment.type !== type) return false
            if (dom.title !== attachment.name) dom.title = attachment.name
            if (text.data !== attachment.name) text.data = attachment.name
            if (size) {
              const formatted = formatBytes(attachment.sizeBytes)
              const label = formatted ? ` ${formatted}` : ''
              if (size.textContent !== label) size.textContent = label
            }
            if (image) {
              const thumbnail = labels[0]?.spec.thumbnail ?? null
              if (thumbnail !== image.getAttribute('src')) {
                image.hidden = true
                if (thumbnail) image.src = thumbnail
                else image.removeAttribute('src')
              }
            }
            return true
          }
          update(node, decorations)
          return { dom, update }
        },
        command(node) {
          const dom = document.createElement('span')
          dom.className = 'chat-composer-command'
          dom.contentEditable = 'false'
          dom.dataset.command = node.attrs.id
          const command = current.current.commands.find(item => item.id === node.attrs.id)
          dom.title = command?.description ?? node.attrs.token
          dom.textContent = node.attrs.token
          return { dom }
        },
      },
      dispatchTransaction(tr) {
        const old = view.state
        const attachments = tr.docChanged ? documentAttachments(tr.doc) : undefined
        const previous = attachments ? documentAttachments(old.doc) : []
        const attachmentChanged = !!attachments && (attachments.length !== previous.length
          || attachments.some((attachment, index) => attachment.id !== previous[index]?.id))
        if (attachmentChanged) closeHistory(tr)
        const { state: next, transactions } = old.applyTransaction(tr)
        for (const transaction of transactions) mapAttachmentInsertions(transaction, pendingInsertions.current)
        view.updateState(attachmentChanged ? next.apply(closeHistory(next.tr)) : next)
        if (!next.doc.eq(old.doc)) {
          current.current.onChange(plainText(next.doc), textOffset(next.doc, next.selection.from),
            attachments ?? documentAttachments(next.doc))
        } else if (!next.selection.eq(old.selection)) current.current.onSelect()
      },
    })
    viewRef.current = view
    const pending = pendingInsertions.current
    return () => { clearTimeout(compositionTimer); pending.clear(); viewRef.current = null; view.destroy() }
  }, [])

  useLayoutEffect(() => {
    const view = viewRef.current
    if (!view || (view.composing && scope.current === props.scopeKey)) return
    view.setProps({ attributes: {
      role: 'textbox', 'aria-multiline': 'true', 'aria-label': props.placeholder,
      'data-placeholder': props.placeholder, 'aria-busy': String(!!props.busy),
      'aria-readonly': String(!!props.readOnly), spellcheck: 'false',
      class: `chat-composer-editor custom-scrollbar ${props.className ?? ''}`,
    } })
    if (scope.current !== props.scopeKey) {
      scope.current = props.scopeKey
      pendingInsertions.current.clear()
      view.updateState(EditorState.create({
        doc: documentFromText(props.value, props.commands, props.attachments), plugins: view.state.plugins,
      }))
    } else if (props.value !== plainText(view.state.doc)
      || (!props.value && !props.attachments?.length && view.state.doc.content.size > 0)) {
      pendingInsertions.current.clear()
      const doc = documentFromText(props.value, props.commands, props.attachments)
      // Programmatic draft replacement (send, navigation, history recall) starts
      // a fresh undo scope, so undo cannot resurrect a sent/other-chat message.
      view.updateState(EditorState.create({ doc, plugins: view.state.plugins,
        selection: TextSelection.create(doc, doc.content.size) }))
    } else if (!view.composing) {
      const end = textOffset(view.state.doc, view.state.selection.to, true)
      const doc = documentFromText(props.value, props.commands, props.attachments, end, view.state.doc)
      const tr = updateDocument(view.state.tr, doc)
      if (tr) {
        tr.setMeta('addToHistory', false)
        mapAttachmentInsertions(tr, pendingInsertions.current)
        view.updateState(view.state.apply(tr))
      }
    }
    view.dom.dataset.empty = String(view.state.doc.content.size === 0)
  })

  // 输入框随行数长高 / 变矮时，外层平滑过渡到编辑器的自然高度，而不是逐行跳。
  // 只在过渡期间裁剪溢出（不常驻 overflow:hidden，免得裁掉聚焦光环和优化动画）。
  // 隐藏（高度 0）或刚显示时直接到位，不从 0 长出来。
  useEffect(() => {
    const host = mount.current
    const editor = viewRef.current?.dom
    if (!host || !editor || typeof ResizeObserver === 'undefined') return
    let height = editor.offsetHeight
    if (height > 0) host.style.height = `${height}px`
    const settle = (event: TransitionEvent) => {
      if (event.target === host && event.propertyName === 'height') host.classList.remove('is-resizing')
    }
    const observer = new ResizeObserver(() => {
      const next = editor.offsetHeight
      if (next === height) return
      const instant = height === 0 || next === 0
      height = next
      if (next === 0) {
        host.style.height = ''
        host.classList.remove('is-resizing')
        return
      }
      if (instant) {
        host.classList.add('is-instant')
        host.style.height = `${next}px`
        void host.offsetHeight
        host.classList.remove('is-instant')
        return
      }
      host.classList.add('is-resizing')
      host.style.height = `${next}px`
    })
    observer.observe(editor)
    host.addEventListener('transitionend', settle)
    host.addEventListener('transitioncancel', settle)
    return () => {
      observer.disconnect()
      host.removeEventListener('transitionend', settle)
      host.removeEventListener('transitioncancel', settle)
      host.style.height = ''
      host.classList.remove('is-resizing')
    }
  }, [])

  return <div ref={mount} className="chat-composer-editor-host" onAnimationEnd={props.onAnimationEnd} />
})
