import { act, renderHook } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { useLensContentController } from './useLensContentController'
import type { HistoryItem } from './types'

vi.mock('../api/tauri', () => ({ api: {
  lensReadImage: () => new Promise(() => {}),
  lensCommitImageToHistory: async () => {},
  lensDeleteHistoryImage: async () => {},
} }))

const saved: HistoryItem = {
  id: 'history-image', imagePreview: 'history-preview', appLabel: 'History app',
  messages: [{ role: 'assistant', content: 'saved answer' }], capturedFrame: null, timestamp: 1,
}

describe('Lens content transitions', () => {
  beforeEach(() => localStorage.clear())

  it('restores history through one intent, clearing capture, annotation, translation and send preparation', async () => {
    const cancelRequest = vi.fn().mockResolvedValue(undefined)
    const { result } = renderHook(() => useLensContentController({ initialMode: 'chat', cancelRequest }))
    act(() => {
      const opening = result.current.beginOpening()
      result.current.open({ mode: 'chat', opening, freezeFrameImageId: 'freeze' })
      result.current.captureImage('old-image')
      result.current.conversation.editInput('unsent')
      result.current.selection.queueCapture({ x: 1, y: 2, width: 10, height: 20 })
      result.current.annotation.begin('arrow', 0, 0)
      result.current.annotation.move(40, 50)
      result.current.annotation.finish()
      result.current.translation.beginTranslation()
      result.current.translation.applyTranslationPayload({ imageId: 'old-image', kind: 'translated', delta: 'old translation' })
      result.current.prepareSend('chat')
    })
    const old = result.current.session.currentToken()

    await act(async () => result.current.restoreHistory(saved))

    expect(result.current.conversation.view).toMatchObject({ input: '', streaming: false, messages: saved.messages })
    expect(result.current.imagePreview).toBe('history-preview')
    expect(result.current.currentImageId()).toBe('history-image')
    expect(result.current.isPreparingSend()).toBe(false)
    expect(result.current.selection.view.pendingCapture).toBeNull()
    expect(result.current.annotation.view.arrows).toEqual([])
    expect(result.current.translation.translateText).toBe('')
    expect(result.current.session.isTokenCurrent(old)).toBe(false)
    expect(cancelRequest).toHaveBeenCalledOnce()
  })

  it('hides and reopens through atomic content transitions without reviving the old request', () => {
    const cancelRequest = vi.fn().mockResolvedValue(undefined)
    const { result } = renderHook(() => useLensContentController({ initialMode: 'chat', cancelRequest }))
    act(() => {
      const opening = result.current.beginOpening()
      result.current.open({ mode: 'chat', opening, freezeFrameImageId: 'old-freeze' })
      result.current.captureImage('old-image')
      result.current.conversation.editInput('old draft')
      result.current.selection.queueCapture({ x: 1, y: 2, width: 10, height: 20 })
      result.current.annotation.begin('arrow', 0, 0)
      result.current.annotation.move(40, 50)
      result.current.annotation.finish()
      result.current.translation.beginTranslation()
      result.current.prepareSend('chat')
    })
    const old = result.current.session.currentToken()

    act(() => result.current.hide())
    expect(result.current.currentImageId()).toBe('')
    expect(result.current.imagePreview).toBe('')
    expect(result.current.isPreparingSend()).toBe(false)
    expect(result.current.conversation.view.messages).toEqual([])
    expect(result.current.selection.view.pendingCapture).toBeNull()
    expect(result.current.annotation.view.arrows).toEqual([])
    expect(result.current.translation.translateText).toBe('')
    expect(result.current.session.isTokenCurrent(old)).toBe(false)

    act(() => {
      const opening = result.current.beginOpening()
      result.current.open({ mode: 'translateText', opening, freezeFrameImageId: 'new-freeze' })
    })
    expect(result.current.mode).toBe('translateText')
    expect(result.current.conversation.view.stage).toBe('translating')
    expect(result.current.currentImageId()).toBe('')
    expect(result.current.session.freezeFrameImageId).toBe('new-freeze')
    expect(result.current.session.isTokenCurrent(old)).toBe(false)
  })
})
