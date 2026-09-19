import { useCallback, useEffect, useRef, useState } from 'react'

export type LensSessionToken = Readonly<{ open: number; initialization: number }>
export type LensRequestKind = 'chat' | 'handoff' | 'translate' | 'translate_text' | 'replace'
export type LensRequestToken = Readonly<{
  open: number
  request: number
  kind: LensRequestKind
  resourceId: string
}>

export type LensSessionCoordinatorOptions = {
  cancelRequest?: () => Promise<unknown> | unknown
}

export type LensCloseOperations = {
  /** Conceal the current DOM before native hide, without discarding content yet. */
  prepareHiddenSurface: () => void
  /** Discard content only after native hide succeeds for the same opening. */
  commitHiddenSurface?: () => void
  /** Reveal retained content when native hide fails for the same opening. */
  rollbackHiddenSurface?: () => void
  /** Resume an unfinished capture setup with a fresh initialization token. */
  resumeCaptureInitialization?: () => void
  waitForPaint: () => Promise<void>
  hide: () => Promise<unknown>
}

/**
 * Owns the identity and disposable capture resources of one Lens opening.
 *
 * UI animation state remains in the Lens view, but async initialization,
 * selection reads and capture work must all prove that their token still
 * belongs to the current opening before publishing a result. A request starts
 * before its first awaited preparation step and ends on its terminal event or
 * synchronous result. Cancellation invalidates the token before awaiting the
 * backend boundary; reopen and unmount release an active request. Stream events
 * are accepted only when opening, request kind and resource identity all match.
 */
export function useLensSessionCoordinator(options: LensSessionCoordinatorOptions = {}) {
  const openSequence = useRef(0)
  const initializationSequence = useRef(0)
  const selectionSequence = useRef(0)
  const requestSequence = useRef(0)
  const activeRequest = useRef<LensRequestToken | null>(null)
  const closing = useRef<{ opening: number; promise: Promise<boolean> } | null>(null)
  const feedbackClose = useRef<{ timer: ReturnType<typeof setTimeout>; resolve: (closed: boolean) => void } | null>(null)
  const closedOpening = useRef<number | null>(null)
  const cancelRequest = useRef(options.cancelRequest)
  cancelRequest.current = options.cancelRequest
  const captureReadyRef = useRef(false)
  const [captureReady, setCaptureReady] = useState(false)
  const capturing = useRef(false)
  const [freezeFrameImageId, setFreezeFrameImageId] = useState('')
  const [freezeFramePreviewId, setFreezeFramePreviewId] = useState('')

  const beginInitialization = useCallback(() => {
    initializationSequence.current += 1
    captureReadyRef.current = false
    setCaptureReady(false)
    return initializationSequence.current
  }, [])

  const invalidateInitialization = useCallback(() => {
    initializationSequence.current += 1
    captureReadyRef.current = false
    setCaptureReady(false)
  }, [])

  const isInitializationCurrent = useCallback(
    (sequence: number) => sequence === initializationSequence.current,
    [],
  )

  const invalidateActiveRequest = useCallback(() => {
    requestSequence.current += 1
    const hadActiveRequest = activeRequest.current !== null
    activeRequest.current = null
    return hadActiveRequest
  }, [])

  const invokeCancelBoundary = useCallback(() => {
    try {
      const pending = cancelRequest.current?.()
      if (pending && typeof (pending as Promise<unknown>).catch === 'function') {
        void (pending as Promise<unknown>).catch((error) => console.error('[lens] cancel request failed', error))
      }
    } catch (error) {
      console.error('[lens] cancel request failed', error)
    }
  }, [])

  const cancelFeedbackClose = useCallback(() => {
    const pending = feedbackClose.current
    if (!pending) return
    feedbackClose.current = null
    clearTimeout(pending.timer)
    pending.resolve(false)
  }, [])

  const beginOpening = useCallback(() => {
    cancelFeedbackClose()
    const first = openSequence.current === 0
    if (invalidateActiveRequest()) invokeCancelBoundary()
    openSequence.current += 1
    captureReadyRef.current = false
    setCaptureReady(false)
    capturing.current = false
    return { first, sequence: openSequence.current }
  }, [cancelFeedbackClose, invalidateActiveRequest, invokeCancelBoundary])

  const beginRequest = useCallback((kind: LensRequestKind, resourceId: string): LensRequestToken => {
    if (activeRequest.current !== null) {
      requestSequence.current += 1
      activeRequest.current = null
      invokeCancelBoundary()
    }
    requestSequence.current += 1
    const token = {
      open: openSequence.current,
      request: requestSequence.current,
      kind,
      resourceId,
    }
    activeRequest.current = token
    return token
  }, [invokeCancelBoundary])

  const isRequestCurrent = useCallback((token: LensRequestToken) => (
    token.open === openSequence.current
    && token.request === requestSequence.current
    && activeRequest.current === token
  ), [])

  const isRequestLatest = useCallback((token: LensRequestToken) => (
    token.open === openSequence.current
    && token.request === requestSequence.current
  ), [])

  const acceptsRequestEvent = useCallback((kind: LensRequestKind, resourceId: string) => {
    const request = activeRequest.current
    return request !== null
      && request.open === openSequence.current
      && request.request === requestSequence.current
      && request.kind === kind
      && request.resourceId === resourceId
  }, [])

  const finishRequest = useCallback((token: LensRequestToken) => {
    if (!isRequestCurrent(token)) return false
    activeRequest.current = null
    return true
  }, [isRequestCurrent])

  const finishRequestEvent = useCallback((kind: LensRequestKind, resourceId: string) => {
    if (!acceptsRequestEvent(kind, resourceId)) return false
    activeRequest.current = null
    return true
  }, [acceptsRequestEvent])

  const cancelActiveRequest = useCallback(async () => {
    if (!invalidateActiveRequest()) return false
    await cancelRequest.current?.()
    return true
  }, [invalidateActiveRequest])

  const currentToken = useCallback((): LensSessionToken => ({
    open: openSequence.current,
    initialization: initializationSequence.current,
  }), [])

  const currentOpening = useCallback(() => openSequence.current, [])

  const isOpeningCurrent = useCallback((sequence: number) => sequence === openSequence.current, [])

  const isTokenCurrent = useCallback((token: LensSessionToken) => (
    token.open === openSequence.current
    && token.initialization === initializationSequence.current
  ), [])

  const markCaptureReady = useCallback((initialization: number) => {
    if (initialization !== initializationSequence.current) return false
    captureReadyRef.current = true
    setCaptureReady(true)
    return true
  }, [])

  const canCapture = useCallback(() => captureReadyRef.current && !capturing.current, [])

  const beginCapture = useCallback((): LensSessionToken | null => {
    if (!captureReadyRef.current || capturing.current) return null
    capturing.current = true
    return {
      open: openSequence.current,
      initialization: initializationSequence.current,
    }
  }, [])

  const finishCapture = useCallback((token: LensSessionToken) => {
    if (!isTokenCurrent(token)) return false
    capturing.current = false
    return true
  }, [isTokenCurrent])

  const isCapturing = useCallback(() => capturing.current, [])

  const beginSelectionRead = useCallback(() => {
    selectionSequence.current += 1
    return selectionSequence.current
  }, [])

  const isSelectionCurrent = useCallback(
    (sequence: number) => sequence === selectionSequence.current,
    [],
  )

  const resetForHide = useCallback(() => {
    cancelFeedbackClose()
    invalidateActiveRequest()
    initializationSequence.current += 1
    selectionSequence.current += 1
    captureReadyRef.current = false
    setCaptureReady(false)
    capturing.current = false
    setFreezeFrameImageId('')
    setFreezeFramePreviewId('')
  }, [cancelFeedbackClose, invalidateActiveRequest])

  /** An old selection read or late invoke result cannot publish over restored history. */
  const restoreSession = useCallback((applySnapshot: () => void): Promise<void> => {
    cancelFeedbackClose()
    const cancellation = cancelActiveRequest()
    invalidateInitialization()
    capturing.current = false
    beginSelectionRead()
    try {
      applySnapshot()
    } catch (error) {
      void cancellation.catch(() => undefined)
      return Promise.reject(error)
    }
    return cancellation.then(() => undefined)
  }, [beginSelectionRead, cancelActiveRequest, cancelFeedbackClose, invalidateInitialization])

  /** Reset the visible surface before native hide, and refuse to hide a newer opening. */
  const closeOpening = useCallback((operations: LensCloseOperations): Promise<boolean> => {
    const opening = openSequence.current
    if (closing.current?.opening === opening) return closing.current.promise
    if (closedOpening.current === opening) return Promise.resolve(false)
    const captureWasReady = captureReadyRef.current
    cancelFeedbackClose()
    // Closing owns the context from the intent, not from the eventual cancel reply.
    // A slow backend must not let an old capture/selection repopulate the surface.
    invalidateInitialization()
    beginSelectionRead()
    capturing.current = false
    const promise = (async () => {
      try {
        await cancelActiveRequest()
      } catch (error) {
        console.error('[lens] cancel request failed', error)
      }
      if (openSequence.current !== opening) return false
      let prepared = false
      try {
        prepared = true
        operations.prepareHiddenSurface()
        await operations.waitForPaint()
        if (openSequence.current !== opening) return false
        await operations.hide()
      } catch (error) {
        if (prepared && openSequence.current === opening) {
          try {
            if (operations.rollbackHiddenSurface) operations.rollbackHiddenSurface()
          }
          catch (rollbackError) { console.error('[lens] close rollback failed', rollbackError) }
          if (openSequence.current === opening) {
            captureReadyRef.current = captureWasReady
            setCaptureReady(captureWasReady)
            if (!captureWasReady) {
              try { operations.resumeCaptureInitialization?.() }
              catch (resumeError) { console.error('[lens] capture resume failed', resumeError) }
            }
          }
        }
        throw error
      }
      if (openSequence.current !== opening) return false
      operations.commitHiddenSurface?.()
      closedOpening.current = opening
      return true
    })()
    closing.current = { opening, promise }
    void promise.finally(() => {
      if (closing.current?.promise === promise) closing.current = null
    }).catch(() => undefined)
    return promise
  }, [beginSelectionRead, cancelActiveRequest, cancelFeedbackClose, invalidateInitialization])

  /** Copy feedback belongs to the content that produced it, never the next opening/history. */
  const closeAfterFeedback = useCallback((
    token: LensSessionToken,
    delayMs: number,
    operations: LensCloseOperations,
  ): Promise<boolean> => {
    if (!isTokenCurrent(token)) return Promise.resolve(false)
    cancelFeedbackClose()
    return new Promise<boolean>((resolve, reject) => {
      const timer = setTimeout(() => {
        feedbackClose.current = null
        if (!isTokenCurrent(token)) { resolve(false); return }
        void closeOpening(operations).then(resolve, reject)
      }, delayMs)
      feedbackClose.current = { timer, resolve }
    })
  }, [cancelFeedbackClose, closeOpening, isTokenCurrent])

  useEffect(() => () => {
    cancelFeedbackClose()
    if (invalidateActiveRequest()) invokeCancelBoundary()
    initializationSequence.current += 1
    selectionSequence.current += 1
    captureReadyRef.current = false
    capturing.current = false
  }, [cancelFeedbackClose, invalidateActiveRequest, invokeCancelBoundary])

  const replaceFreezeFrame = useCallback((imageId: string) => {
    setFreezeFrameImageId(imageId)
    setFreezeFramePreviewId(imageId)
  }, [])

  const consumeFreezeFrame = useCallback(() => setFreezeFrameImageId(''), [])

  return {
    acceptsRequestEvent,
    beginCapture,
    beginInitialization,
    beginOpening,
    beginRequest,
    beginSelectionRead,
    canCapture,
    cancelActiveRequest,
    closeOpening,
    closeAfterFeedback,
    captureReady,
    consumeFreezeFrame,
    currentOpening,
    currentToken,
    finishCapture,
    finishRequest,
    finishRequestEvent,
    freezeFrameImageId,
    freezeFramePreviewId,
    invalidateInitialization,
    isCapturing,
    isInitializationCurrent,
    isOpeningCurrent,
    isRequestCurrent,
    isRequestLatest,
    isSelectionCurrent,
    isTokenCurrent,
    markCaptureReady,
    replaceFreezeFrame,
    resetForHide,
    restoreSession,
  }
}
