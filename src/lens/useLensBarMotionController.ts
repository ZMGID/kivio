import { useCallback, useReducer } from 'react'
import type { BarRect } from './types'
import { FLOATING_PADDING } from './layout'

type View = {
  rect: BarRect
  intro: boolean
  noTransition: boolean
  delta: { x: number; y: number }
  floatingRebased: boolean
  cardDragging: boolean
  cardResizing: boolean
  cardHeight: number
}
type Action =
  | { type: 'reset'; rect: BarRect }
  | { type: 'snap'; rect: BarRect }
  | { type: 'fly'; rect: BarRect; intro: boolean }
  | { type: 'settleFly' }
  | { type: 'reveal' }
  | { type: 'floatingMac'; width: number; translate: boolean }
  | { type: 'floatingWindows'; rect: BarRect; translate: boolean }
  | { type: 'floatingRebased'; value: boolean }
  | { type: 'floatingAbort'; rect: BarRect }
  | { type: 'intro'; value: boolean }
  | { type: 'transition'; disabled: boolean }
  | { type: 'cardDrag'; active: boolean }
  | { type: 'cardMove'; rect: BarRect }
  | { type: 'cardResize'; active: boolean }
  | { type: 'cardSize'; width: number; height: number }

const zero = () => ({ x: 0, y: 0 })
const initialView = (rect: BarRect): View => ({
  rect, intro: false, noTransition: true, delta: zero(), floatingRebased: false,
  cardDragging: false, cardResizing: false, cardHeight: 0,
})

function transition(view: View, action: Action): View {
  switch (action.type) {
    case 'reset': return initialView(action.rect)
    case 'snap': return { ...view, rect: action.rect }
    case 'fly': return {
      ...view, rect: action.rect, intro: action.intro, noTransition: true,
      delta: { x: view.rect.x - action.rect.x, y: view.rect.y - action.rect.y },
    }
    case 'settleFly': return { ...view, delta: zero(), noTransition: false }
    case 'reveal': return { ...view, intro: true, noTransition: false }
    case 'floatingMac': return {
      ...view, floatingRebased: false,
      rect: { x: FLOATING_PADDING, y: FLOATING_PADDING, width: action.width },
      delta: zero(), intro: !action.translate, noTransition: action.translate,
    }
    case 'floatingWindows': return {
      ...view, floatingRebased: false, rect: action.rect,
      delta: { x: view.rect.x - action.rect.x, y: view.rect.y - action.rect.y },
      intro: !action.translate, noTransition: true,
    }
    case 'floatingRebased': return { ...view, floatingRebased: action.value }
    case 'floatingAbort': return {
      ...view, floatingRebased: false, noTransition: true, rect: action.rect,
      delta: zero(), intro: true,
    }
    case 'intro': return { ...view, intro: action.value }
    case 'transition': return { ...view, noTransition: action.disabled }
    case 'cardDrag': return { ...view, cardDragging: action.active, noTransition: action.active }
    case 'cardMove': return { ...view, rect: action.rect }
    case 'cardResize': return { ...view, cardResizing: action.active }
    case 'cardSize': return {
      ...view,
      cardHeight: action.height,
      rect: view.rect.width === action.width ? view.rect : { ...view.rect, width: action.width },
    }
  }
}

/** Owns the bar's animation and card interaction state across one Lens opening. */
export function useLensBarMotionController(initialRect: BarRect) {
  const [view, dispatch] = useReducer(transition, initialRect, initialView)
  const open = useCallback((rect: BarRect) => dispatch({ type: 'reset', rect }), [])
  const hide = useCallback((rect: BarRect) => dispatch({ type: 'reset', rect }), [])
  const snap = useCallback((rect: BarRect) => dispatch({ type: 'snap', rect }), [])
  const flyTo = useCallback((rect: BarRect, intro = true) => dispatch({ type: 'fly', rect, intro }), [])
  const settleFly = useCallback(() => dispatch({ type: 'settleFly' }), [])
  const reveal = useCallback(() => dispatch({ type: 'reveal' }), [])
  const floatMac = useCallback((width: number, translate: boolean) => dispatch({ type: 'floatingMac', width, translate }), [])
  const floatWindows = useCallback((rect: BarRect, translate: boolean) => dispatch({ type: 'floatingWindows', rect, translate }), [])
  const markFloatingRebased = useCallback((value: boolean) => dispatch({ type: 'floatingRebased', value }), [])
  const abortFloatingRebase = useCallback((rect: BarRect) => dispatch({ type: 'floatingAbort', rect }), [])
  const showIntro = useCallback((value: boolean) => dispatch({ type: 'intro', value }), [])
  const suppressTransition = useCallback((disabled: boolean) => dispatch({ type: 'transition', disabled }), [])
  const beginCardDrag = useCallback(() => dispatch({ type: 'cardDrag', active: true }), [])
  const moveCard = useCallback((rect: BarRect) => dispatch({ type: 'cardMove', rect }), [])
  const endCardDrag = useCallback(() => dispatch({ type: 'cardDrag', active: false }), [])
  const beginCardResize = useCallback(() => dispatch({ type: 'cardResize', active: true }), [])
  const resizeCard = useCallback((width: number, height: number) => dispatch({ type: 'cardSize', width, height }), [])
  const endCardResize = useCallback(() => dispatch({ type: 'cardResize', active: false }), [])
  return {
    view, open, hide, snap, flyTo, settleFly, reveal, floatMac, floatWindows,
    markFloatingRebased, abortFloatingRebase, showIntro, suppressTransition,
    beginCardDrag, moveCard, endCardDrag, beginCardResize, resizeCard, endCardResize,
  }
}
