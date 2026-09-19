import { useCallback, useReducer } from 'react'
import type { LensWindowInfo } from '../api/tauri'
import { DRAG_THRESHOLD } from './layout'
import type { CapturedFrame, Point } from './types'

type CaptureRect = { x: number; y: number; width: number; height: number }
type View = {
  windows: LensWindowInfo[]
  hovered: LensWindowInfo | null
  dragStart: Point | null
  dragCurrent: Point | null
  dragging: boolean
  pendingCapture: CaptureRect | null
  capturedFrame: CapturedFrame | null
  showCaptureHint: boolean
}
type Action =
  | { type: 'open'; first: boolean }
  | { type: 'hide' }
  | { type: 'windows'; windows: LensWindowInfo[] }
  | { type: 'hover'; window: LensWindowInfo | null }
  | { type: 'startDrag'; point: Point }
  | { type: 'moveDrag'; point: Point }
  | { type: 'recoverDrag'; start: Point; point: Point }
  | { type: 'clearDrag' }
  | { type: 'queueCapture'; rect: CaptureRect | null }
  | { type: 'captureFrame'; frame: CapturedFrame | null }
  | { type: 'hint'; visible: boolean }

const emptyView = (): View => ({
  windows: [], hovered: null, dragStart: null, dragCurrent: null, dragging: false,
  pendingCapture: null, capturedFrame: null, showCaptureHint: false,
})

function movedBeyondThreshold(start: Point, point: Point): boolean {
  return Math.abs(point.x - start.x) > DRAG_THRESHOLD || Math.abs(point.y - start.y) > DRAG_THRESHOLD
}

function transition(view: View, action: Action): View {
  switch (action.type) {
    case 'open': return {
      ...view, hovered: null, capturedFrame: null, showCaptureHint: false,
      ...(action.first ? {} : { dragStart: null, dragCurrent: null, dragging: false, pendingCapture: null }),
    }
    case 'hide': return emptyView()
    case 'windows': return { ...view, windows: action.windows }
    case 'hover': return { ...view, hovered: view.dragging ? null : action.window }
    case 'startDrag': return { ...view, dragStart: action.point, dragCurrent: action.point, dragging: false }
    case 'moveDrag': return {
      ...view, dragCurrent: action.point,
      dragging: view.dragging || (view.dragStart !== null && movedBeyondThreshold(view.dragStart, action.point)),
      hovered: view.dragStart !== null && movedBeyondThreshold(view.dragStart, action.point) ? null : view.hovered,
    }
    case 'recoverDrag': return {
      ...view, dragStart: action.start, dragCurrent: action.point,
      dragging: movedBeyondThreshold(action.start, action.point), hovered: null,
    }
    case 'clearDrag': return { ...view, dragStart: null, dragCurrent: null, dragging: false }
    case 'queueCapture': return { ...view, pendingCapture: action.rect }
    case 'captureFrame': return { ...view, capturedFrame: action.frame }
    case 'hint': return { ...view, showCaptureHint: action.visible }
  }
}

/** Selection gesture and pending-capture view state for a Lens opening. */
export function useLensSelectionController() {
  const [view, dispatch] = useReducer(transition, undefined, emptyView)
  const open = useCallback((first: boolean) => dispatch({ type: 'open', first }), [])
  const hide = useCallback(() => dispatch({ type: 'hide' }), [])
  const windowsDiscovered = useCallback((windows: LensWindowInfo[]) => dispatch({ type: 'windows', windows }), [])
  const hoverWindow = useCallback((window: LensWindowInfo | null) => dispatch({ type: 'hover', window }), [])
  const startDrag = useCallback((point: Point) => dispatch({ type: 'startDrag', point }), [])
  const moveDrag = useCallback((point: Point) => dispatch({ type: 'moveDrag', point }), [])
  const recoverDrag = useCallback((start: Point, point: Point) => dispatch({ type: 'recoverDrag', start, point }), [])
  const clearDrag = useCallback(() => dispatch({ type: 'clearDrag' }), [])
  const queueCapture = useCallback((rect: CaptureRect | null) => dispatch({ type: 'queueCapture', rect }), [])
  const captureFrame = useCallback((frame: CapturedFrame | null) => dispatch({ type: 'captureFrame', frame }), [])
  const showHint = useCallback((visible: boolean) => dispatch({ type: 'hint', visible }), [])
  return { view, open, hide, windowsDiscovered, hoverWindow, startDrag, moveDrag, recoverDrag, clearDrag, queueCapture, captureFrame, showHint }
}
