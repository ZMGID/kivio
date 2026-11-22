import { Plugin } from '@milkdown/prose/state'

// macOS WebKit paints a 2px native caret and stretches it into the leading on
// wrapped lines. Keep the real DOM selection for editing and IME positioning.
export const composerCaret = new Plugin({
  view(view) {
    const editor = view.dom
    const host = editor.parentElement
    if (!host || !/Apple Computer/.test(navigator.vendor) || !/Macintosh/.test(navigator.userAgent)
      || typeof ResizeObserver === 'undefined' || !CSS.supports('selector(:has(+ span))')) return {}
    const caret = document.createElement('span')
    caret.className = 'chat-composer-caret'
    caret.ariaHidden = 'true'
    caret.hidden = true
    host.append(caret)
    const listeners = new AbortController()
    const options = { signal: listeners.signal }
    let frame = 0

    const draw = () => {
      frame = 0
      const selection = document.getSelection()
      if (!view.hasFocus() || !document.hasFocus() || !view.editable || view.composing
        || !view.state.selection.empty || !selection?.isCollapsed || !editor.contains(selection.focusNode)) {
        caret.hidden = true
        return
      }
      const { head } = view.state.selection
      const end = view.state.doc.content.size
      // The trailing BR carries the actual empty line after a final newline.
      const rect = head === end && editor.lastChild instanceof HTMLBRElement
        ? editor.lastChild.getBoundingClientRect() : view.coordsAtPos(head)
      if (head > 0 && head < end && view.state.doc.textBetween(head - 1, head) !== '\n') {
        // A soft-wrap or bidi boundary can have two visual insertion points.
        // Only the native selection knows which side the user approached from.
        const before = view.coordsAtPos(head, -1)
        if (Math.abs(rect.top - before.top) > 1 || Math.abs(rect.left - before.left) > 1) {
          caret.hidden = true
          return
        }
      }
      const bounds = editor.getBoundingClientRect()
      if (rect.bottom <= rect.top || rect.top < bounds.top || rect.bottom > bounds.bottom) {
        caret.hidden = true
        return
      }
      const origin = host.getBoundingClientRect()
      const left = `${Math.round(rect.left - origin.left)}px`
      const top = `${Math.round(rect.top - origin.top)}px`
      const height = `${Math.round(rect.bottom - rect.top)}px`
      if (caret.hidden || left !== caret.style.left || top !== caret.style.top || height !== caret.style.height) {
        caret.style.left = left
        caret.style.top = top
        caret.style.height = height
        caret.hidden = false
        for (const animation of caret.getAnimations()) animation.currentTime = 0
      }
    }

    function schedule() {
      if (!frame && (view.hasFocus() || !caret.hidden)) frame = requestAnimationFrame(draw)
    }
    function hide() { caret.hidden = true }
    document.addEventListener('selectionchange', schedule, options)
    editor.addEventListener('focus', schedule, options)
    editor.addEventListener('blur', hide, options)
    editor.addEventListener('compositionstart', hide, options)
    editor.addEventListener('compositionend', schedule, options)
    editor.addEventListener('scroll', schedule, options)
    window.addEventListener('blur', hide, options)
    window.addEventListener('focus', schedule, options)
    const resize = new ResizeObserver(schedule)
    resize.observe(editor)
    return {
      update: schedule,
      destroy() {
        cancelAnimationFrame(frame)
        listeners.abort()
        resize.disconnect()
        caret.remove()
      },
    }
  },
})
