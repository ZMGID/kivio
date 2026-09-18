import { act, renderHook } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { useLensTranslationSession } from './useLensTranslationSession'

describe('useLensTranslationSession', () => {
  it('owns translation accumulation and ignores events after completion', () => {
    const onFinished = vi.fn()
    const { result } = renderHook(() => useLensTranslationSession({ onFinished }))

    act(() => {
      result.current.beginTranslation()
      result.current.applyTranslationPayload({ imageId: 'image-1', kind: 'original', delta: 'hello' })
      result.current.applyTranslationPayload({ imageId: 'image-1', kind: 'translated', delta: '你好' })
      result.current.applyTranslationPayload({ imageId: 'image-1', kind: 'translated', done: true })
      result.current.applyTranslationPayload({ imageId: 'image-1', kind: 'translated', delta: 'late' })
    })

    expect(result.current.translateOriginal).toBe('hello')
    expect(result.current.translateText).toBe('你好')
    expect(result.current.durationMs).not.toBeNull()
    expect(onFinished).toHaveBeenCalledOnce()
  })

  it('resets replacement resources and does not complete twice', () => {
    const onFinished = vi.fn()
    const { result } = renderHook(() => useLensTranslationSession({ onFinished }))

    act(() => {
      result.current.beginReplacement()
      result.current.applyReplacementPayload({
        imageId: 'image-1',
        phase: 'done',
        cleanedImage: 'cleaned',
        warning: 'partial',
      })
      result.current.applyReplacementPayload({ imageId: 'image-1', phase: 'error', error: 'late' })
    })

    expect(result.current.replaceCleanedImage).toBe('cleaned')
    expect(result.current.replaceWarning).toBe('partial')
    expect(result.current.replaceError).toBe('')
    expect(onFinished).toHaveBeenCalledOnce()

    act(() => result.current.reset())
    expect(result.current.replaceCleanedImage).toBe('')
    expect(result.current.replaceWarning).toBe('')
    expect(result.current.durationMs).toBeNull()
  })
})
