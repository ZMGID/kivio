import { act, renderHook } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { useProviderModalController } from './useProviderModalController'

describe('useProviderModalController', () => {
  it('invalidates picker and delete confirmation as soon as a provider disappears', () => {
    const { result, rerender } = renderHook(({ ids }) => useProviderModalController(ids), {
      initialProps: { ids: ['a', 'b'] },
    })
    act(() => { result.current.openPicker('a'); result.current.requestDelete('a') })
    expect(result.current.pickerId).toBe('a')
    expect(result.current.deleteId).toBe('a')
    rerender({ ids: ['b'] })
    expect(result.current.pickerId).toBeNull()
    expect(result.current.deleteId).toBeNull()
    expect(result.current.selectedId).toBe('b')
  })

  it('dismisses the highest-priority modal and does not reopen stale IDs', () => {
    const { result, rerender } = renderHook(({ ids }) => useProviderModalController(ids), {
      initialProps: { ids: ['a'] },
    })
    act(() => { result.current.openPicker('a'); result.current.requestDelete('a') })
    act(() => { expect(result.current.dismissTop()).toBe(true) })
    expect(result.current.pickerId).toBeNull()
    expect(result.current.deleteId).toBe('a')
    rerender({ ids: [] })
    act(() => { result.current.openPicker('a') })
    expect(result.current.pickerId).toBeNull()
    expect(result.current.dismissTop()).toBe(false)
  })
})
