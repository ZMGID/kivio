import { createChatHistoryReader } from './chatHistoryReader'
import { conversationHash, getRouteConversationId, hashPath, setHash } from './chatRoutes'
import {
  awaitCurrentConversationNavigation,
  beginConversationTransition,
  cancelConversationTransition,
  captureConversationNavigation,
  completeConversationTransition,
  getConversationTransitionSnapshot,
  invalidateConversationTransition,
  isCurrentConversationNavigation,
  isCurrentConversationTransition,
  type ConversationLoadHint,
} from './conversationTransitionStore'
import { forgetRememberedChatRoute } from './persistence'
import { draftKey, migrateNewChatDraft } from './composerDraft'
import type { Conversation } from './types'
import type { ConversationHistoryPage } from './conversationHistoryWindow'


type ClearChatDecision = 'busy' | 'cancelled' | 'confirmed'
interface NavigationPorts {
  currentConversation: () => Conversation | null
  currentConversationId: () => string | null
  isResumingConversation?: (id: string) => boolean
  listPopouts: () => Promise<ReadonlySet<string>>
  readConversation: (conversationId: string) => Promise<Conversation>
  readConversationWindow?: (
    conversationId: string, preview: (window?: Conversation) => void, anchorMessageId?: string,
  ) => Promise<Conversation>
  knownConversationRevision?: (conversationId: string) => number
  readHistoryPage: (conversationId: string, before: number, after?: number) => Promise<ConversationHistoryPage>
  showHistoryPage: (conversation: Conversation) => void
  isConversationInFlight: (conversationId: string) => boolean
  prepareNewConversation: () => void
  clearEmptyChat: () => void
  /**
   * Check busy state before asking for confirmation; no route mutation here.
   * May resolve asynchronously (in-app confirm dialog); re-check busy after the answer.
   */
  requestClearChat: (conversationId: string) => ClearChatDecision | Promise<ClearChatDecision>
  deleteConversation: (conversationId: string) => Promise<void>
  cancelDeletedRun: (conversationId: string) => Promise<void>
  /** Synchronously drop local execution state, optionally clear this view, and refresh the list. */
  finalizeDeletedChat: (conversationId: string, clearCurrentView: boolean) => void
  reportClearError: (conversationId: string, message: string) => void
  focusPopout: (conversationId: string) => void
  occupyPopout: (conversationId: string) => void
  prepareSelection: (focusMessageId: string | null, fresh: boolean, focusReasoning?: boolean) => void
  showHistoryTarget: (conversation: Conversation, messageId: string) => void
  reportHistoryError: (conversationId: string, message: string | null) => void
  reportConversationChanged?: (conversationId: string) => void
  showConversation: (conversation: Conversation, context: {
    renderRequestId: number
    retained?: boolean
    selection: boolean
    focusMessageId?: string
    focusReasoning?: boolean
  }) => void
  resetConversation: () => void
  discardConversation: (conversationId: string, error: Error, selection: boolean) => void
}

interface ReloadOptions {
  embedded?: boolean
  force?: boolean
  transitionRequestId?: number
  loadPoppedOut?: boolean
  initialWindow?: boolean
  /** An execution permit may expire while the read is pending. */
  canCommit?: () => boolean
}

function asError(value: unknown, fallback = '对话加载失败，已从列表移除'): Error {
  if (value instanceof Error) return value
  return new Error(typeof value === 'string' ? value : fallback)
}

/** Owns navigation commit rights. Backend runs are deliberately not cancelled
 * when a view changes: only an obsolete UI result loses its lease. */
export function createChatNavigationController(ports: NavigationPorts) {
  const readInitialWindow = (conversationId: string, requestId?: number, anchorMessageId?: string) =>
    ports.readConversationWindow?.(conversationId, (candidate) => {
      if (!candidate || requestId === undefined || !isCurrentConversationTransition(requestId, conversationId)) return
      ports.showConversation(candidate, { renderRequestId: requestId, selection: true, retained: true })
      setHash(conversationHash(conversationId))
    }, anchorMessageId) ?? ports.readConversation(conversationId)

  const { readAndShowConversation, loadHistoryPage, focusHistoryMessage } = createChatHistoryReader({
    ...ports,
    readWindow: (id, anchor) => readInitialWindow(id, undefined, anchor),
    captureView: () => {
      const lease = captureConversationNavigation()
      return () => isCurrentConversationNavigation(lease)
    },
  })
  const beginConversationCreation = () => {
    // A creation is a navigation intent even while its backend request is
    // pending. Revoking the previous generation also orders two creations
    // started from the same empty route.
    invalidateConversationTransition()
    return {
      navigation: captureConversationNavigation(),
      startingConversationId: ports.currentConversationId(),
      startingPath: hashPath(),
    }
  }

  const isConversationCreationCurrent = (permit: ReturnType<typeof beginConversationCreation>) =>
    isCurrentConversationNavigation(permit.navigation)
    && ports.currentConversationId() === permit.startingConversationId
    && hashPath() === permit.startingPath

  const commitCreatedConversation = (
    permit: ReturnType<typeof beginConversationCreation>, conversation: Conversation,
  ): boolean => {
    if (!isConversationCreationCurrent(permit)) return false
    if (permit.startingConversationId === null) migrateNewChatDraft(draftKey(null), conversation.id)
    ports.showConversation(conversation, { renderRequestId: 0, selection: true })
    syncConversationRoute(conversation.id)
    return true
  }

  const syncConversationRoute = (conversationId: string | null) => {
    if (!conversationId) invalidateConversationTransition()
    setHash(conversationHash(conversationId))
  }

  const leaveConversation = () => invalidateConversationTransition()

  const resetRouteConversation = () => {
    leaveConversation()
    ports.resetConversation()
  }

  const startNewConversation = () => {
    // Revoke the old route before any view projection can synchronously notify
    // subscribers or start a replacement load.
    leaveConversation()
    ports.prepareNewConversation()
    forgetRememberedChatRoute()
    syncConversationRoute(null)
  }

  const clearCurrentChat = async () => {
    const conversationId = ports.currentConversationId()
    if (!conversationId) {
      ports.clearEmptyChat()
      return
    }
    // A rejected clear must not revoke a pending navigation's commit right.
    const decision = await ports.requestClearChat(conversationId)
    if (decision === 'busy') {
      ports.reportClearError(conversationId, '请先停止当前回复，再清空对话。')
      return
    }
    if (decision !== 'confirmed') return

    const navigationLease = captureConversationNavigation()
    try {
      await ports.deleteConversation(conversationId)
    } catch (value) {
      ports.reportClearError(conversationId, asError(value, '清空对话失败').message || '清空对话失败')
      return
    }

    // Deletion has committed remotely. Local settlement must not depend on the
    // best-effort cancellation request that follows it.
    const cancelRun = ports.isConversationInFlight(conversationId)
    const transition = getConversationTransitionSnapshot()
    const clearCurrentView = isCurrentConversationNavigation(navigationLease)
      && (!transition.loading || transition.targetConversationId === conversationId)
      && ports.currentConversationId() === conversationId
    ports.finalizeDeletedChat(conversationId, clearCurrentView)
    if (clearCurrentView) {
      forgetRememberedChatRoute()
      syncConversationRoute(null)
    }
    if (cancelRun) {
      try {
        await ports.cancelDeletedRun(conversationId)
      } catch (value) {
        console.warn('Failed to cancel a deleted conversation run:', value)
      }
    }
  }

  // A retained preview has already changed both the route and the composer scope.
  // If its revision cannot be verified, leave that view with its draft still keyed by id.
  const finishStaleRead = (id: string, requestId?: number, embedded = false) => {
    if (requestId !== undefined) cancelConversationTransition(requestId)
    const displayed = ports.currentConversationId()
    const tentative = displayed && ports.isResumingConversation?.(displayed)
    if (tentative) ports.resetConversation()
    if (!embedded && (tentative || getRouteConversationId() === id)) syncConversationRoute(ports.currentConversationId())
    ports.reportConversationChanged?.(id)
  }

  const reloadConversation = async (conversationId: string, options?: ReloadOptions) => {
    const transitionRequestId = options?.transitionRequestId
    const navigationLease = captureConversationNavigation()
    const startingConversationId = ports.currentConversationId()
    const canCommitResult = () => {
      if (options?.canCommit && !options.canCommit()) return false
      if (transitionRequestId !== undefined) {
        return isCurrentConversationTransition(transitionRequestId, conversationId)
      }
      return isCurrentConversationNavigation(navigationLease)
        && ports.currentConversationId() === conversationId
        && startingConversationId === conversationId
    }
    const ownership = await awaitCurrentConversationNavigation(
      options?.loadPoppedOut ? Promise.resolve(new Set<string>()) : ports.listPopouts(),
      canCommitResult,
    )
    if (ownership.status === 'stale') return
    if (ownership.value.has(conversationId)) {
      ports.occupyPopout(conversationId)
      if (transitionRequestId !== undefined) {
        completeConversationTransition(conversationId, transitionRequestId)
      }
      return
    }
    if (ports.isConversationInFlight(conversationId) && !options?.force) return
    try {
      // A run appends drafts to its own journal; the stored history a window
      // is cut from is the same whether or not the conversation is running.
      const read = options?.initialWindow
        ? (id: string) => readInitialWindow(id, options.embedded ? undefined : transitionRequestId)
        : ports.readConversation
      const outcome = await readAndShowConversation(conversationId, read, () => {
        const transition = getConversationTransitionSnapshot()
        return canCommitResult() && (!transition.loading || transition.targetConversationId === conversationId)
      }, (conversation) => {
        const transition = getConversationTransitionSnapshot()
        const renderRequestId = transitionRequestId
          ?? (transition.loading && transition.targetConversationId === conversationId ? transition.requestId : 0)
        ports.showConversation(conversation, { renderRequestId, selection: false })
        if (renderRequestId > 0 && conversation.messages.length === 0) {
          window.requestAnimationFrame(() => completeConversationTransition(conversationId, renderRequestId))
        }
      })
      if (outcome === 'changed' && canCommitResult()) {
        finishStaleRead(conversationId, transitionRequestId, options?.embedded)
      }
    } catch (value) {
      const transition = getConversationTransitionSnapshot()
      if (!canCommitResult() || (transition.loading && transition.targetConversationId !== conversationId)) return
      if (options?.embedded) {
        if (transitionRequestId !== undefined) cancelConversationTransition(transitionRequestId)
        throw asError(value)
      }
      ports.discardConversation(conversationId, asError(value), false)
      forgetRememberedChatRoute()
      syncConversationRoute(null)
      if (transitionRequestId !== undefined) cancelConversationTransition(transitionRequestId)
      else if (transition.loading && transition.targetConversationId === conversationId) {
        cancelConversationTransition(transition.requestId)
      }
    }
  }

  const loadRouteConversation = (conversationId: string) => {
    const requestId = beginConversationTransition(conversationId)
    return reloadConversation(conversationId, { force: true, transitionRequestId: requestId, initialWindow: true })
  }

  const loadEmbeddedConversation = (conversationId: string) => {
    const requestId = beginConversationTransition(conversationId)
    return reloadConversation(conversationId, { force: true, transitionRequestId: requestId, initialWindow: true, embedded: true })
  }

  const openConversation = (conversationId: string, options?: { reload?: boolean | null }) => {
    if (getRouteConversationId() !== conversationId) {
      syncConversationRoute(conversationId)
      return Promise.resolve()
    }
    if (options?.reload === false) return Promise.resolve()
    const requestId = beginConversationTransition(conversationId)
    return reloadConversation(conversationId, { force: true, transitionRequestId: requestId })
  }

  let selectionSequence = 0
  const selectConversation = async (conversationId: string, hint?: ConversationLoadHint) => {
    const selection = ++selectionSequence
    const current = ports.currentConversation()
    const alreadyOpen = ports.currentConversationId() === conversationId
      && current?.id === conversationId
      && !ports.isResumingConversation?.(conversationId)
      && (!hint?.focusMessageId || current.messages.some(message => message.id === hint.focusMessageId))
    if (alreadyOpen) {
      if (getConversationTransitionSnapshot().loading) leaveConversation()
      const navigationLease = captureConversationNavigation()
      const ownership = await awaitCurrentConversationNavigation(
        ports.listPopouts(),
        () => selection === selectionSequence && isCurrentConversationNavigation(navigationLease),
      )
      if (ownership.status === 'stale') return
      if (ownership.value.has(conversationId)) {
        ports.focusPopout(conversationId)
        ports.occupyPopout(conversationId)
        return
      }
      ports.prepareSelection(hint?.focusMessageId ?? null, false, hint?.focusReasoning)
      syncConversationRoute(conversationId)
      return
    }
    const requestId = beginConversationTransition(conversationId)
    const isCurrent = () => selection === selectionSequence && isCurrentConversationTransition(requestId, conversationId)
    const ownership = await awaitCurrentConversationNavigation(
      ports.listPopouts(),
      isCurrent,
    )
    if (ownership.status === 'stale') return
    if (ownership.value.has(conversationId)) {
      ports.focusPopout(conversationId)
      ports.occupyPopout(conversationId)
      return
    }
    ports.prepareSelection(null, true)
    try {
      const read = (id: string) => readInitialWindow(id, requestId, hint?.focusMessageId)
      const outcome = await readAndShowConversation(conversationId, read, isCurrent,
        (conversation) => {
          ports.showConversation(conversation, {
            renderRequestId: requestId, selection: true,
            focusMessageId: hint?.focusMessageId, focusReasoning: hint?.focusReasoning,
          })
          if (conversation.messages.length === 0) {
            window.requestAnimationFrame(() => completeConversationTransition(conversationId, requestId))
          }
          syncConversationRoute(conversationId)
        })
      if (outcome === 'changed' && isCurrent()) {
        finishStaleRead(conversationId, requestId)
      }
    } catch (value) {
      if (!isCurrent()) return
      ports.discardConversation(conversationId, asError(value), true)
      forgetRememberedChatRoute()
      syncConversationRoute(null)
      cancelConversationTransition(requestId)
    }
  }

  const reconcilePopouts = async (previous: ReadonlySet<string>, next: ReadonlySet<string>) => {
    const pending = getConversationTransitionSnapshot()
    const target = pending.targetConversationId
    if (pending.loading && target && next.has(target) && !previous.has(target)) {
      ports.occupyPopout(target)
      return
    }
    const currentId = ports.currentConversationId()
    if (!currentId) return
    if (next.has(currentId) && !previous.has(currentId)) {
      ports.occupyPopout(currentId)
    } else if (previous.has(currentId) && !next.has(currentId)) {
      await reloadConversation(currentId, { force: true, loadPoppedOut: true })
    }
  }

  return {
    loadHistoryPage,
    focusHistoryMessage,
    beginConversationCreation,
    isConversationCreationCurrent,
    commitCreatedConversation,
    leaveConversation,
    startNewConversation,
    clearCurrentChat,
    resetRouteConversation,
    loadRouteConversation,
    loadEmbeddedConversation,
    openConversation,
    reloadConversation,
    selectConversation,
    reconcilePopouts,
    syncConversationRoute,
  }
}
