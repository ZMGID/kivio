import { useCallback, useReducer } from 'react'
import { ARROW_MIN_DRAG_PX } from './annotation'
import type { Annotation, AnnotationKind, Stage } from './types'

type View = {
  drawMode: boolean
  arrows: Annotation[]
  draft: Annotation | null
  tool: AnnotationKind
  copied: boolean
  saving: boolean
}
type Action =
  | { type: 'hide' }
  | { type: 'leaveReady' }
  | { type: 'toggleDraw' }
  | { type: 'exitDraw' }
  | { type: 'tool'; tool: AnnotationKind }
  | { type: 'begin'; kind: AnnotationKind; x: number; y: number }
  | { type: 'move'; x: number; y: number }
  | { type: 'finish' }
  | { type: 'cancelDraft' }
  | { type: 'undo' }
  | { type: 'clearSubmitted' }
  | { type: 'copied'; value: boolean }
  | { type: 'saving'; value: boolean }

const emptyView = (): View => ({
  drawMode: false, arrows: [], draft: null, tool: 'arrow', copied: false, saving: false,
})

function transition(view: View, action: Action): View {
  switch (action.type) {
    case 'hide': return emptyView()
    case 'leaveReady': return { ...view, drawMode: false, arrows: [], draft: null, copied: false, saving: false }
    case 'toggleDraw': return { ...view, drawMode: !view.drawMode }
    case 'exitDraw': return { ...view, drawMode: false, draft: null }
    case 'tool': return { ...view, tool: action.tool }
    case 'begin': return { ...view, draft: { kind: action.kind, x1: action.x, y1: action.y, x2: action.x, y2: action.y } }
    case 'move': return view.draft
      ? { ...view, draft: { ...view.draft, x2: action.x, y2: action.y } }
      : view
    case 'finish': {
      const draft = view.draft
      if (!draft) return view
      const length = Math.hypot(draft.x2 - draft.x1, draft.y2 - draft.y1)
      return {
        ...view, draft: null,
        arrows: length >= ARROW_MIN_DRAG_PX ? [...view.arrows, draft] : view.arrows,
      }
    }
    case 'cancelDraft': return { ...view, draft: null }
    case 'undo': return { ...view, arrows: view.arrows.slice(0, -1), draft: null }
    case 'clearSubmitted': return { ...view, arrows: [], draft: null, drawMode: false }
    case 'copied': return { ...view, copied: action.value }
    case 'saving': return { ...view, saving: action.value }
  }
}

/** Owns drawing lifecycle, completion threshold and session reset atomically. */
export function useLensAnnotationController() {
  const [view, dispatch] = useReducer(transition, undefined, emptyView)
  const hide = useCallback(() => dispatch({ type: 'hide' }), [])
  const stageChanged = useCallback((stage: Stage) => {
    if (stage !== 'ready') dispatch({ type: 'leaveReady' })
  }, [])
  const toggleDraw = useCallback(() => dispatch({ type: 'toggleDraw' }), [])
  const exitDraw = useCallback(() => dispatch({ type: 'exitDraw' }), [])
  const selectTool = useCallback((tool: AnnotationKind) => dispatch({ type: 'tool', tool }), [])
  const begin = useCallback((kind: AnnotationKind, x: number, y: number) => dispatch({ type: 'begin', kind, x, y }), [])
  const move = useCallback((x: number, y: number) => dispatch({ type: 'move', x, y }), [])
  const finish = useCallback(() => dispatch({ type: 'finish' }), [])
  const cancelDraft = useCallback(() => dispatch({ type: 'cancelDraft' }), [])
  const undo = useCallback(() => dispatch({ type: 'undo' }), [])
  const clearSubmitted = useCallback(() => dispatch({ type: 'clearSubmitted' }), [])
  const setCopied = useCallback((value: boolean) => dispatch({ type: 'copied', value }), [])
  const setSaving = useCallback((value: boolean) => dispatch({ type: 'saving', value }), [])
  return { view, hide, stageChanged, toggleDraw, exitDraw, selectTool, begin, move, finish, cancelDraft, undo, clearSubmitted, setCopied, setSaving }
}
