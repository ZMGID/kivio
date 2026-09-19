import { act, renderHook } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { useLensBarMotionController } from './useLensBarMotionController'

const selectRect = { x: 20, y: 500, width: 400 }
const targetRect = { x: 600, y: 100, width: 400 }

describe('useLensBarMotionController', () => {
  it('starts and settles a fly atomically, then hides with no stale animation', () => {
    const { result } = renderHook(() => useLensBarMotionController(selectRect))
    act(() => result.current.flyTo(targetRect, true))
    expect(result.current.view).toMatchObject({
      rect: targetRect, delta: { x: -580, y: 400 }, noTransition: true, intro: true,
    })
    act(() => result.current.settleFly())
    expect(result.current.view).toMatchObject({ delta: { x: 0, y: 0 }, noTransition: false })
    act(() => result.current.hide(selectRect))
    expect(result.current.view).toMatchObject({ rect: selectRect, intro: false, noTransition: true, floatingRebased: false, cardDragging: false, cardResizing: false, cardHeight: 0 })
  })

  it('clears drag and resize activity on reopen', () => {
    const { result } = renderHook(() => useLensBarMotionController(selectRect))
    act(() => {
      result.current.beginCardDrag()
      result.current.beginCardResize()
      result.current.resizeCard(500, 300)
      result.current.open(targetRect)
    })
    expect(result.current.view).toMatchObject({ rect: targetRect, cardDragging: false, cardResizing: false, cardHeight: 0 })
  })
})
