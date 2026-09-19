import { useCallback, useEffect, useRef, useState } from 'react'
import { flushSync } from 'react-dom'
import { api, type ExplainMessage, type LensStreamPayload, type LensWebSearchPayload } from '../api/tauri'
import { composeAnnotatedImage } from './annotation'
import { useImageObjectUrl } from './useImageObjectUrl'
import { useLensAnnotationController } from './useLensAnnotationController'
import { useLensConversationController } from './useLensConversationController'
import { useLensHistory } from './useLensHistory'
import { useLensSelectionController } from './useLensSelectionController'
import { useLensSessionCoordinator, type LensRequestToken, type LensSessionCoordinatorOptions } from './useLensSessionCoordinator'
import { useLensTranslationSession } from './useLensTranslationSession'
import type { HistoryItem, Mode } from './types'

type ContentIdentity = {
  mode: Mode
  imageId: string
  historyKey: string
  preview: { imageId: string; url: string }
  preparing: boolean
  completed: boolean
}
const emptyContent = (mode: Mode): ContentIdentity => ({
  mode, imageId: '', historyKey: '', preview: { imageId: '', url: '' }, preparing: false, completed: false,
})

/** Owns content identity and the open/hide/history transitions between existing Lens owners.
 * The page owns geometry and native surface painting; it cannot reset individual content owners.
 * No component setters or refs are accepted or exposed by this boundary. */
export function useLensContentController(options: LensSessionCoordinatorOptions & { initialMode: Mode }) {
  const conversation = useLensConversationController()
  const selection = useLensSelectionController()
  const annotation = useLensAnnotationController()
  const translation = useLensTranslationSession({ onFinished: () => conversation.showStage('translated') })
  const session = useLensSessionCoordinator(options)
  const history = useLensHistory()
  const [identity, setIdentity] = useState(() => emptyContent(options.initialMode))
  const current = useRef(identity)
  const deliveredHandoffOpening = useRef<number | null>(null)
  const retryingHandoffClose = useRef(false)
  const lastChatStreamEvent = useRef('')
  const owners = useRef({ conversation, selection, annotation, translation, session })
  owners.current = { conversation, selection, annotation, translation, session }
  const publish = useCallback((update: (previous: ContentIdentity) => ContentIdentity) => {
    const next = update(current.current)
    current.current = next
    setIdentity(next)
  }, [])
  const capturedPreview = useImageObjectUrl(identity.preview.imageId, api.lensReadImage)
  const imagePreview = identity.preview.url || capturedPreview

  const beginOpening = useCallback(() => {
    deliveredHandoffOpening.current = null
    retryingHandoffClose.current = false
    publish(previous => ({ ...previous, preparing: false, completed: false }))
    return owners.current.session.beginOpening()
  }, [publish])

  const open = useCallback((input: {
    mode: Mode
    opening: { first: boolean; sequence: number }
    freezeFrameImageId: string
  }) => {
    const owner = owners.current
    if (!owner.session.isOpeningCurrent(input.opening.sequence)) return false
    publish(() => emptyContent(input.mode))
    owner.conversation.open(input.mode)
    owner.selection.open(input.opening.first)
    owner.annotation.hide()
    owner.translation.reset()
    owner.session.replaceFreezeFrame(input.freezeFrameImageId)
    return true
  }, [publish])

  const hide = useCallback(() => {
    deliveredHandoffOpening.current = null
    retryingHandoffClose.current = false
    const owner = owners.current
    owner.session.resetForHide()
    publish(previous => emptyContent(previous.mode))
    owner.conversation.hide()
    owner.selection.hide()
    owner.annotation.hide()
    owner.translation.reset()
  }, [publish])

  const restoreHistory = useCallback((item: HistoryItem) => {
    deliveredHandoffOpening.current = null
    retryingHandoffClose.current = false
    const owner = owners.current
    return owner.session.restoreSession(() => {
      publish(previous => ({
        ...emptyContent(previous.mode),
        imageId: item.imagePreview ? item.id : '',
        historyKey: item.id,
        preview: { imageId: '', url: item.imagePreview },
      }))
      owner.conversation.restoreHistory(item.appLabel, item.messages)
      owner.selection.hide()
      owner.annotation.hide()
      owner.translation.reset()
    })
  }, [publish])

  const captureImage = useCallback((imageId: string) => {
    publish(previous => ({ ...previous, imageId, preview: { imageId, url: '' } }))
  }, [publish])
  const adoptAnnotatedImage = useCallback((imageId: string, preview?: string) => {
    publish(previous => ({
      ...previous, imageId,
      preview: preview === undefined ? previous.preview : { imageId: '', url: preview },
    }))
  }, [publish])
  const currentImageId = useCallback(() => current.current.imageId, [])
  const isPreparingSend = useCallback(() => current.current.preparing, [])

  const beginTextTranslation = useCallback((text: string, requestId: string) => {
    const owner = owners.current
    publish(previous => ({ ...previous, imageId: requestId }))
    const token = owner.session.beginRequest('translate_text', requestId)
    owner.conversation.selectText(text)
    owner.translation.beginTranslation()
    return token
  }, [publish])

  const prepareSend = useCallback((kind: 'chat' | 'handoff') => {
    publish(previous => ({ ...previous, preparing: true, completed: false }))
    return owners.current.session.beginRequest(kind, current.current.imageId)
  }, [publish])
  const releaseSendPreparation = useCallback((token: LensRequestToken) => {
    if (!owners.current.session.isRequestLatest(token)) return
    publish(previous => ({ ...previous, preparing: false }))
  }, [publish])
  const beginAnswer = useCallback((messages: ExplainMessage[]) => {
    publish(previous => ({
      ...previous, completed: false,
      historyKey: previous.imageId || previous.historyKey || `text-${Date.now()}-${Math.random().toString(36).slice(2)}`,
    }))
    owners.current.conversation.beginAnswer(messages)
  }, [publish])
  const finishAnswer = useCallback(() => {
    if (current.current.completed) return
    publish(previous => ({ ...previous, completed: true }))
    owners.current.conversation.setBusy(false)
  }, [publish])

  const receiveChatStream = useCallback((payload: LensStreamPayload) => {
    const owner = owners.current
    if (!owner.session.acceptsRequestEvent('chat', payload.imageId)) return false
    if (payload.done) {
      owner.session.finishRequestEvent('chat', payload.imageId)
      lastChatStreamEvent.current = ''
      finishAnswer()
      return true
    }
    const eventKey = [payload.imageId, payload.kind, payload.delta ?? '', payload.reasoningDelta ?? ''].join('\u0000')
    if (eventKey === lastChatStreamEvent.current) return false
    lastChatStreamEvent.current = eventKey
    if (payload.reasoningDelta || payload.delta) owner.conversation.applyStream(payload)
    return true
  }, [finishAnswer])

  const receiveChatWebSearch = useCallback((payload: LensWebSearchPayload) => {
    const owner = owners.current
    if (!owner.session.acceptsRequestEvent('chat', payload.imageId)) return false
    owner.conversation.applyWebSearch(payload)
    return true
  }, [])

  const cancelAnswer = useCallback(async () => {
    const cancellation = owners.current.session.cancelActiveRequest()
    finishAnswer()
    return cancellation
  }, [finishAnswer])

  const ask = useCallback(async (intent: { question: string; webSearch: boolean; errorLabel: string }) => {
    const owner = owners.current
    if (owner.conversation.view.streaming) return
    const sendMessages: ExplainMessage[] = [
      ...owner.conversation.view.messages,
      { role: 'user', content: intent.question },
    ]
    const image = current.current.imageId
    const preview = imagePreview
    const arrows = owner.annotation.view.arrows
    const frame = owner.selection.view.capturedFrame
    // The backend may synchronously publish its first chunk from the invoke boundary.
    // Commit the assistant placeholder before crossing that boundary.
    flushSync(() => beginAnswer([...sendMessages, { role: 'assistant', content: '' }]))
    lastChatStreamEvent.current = ''
    let token = prepareSend('chat')
    try {
      let effectiveImageId = image
      if (arrows.length > 0 && preview && frame) {
        try {
          const base64 = await composeAnnotatedImage(preview, arrows, frame.width, frame.height)
          const registered = await api.lensRegisterAnnotatedImage(base64)
          if (!owner.session.isRequestCurrent(token)) return
          if (registered.success && registered.imageId) {
            effectiveImageId = registered.imageId
            adoptAnnotatedImage(registered.imageId, `data:image/png;base64,${base64}`)
            owner.session.finishRequest(token)
            token = prepareSend('chat')
            owner.annotation.clearSubmitted()
          } else {
            console.warn('[lens-arrow] register annotated image failed:', registered.error)
          }
        } catch (error) {
          console.warn('[lens-arrow] compose failed, fallback to original:', error)
        }
      }
      if (!owner.session.isRequestCurrent(token)) return
      releaseSendPreparation(token)
      const reply = await api.lensAsk(effectiveImageId || '', sendMessages, { webSearch: intent.webSearch })
      if (!owner.session.isRequestLatest(token)) return
      if (!reply.success) owner.conversation.applyFinal({ error: `${intent.errorLabel}: ${reply.error}` })
      else {
        if (reply.response) owner.conversation.applyFinal({ response: reply.response })
        if (reply.webSearchResults?.length) owner.conversation.applyFinal({ results: reply.webSearchResults })
      }
      owner.session.finishRequest(token)
      finishAnswer()
    } catch (error) {
      if (!owner.session.isRequestLatest(token)) return
      const message = error instanceof Error ? error.message : String(error)
      owner.conversation.applyFinal({ error: `${intent.errorLabel}: ${message}` })
      owner.session.finishRequest(token)
      finishAnswer()
    } finally {
      releaseSendPreparation(token)
    }
  }, [adoptAnnotatedImage, beginAnswer, finishAnswer, imagePreview, prepareSend, releaseSendPreparation])

  const handoff = useCallback(async (intent: (
    | { question: string; history?: never }
    | { history: Pick<ExplainMessage, 'role' | 'content'>[]; question?: never }
  ) & { close: () => Promise<boolean | void> }) => {
    const owner = owners.current
    if (current.current.preparing || owner.conversation.view.streaming) return
    const opening = owner.session.currentOpening()
    if (deliveredHandoffOpening.current === opening) {
      // The backend accepted this handoff, but native hide failed. A second click
      // retries only the close; sending again would enqueue duplicate content.
      if (retryingHandoffClose.current) return
      retryingHandoffClose.current = true
      owner.conversation.setBusy(true)
      try {
        const closed = await intent.close()
        if (closed === false && owner.session.isOpeningCurrent(opening)) owner.conversation.handoffFailed()
      } catch (error) {
        if (owner.session.isOpeningCurrent(opening)) {
          console.error('[lens-chat] close after accepted handoff failed:', error)
          owner.conversation.handoffFailed()
        }
      } finally {
        retryingHandoffClose.current = false
      }
      return
    }
    const image = current.current.imageId
    const preview = imagePreview
    const arrows = owner.annotation.view.arrows
    const frame = owner.selection.view.capturedFrame
    owner.conversation.setBusy(true)
    let token = prepareSend('handoff')
    try {
      let effectiveImageId = image
      if (arrows.length > 0 && preview && frame) {
        try {
          const base64 = await composeAnnotatedImage(preview, arrows, frame.width, frame.height)
          const registered = await api.lensRegisterAnnotatedImage(base64)
          if (!owner.session.isRequestCurrent(token)) return
          if (registered.success && registered.imageId) {
            effectiveImageId = registered.imageId
            adoptAnnotatedImage(registered.imageId, `data:image/png;base64,${base64}`)
            owner.session.finishRequest(token)
            token = prepareSend('handoff')
            if (intent.question !== undefined) owner.annotation.clearSubmitted()
          } else {
            console.warn('[lens-arrow] register annotated image failed:', registered.error)
          }
        } catch (error) {
          console.warn('[lens-arrow] compose failed, fallback to original:', error)
        }
      }
      if (!owner.session.isRequestCurrent(token)) return
      const reply = intent.question !== undefined
        ? await api.lensSendToChat(effectiveImageId || '', intent.question)
        : await api.lensSendHistoryToChat(effectiveImageId || '', intent.history)
      if (!owner.session.isRequestCurrent(token)) return
      if (!reply.success) {
        console.error('[lens-chat] send failed:', reply.error)
        owner.session.finishRequest(token)
        owner.conversation.handoffFailed()
        return
      }
      owner.session.finishRequest(token)
      deliveredHandoffOpening.current = token.open
      const closed = await intent.close()
      if (closed === false && owner.session.isRequestLatest(token)) owner.conversation.handoffFailed()
    } catch (error) {
      if (!owner.session.isRequestLatest(token)) return
      console.error('[lens-chat] handoff failed:', error)
      owner.session.finishRequest(token)
      owner.conversation.handoffFailed()
    } finally {
      releaseSendPreparation(token)
    }
  }, [adoptAnnotatedImage, imagePreview, prepareSend, releaseSendPreparation])

  const { stage, streaming, messages, appLabel } = conversation.view
  const { stageChanged } = annotation
  useEffect(() => stageChanged(stage), [stage, stageChanged])

  const { recordCompleted } = history
  const { capturedFrame } = selection.view
  useEffect(() => {
    if (!identity.completed || identity.mode !== 'chat' || streaming) return
    const id = identity.imageId || identity.historyKey
    if (!id || !messages.some(message => message.role === 'assistant' && message.content)) return
    void recordCompleted({ id, imagePreview, appLabel, messages, capturedFrame, timestamp: Date.now() }, identity.imageId)
  }, [identity.completed, identity.mode, identity.imageId, identity.historyKey, streaming, messages, imagePreview, appLabel, capturedFrame, recordCompleted])

  return {
    mode: identity.mode, imagePreview, history: history.items,
    beginOpening, open, hide, restoreHistory,
    captureImage, adoptAnnotatedImage, currentImageId, beginTextTranslation,
    prepareSend, releaseSendPreparation, isPreparingSend,
    ask, handoff, receiveChatStream, receiveChatWebSearch, cancelAnswer,
    conversation: conversation as Omit<typeof conversation,
      'open' | 'hide' | 'restoreHistory' | 'beginAnswer' | 'applyStream' | 'applyWebSearch' | 'applyFinal'>,
    selection: selection as Omit<typeof selection, 'open' | 'hide'>,
    annotation: annotation as Omit<typeof annotation, 'hide' | 'stageChanged'>,
    translation: translation as Omit<typeof translation, 'reset'>,
    session: session as Omit<typeof session, 'beginOpening' | 'resetForHide' | 'restoreSession' | 'replaceFreezeFrame'>,
  }
}
