import { act, renderHook } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { useLensAnnotationController } from './useLensAnnotationController'

describe('useLensAnnotationController', () => {
  it('owns drawing completion, undo and hide reset as one session transition', () => {
    const { result } = renderHook(() => useLensAnnotationController())
    act(() => {
      result.current.toggleDraw()
      result.current.begin('arrow', 10, 10)
      result.current.move(40, 40)
      result.current.finish()
    })
    expect(result.current.view.arrows).toHaveLength(1)
    expect(result.current.view.draft).toBeNull()
    act(() => result.current.undo())
    expect(result.current.view.arrows).toHaveLength(0)
    act(() => {
      result.current.setCopied(true)
      result.current.setSaving(true)
      result.current.hide()
    })
    expect(result.current.view).toMatchObject({
      drawMode: false, arrows: [], draft: null, copied: false, saving: false, tool: 'arrow',
    })
  })

  it('keeps committed annotations on exiting draw mode but clears them on leaving ready', () => {
    const { result } = renderHook(() => useLensAnnotationController())
    act(() => {
      result.current.begin('arrow', 0, 0)
      result.current.move(30, 0)
      result.current.finish()
      result.current.exitDraw()
    })
    expect(result.current.view.arrows).toHaveLength(1)
    act(() => result.current.stageChanged('answering'))
    expect(result.current.view.arrows).toEqual([])
  })
})
