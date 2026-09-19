import { act, renderHook } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { useLensSelectionController } from './useLensSelectionController'

describe('useLensSelectionController', () => {
  it('keeps a cold-start drag, but clears it and a pending capture on reopen', () => {
    const { result } = renderHook(() => useLensSelectionController())
    act(() => {
      result.current.startDrag({ x: 20, y: 30 })
      result.current.moveDrag({ x: 80, y: 90 })
      result.current.queueCapture({ x: 20, y: 30, width: 60, height: 60 })
      result.current.open(true)
    })
    expect(result.current.view.dragStart).toEqual({ x: 20, y: 30 })
    expect(result.current.view.pendingCapture).not.toBeNull()

    act(() => result.current.open(false))
    expect(result.current.view).toMatchObject({ dragStart: null, dragCurrent: null, dragging: false, pendingCapture: null })
  })

  it('owns drag threshold, hover exclusion and hide cleanup', () => {
    const { result } = renderHook(() => useLensSelectionController())
    act(() => result.current.startDrag({ x: 10, y: 10 }))
    act(() => result.current.moveDrag({ x: 12, y: 12 }))
    expect(result.current.view.dragging).toBe(false)
    act(() => result.current.moveDrag({ x: 30, y: 10 }))
    expect(result.current.view.dragging).toBe(true)
    expect(result.current.view.hovered).toBeNull()
    act(() => result.current.hide())
    expect(result.current.view).toMatchObject({ dragStart: null, dragging: false, capturedFrame: null, showCaptureHint: false })
  })
})
