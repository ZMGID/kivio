import { act, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { useLensContentController } from './useLensContentController'
import type { HistoryItem } from './types'

const ask = vi.fn()
const sendToChat = vi.fn()
const sendHistoryToChat = vi.fn()
const registerAnnotatedImage = vi.fn()
const composeAnnotatedImage = vi.fn()

vi.mock('./annotation', async importOriginal => ({
  ...await importOriginal<typeof import('./annotation')>(),
  composeAnnotatedImage: (...args: unknown[]) => composeAnnotatedImage(...args),
}))

vi.mock('../api/tauri', () => ({ api: {
  lensReadImage: () => new Promise(() => {}),
  lensAsk: (...args: unknown[]) => ask(...args),
  lensSendToChat: (...args: unknown[]) => sendToChat(...args),
  lensSendHistoryToChat: (...args: unknown[]) => sendHistoryToChat(...args),
  lensRegisterAnnotatedImage: (...args: unknown[]) => registerAnnotatedImage(...args),
  lensCommitImageToHistory: async () => {},
  lensDeleteHistoryImage: async () => {},
} }))

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => { resolve = done })
  return { promise, resolve }
}

const saved: HistoryItem = {
  id: 'history-image', imagePreview: 'history-preview', appLabel: 'History app',
  messages: [{ role: 'assistant', content: 'saved answer' }], capturedFrame: null, timestamp: 1,
}

describe('Lens content transitions', () => {
  beforeEach(() => {
    localStorage.clear()
    ask.mockReset()
    sendToChat.mockReset()
    sendHistoryToChat.mockReset()
    registerAnnotatedImage.mockReset()
    composeAnnotatedImage.mockReset()
  })
  afterEach(() => vi.restoreAllMocks())

  it('closes only after the handoff send succeeds', async () => {
    const reply = deferred<{ success: boolean }>()
    sendToChat.mockReturnValue(reply.promise)
    const close = vi.fn().mockResolvedValue(undefined)
    const { result } = renderHook(() => useLensContentController({ initialMode: 'chat' }))
    act(() => {
      const opening = result.current.beginOpening()
      result.current.open({ mode: 'chat', opening, freezeFrameImageId: '' })
      result.current.captureImage('image-1')
    })
    let pending!: Promise<void>
    act(() => { pending = result.current.handoff({ question: 'Send me', close }) })
    expect(sendToChat).toHaveBeenCalledWith('image-1', 'Send me')
    expect(close).not.toHaveBeenCalled()
    await act(async () => { reply.resolve({ success: true }); await pending })
    expect(close).toHaveBeenCalledOnce()
  })

  it('returns to ready when closing the accepted handoff fails', async () => {
    sendToChat.mockResolvedValue({ success: true })
    const close = vi.fn().mockRejectedValue(new Error('OS close rejected'))
    vi.spyOn(console, 'error').mockImplementation(() => undefined)
    const { result } = renderHook(() => useLensContentController({ initialMode: 'chat' }))
    act(() => {
      const opening = result.current.beginOpening()
      result.current.open({ mode: 'chat', opening, freezeFrameImageId: '' })
      result.current.captureImage('image-1')
      result.current.conversation.showStage('ready')
    })
    await act(async () => { await result.current.handoff({ question: 'Send me', close }) })
    expect(close).toHaveBeenCalledOnce()
    expect(result.current.isPreparingSend()).toBe(false)
    expect(result.current.conversation.view).toMatchObject({ stage: 'ready', streaming: false })
  })

  it('returns to ready when the native close owner reports failure without throwing', async () => {
    sendToChat.mockResolvedValue({ success: true })
    const close = vi.fn().mockResolvedValue(false)
    const { result } = renderHook(() => useLensContentController({ initialMode: 'chat' }))
    act(() => {
      const opening = result.current.beginOpening()
      result.current.open({ mode: 'chat', opening, freezeFrameImageId: '' })
      result.current.conversation.showStage('ready')
    })
    await act(async () => { await result.current.handoff({ question: 'Send me', close }) })
    expect(result.current.conversation.view).toMatchObject({ stage: 'ready', streaming: false })
    expect(close).toHaveBeenCalledOnce()
  })

  it('retries only native close after an accepted history handoff', async () => {
    sendHistoryToChat.mockResolvedValue({ success: true })
    const close = vi.fn().mockResolvedValueOnce(false).mockResolvedValueOnce(true)
    const { result } = renderHook(() => useLensContentController({ initialMode: 'chat' }))
    act(() => {
      const opening = result.current.beginOpening()
      result.current.open({ mode: 'chat', opening, freezeFrameImageId: '' })
      result.current.captureImage('image-1')
      result.current.conversation.showStage('ready')
    })
    const history = [{ role: 'user' as const, content: 'Question' }, { role: 'assistant' as const, content: 'Answer' }]
    await act(async () => { await result.current.handoff({ history, close }) })
    expect(result.current.conversation.view).toMatchObject({ stage: 'ready', streaming: false })
    await act(async () => { await result.current.handoff({ history, close }) })
    expect(sendHistoryToChat).toHaveBeenCalledOnce()
    expect(sendHistoryToChat).toHaveBeenCalledWith('image-1', history)
    expect(close).toHaveBeenCalledTimes(2)
  })

  it('does not resend a single accepted handoff when closing is retried', async () => {
    sendToChat.mockResolvedValue({ success: true })
    const close = vi.fn().mockResolvedValue(false)
    const { result } = renderHook(() => useLensContentController({ initialMode: 'chat' }))
    act(() => {
      const opening = result.current.beginOpening()
      result.current.open({ mode: 'chat', opening, freezeFrameImageId: '' })
      result.current.conversation.showStage('ready')
    })
    await act(async () => { await result.current.handoff({ question: 'Send me', close }) })
    await act(async () => { await result.current.handoff({ question: 'Send me', close }) })
    expect(sendToChat).toHaveBeenCalledOnce()
    expect(close).toHaveBeenCalledTimes(2)
    expect(result.current.conversation.view).toMatchObject({ stage: 'ready', streaming: false })
  })

  it('does not roll back a newer opening if old native close fails late', async () => {
    sendToChat.mockResolvedValue({ success: true })
    const closing = deferred<boolean>()
    const close = vi.fn(() => closing.promise)
    const { result } = renderHook(() => useLensContentController({ initialMode: 'chat' }))
    act(() => {
      const opening = result.current.beginOpening()
      result.current.open({ mode: 'chat', opening, freezeFrameImageId: '' })
    })
    let pending!: Promise<void>
    act(() => { pending = result.current.handoff({ question: 'Send me', close }) })
    await act(async () => { await Promise.resolve() })
    expect(close).toHaveBeenCalledOnce()
    act(() => {
      result.current.hide()
      const opening = result.current.beginOpening()
      result.current.open({ mode: 'chat', opening, freezeFrameImageId: '' })
    })
    await act(async () => { closing.resolve(false); await pending })
    expect(result.current.conversation.view).toMatchObject({ stage: 'select', streaming: false })
  })

  it('does not send an annotated handoff when image registration arrives after close and reopen', async () => {
    const registration = deferred<{ success: boolean; imageId: string }>()
    composeAnnotatedImage.mockResolvedValue('annotated-base64')
    registerAnnotatedImage.mockReturnValue(registration.promise)
    const close = vi.fn().mockResolvedValue(undefined)
    const { result } = renderHook(() => useLensContentController({ initialMode: 'chat' }))
    act(() => {
      const opening = result.current.beginOpening()
      result.current.open({ mode: 'chat', opening, freezeFrameImageId: '' })
      result.current.captureImage('image-1')
      result.current.adoptAnnotatedImage('image-1', 'data:image/png;base64,original')
      result.current.selection.captureFrame({ x: 0, y: 0, width: 100, height: 80, label: 'App' })
      result.current.conversation.showStage('ready')
    })
    act(() => {
      result.current.annotation.begin('arrow', 0, 0)
      result.current.annotation.move(30, 0)
      result.current.annotation.finish()
    })
    let pending!: Promise<void>
    act(() => { pending = result.current.handoff({ question: 'Send annotated', close }) })
    await act(async () => { await Promise.resolve() })
    expect(registerAnnotatedImage).toHaveBeenCalledWith('annotated-base64')
    act(() => {
      result.current.hide()
      const opening = result.current.beginOpening()
      result.current.open({ mode: 'chat', opening, freezeFrameImageId: '' })
    })
    await act(async () => { registration.resolve({ success: true, imageId: 'image-2' }); await pending })
    expect(sendToChat).not.toHaveBeenCalled()
    expect(close).not.toHaveBeenCalled()
    expect(result.current.currentImageId()).toBe('')
  })

  it('sends the registered annotated image and clears submitted marks', async () => {
    composeAnnotatedImage.mockResolvedValue('annotated-base64')
    registerAnnotatedImage.mockResolvedValue({ success: true, imageId: 'image-2' })
    sendToChat.mockResolvedValue({ success: true })
    const close = vi.fn().mockResolvedValue(undefined)
    const { result } = renderHook(() => useLensContentController({ initialMode: 'chat' }))
    act(() => {
      const opening = result.current.beginOpening()
      result.current.open({ mode: 'chat', opening, freezeFrameImageId: '' })
      result.current.captureImage('image-1')
      result.current.adoptAnnotatedImage('image-1', 'data:image/png;base64,original')
      result.current.selection.captureFrame({ x: 0, y: 0, width: 100, height: 80, label: 'App' })
      result.current.conversation.showStage('ready')
    })
    act(() => {
      result.current.annotation.begin('arrow', 0, 0)
      result.current.annotation.move(30, 0)
      result.current.annotation.finish()
    })
    await act(async () => { await result.current.handoff({ question: 'Annotated', close }) })
    expect(sendToChat).toHaveBeenCalledWith('image-2', 'Annotated')
    expect(result.current.currentImageId()).toBe('image-2')
    expect(result.current.annotation.view.arrows).toEqual([])
    expect(close).toHaveBeenCalledOnce()
  })

  it('does not close a new opening when an old handoff reply arrives, and cancels only once', async () => {
    const reply = deferred<{ success: boolean }>()
    sendToChat.mockReturnValue(reply.promise)
    const cancelRequest = vi.fn().mockResolvedValue(undefined)
    const close = vi.fn().mockResolvedValue(undefined)
    const { result } = renderHook(() => useLensContentController({ initialMode: 'chat', cancelRequest }))
    act(() => {
      const opening = result.current.beginOpening()
      result.current.open({ mode: 'chat', opening, freezeFrameImageId: '' })
      result.current.captureImage('image-1')
    })
    let pending!: Promise<void>
    act(() => { pending = result.current.handoff({ question: 'Send me', close }) })
    expect(sendToChat).toHaveBeenCalledOnce()
    await act(async () => { await result.current.session.closeOpening({
      prepareHiddenSurface: () => result.current.hide(),
      waitForPaint: async () => undefined,
      hide: async () => undefined,
    }) })
    act(() => {
      const opening = result.current.beginOpening()
      result.current.open({ mode: 'chat', opening, freezeFrameImageId: '' })
    })
    await act(async () => { reply.resolve({ success: true }); await pending })
    expect(close).not.toHaveBeenCalled()
    expect(cancelRequest).toHaveBeenCalledOnce()
    expect(result.current.conversation.view).toMatchObject({ stage: 'select', streaming: false, messages: [] })
  })

  it('returns failed handoff to ready without closing or keeping preparation locked', async () => {
    const close = vi.fn().mockResolvedValue(undefined)
    sendToChat.mockResolvedValue({ success: false, error: 'send failed' })
    vi.spyOn(console, 'error').mockImplementation(() => undefined)
    const { result } = renderHook(() => useLensContentController({ initialMode: 'chat' }))
    act(() => {
      const opening = result.current.beginOpening()
      result.current.open({ mode: 'chat', opening, freezeFrameImageId: '' })
      result.current.captureImage('image-1')
      result.current.conversation.showStage('ready')
    })
    await act(async () => { await result.current.handoff({ question: 'Send me', close }) })
    expect(close).not.toHaveBeenCalled()
    expect(result.current.isPreparingSend()).toBe(false)
    expect(result.current.conversation.view).toMatchObject({ stage: 'ready', streaming: false })
  })

  it('keeps the final error after stream done arrives before the ask reply', async () => {
    const reply = deferred<{ success: boolean; error: string }>()
    ask.mockReturnValue(reply.promise)
    const { result } = renderHook(() => useLensContentController({ initialMode: 'chat' }))
    act(() => {
      const opening = result.current.beginOpening()
      result.current.open({ mode: 'chat', opening, freezeFrameImageId: '' })
      result.current.captureImage('image-1')
    })
    let pending!: Promise<void>
    act(() => { pending = result.current.ask({ question: 'Why?', errorLabel: 'Error', webSearch: false }) })
    expect(ask).toHaveBeenCalledOnce()
    act(() => {
      result.current.receiveChatStream({ imageId: 'image-1', kind: 'answer', delta: 'partial' })
      result.current.receiveChatStream({ imageId: 'image-1', kind: 'answer', delta: '', done: true })
    })
    expect(result.current.conversation.view.streaming).toBe(false)
    await act(async () => { reply.resolve({ success: false, error: 'failed' }); await pending })
    expect(result.current.conversation.view.messages).toEqual([
      { role: 'user', content: 'Why?' },
      { role: 'assistant', content: 'Error: failed' },
    ])
  })

  it('publishes the placeholder before invoking the backend so an immediate stream has a target', async () => {
    const { result } = renderHook(() => useLensContentController({ initialMode: 'chat' }))
    act(() => {
      const opening = result.current.beginOpening()
      result.current.open({ mode: 'chat', opening, freezeFrameImageId: '' })
      result.current.captureImage('image-1')
    })
    let visibleAtInvoke: unknown
    ask.mockImplementation(async () => {
      visibleAtInvoke = result.current.conversation.view.messages
      return { success: true, response: 'answer' }
    })
    let pending!: Promise<void>
    act(() => { pending = result.current.ask({ question: 'Now?', errorLabel: 'Error', webSearch: false }) })
    expect(visibleAtInvoke).toEqual([
      { role: 'user', content: 'Now?' },
      { role: 'assistant', content: '' },
    ])
    await act(async () => { await pending })
  })

  it.each(['history', 'close-reopen', 'cancel'])('rejects an old ask reply after done and %s', async change => {
    const reply = deferred<{ success: boolean; response: string }>()
    ask.mockReturnValue(reply.promise)
    const cancelRequest = vi.fn().mockResolvedValue(undefined)
    const { result } = renderHook(() => useLensContentController({ initialMode: 'chat', cancelRequest }))
    act(() => {
      const opening = result.current.beginOpening()
      result.current.open({ mode: 'chat', opening, freezeFrameImageId: '' })
      result.current.captureImage('image-1')
    })
    let pending!: Promise<void>
    act(() => { pending = result.current.ask({ question: 'Old?', errorLabel: 'Error', webSearch: false }) })
    act(() => {
      result.current.receiveChatStream({ imageId: 'image-1', kind: 'answer', delta: 'partial' })
      result.current.receiveChatStream({ imageId: 'image-1', kind: 'answer', delta: '', done: true })
    })
    if (change === 'history') await act(async () => { await result.current.restoreHistory(saved) })
    else if (change === 'close-reopen') act(() => {
      result.current.hide()
      const opening = result.current.beginOpening()
      result.current.open({ mode: 'chat', opening, freezeFrameImageId: '' })
    })
    else await act(async () => { await result.current.cancelAnswer() })

    await act(async () => { reply.resolve({ success: true, response: 'late answer' }); await pending })
    expect(result.current.conversation.view.messages).toEqual(change === 'history'
      ? saved.messages
      : change === 'close-reopen' ? [] : [
        { role: 'user', content: 'Old?' },
        { role: 'assistant', content: 'partial' },
      ])
    expect(result.current.conversation.view.streaming).toBe(false)
    expect(cancelRequest).not.toHaveBeenCalled()
  })

  it('cancels an active ask once and leaves late content out of the stopped answer', async () => {
    const reply = deferred<{ success: boolean; response: string }>()
    ask.mockReturnValue(reply.promise)
    const cancelRequest = vi.fn().mockResolvedValue(undefined)
    const { result } = renderHook(() => useLensContentController({ initialMode: 'chat', cancelRequest }))
    act(() => {
      const opening = result.current.beginOpening()
      result.current.open({ mode: 'chat', opening, freezeFrameImageId: '' })
      result.current.captureImage('image-1')
    })
    let pending!: Promise<void>
    act(() => { pending = result.current.ask({ question: 'Stop?', errorLabel: 'Error', webSearch: false }) })
    act(() => { result.current.receiveChatStream({ imageId: 'image-1', kind: 'answer', delta: 'partial' }) })
    await act(async () => { await result.current.cancelAnswer(); await result.current.cancelAnswer() })
    act(() => { result.current.receiveChatStream({ imageId: 'image-1', kind: 'answer', delta: 'late' }) })
    await act(async () => { reply.resolve({ success: true, response: 'late answer' }); await pending })
    expect(cancelRequest).toHaveBeenCalledOnce()
    expect(result.current.conversation.view.messages).toEqual([
      { role: 'user', content: 'Stop?' },
      { role: 'assistant', content: 'partial' },
    ])
  })

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
