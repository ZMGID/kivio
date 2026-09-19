import { useCallback, useEffect, useRef, useState } from 'react'
import { api, type ExplainMessage } from '../api/tauri'
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
    const owner = owners.current
    owner.session.resetForHide()
    publish(previous => emptyContent(previous.mode))
    owner.conversation.hide()
    owner.selection.hide()
    owner.annotation.hide()
    owner.translation.reset()
  }, [publish])

  const restoreHistory = useCallback((item: HistoryItem) => {
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
    prepareSend, releaseSendPreparation, isPreparingSend, beginAnswer, finishAnswer,
    conversation: conversation as Omit<typeof conversation, 'open' | 'hide' | 'restoreHistory' | 'beginAnswer'>,
    selection: selection as Omit<typeof selection, 'open' | 'hide'>,
    annotation: annotation as Omit<typeof annotation, 'hide' | 'stageChanged'>,
    translation: translation as Omit<typeof translation, 'reset'>,
    session: session as Omit<typeof session, 'beginOpening' | 'resetForHide' | 'restoreSession' | 'replaceFreezeFrame'>,
  }
}
