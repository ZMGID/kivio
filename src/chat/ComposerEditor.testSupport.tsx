// InputBar unit tests exercise send/attachment lifecycles through the editor's
// public interface. Rich editing itself is covered with the real editor.
import { forwardRef, useImperativeHandle, useRef, type ComponentProps } from 'react'
import type { ComposerEditor as Editor, ComposerEditorHandle } from './ComposerEditor'

export const ComposerEditor = forwardRef<ComposerEditorHandle, ComponentProps<typeof Editor>>(function MockEditor(props, ref) {
  const input = useRef<HTMLTextAreaElement>(null)
  useImperativeHandle(ref, () => ({
    get dom() { return input.current! },
    get value() { return input.current!.value },
    get selectionStart() { return input.current!.selectionStart },
    get selectionEnd() { return input.current!.selectionEnd },
    set selectionStart(value) { input.current!.selectionStart = value },
    set selectionEnd(value) { input.current!.selectionEnd = value },
    focus(options) { input.current!.focus(options) },
    setSelectionRange(start, end) { input.current!.setSelectionRange(start, end) },
    select() { input.current!.select() },
    replaceText(start, end, text) {
      const el = input.current!
      el.setRangeText(text, start, end, 'end')
      props.onChange(el.value, el.selectionStart)
    },
    undo() { document.execCommand?.('undo') }, redo() { document.execCommand?.('redo') },
    get canUndo() { return document.queryCommandEnabled?.('undo') ?? false },
    get canRedo() { return document.queryCommandEnabled?.('redo') ?? false },
    atCommand: false,
  }))
  return <textarea ref={input} value={props.value} placeholder={props.placeholder}
    readOnly={props.readOnly} aria-busy={props.busy} className={props.className}
    onChange={event => props.onChange(event.target.value, event.target.selectionStart)}
    onSelect={props.onSelect} onKeyDown={event => props.onKeyDown(event.nativeEvent)}
    onPaste={event => props.onPaste(event.nativeEvent)}
    onContextMenu={event => props.onContextMenu(event.nativeEvent)} />
})
