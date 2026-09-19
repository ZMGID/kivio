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
import type { Conversation } from './types'

interface NavigationPorts {
  currentConversation: () => Conversation | null
  currentConversationId: () => string | null
  listPopouts: () => Promise<ReadonlySet<string>>
  readConversation: (conversationId: string) => Promise<Conversation>
  isConversationInFlight: (conversationId: string) => boolean
  prepareNewConversation: () => void
  clearEmptyChat: () => void
  /** Check busy state before asking for confirmation; no route mutation here. */
  requestClearChat: (conversationId: string) => 'busy' | 'cancelled' | 'confirmed'
  deleteConversation: (conversationId: string) => Promise<void>
  cancelDeletedRun: (conversationId: string) => Promise<void>
  /** Synchronously drop local execution state, optionally clear this view, and refresh the list. */
  finalizeDeletedChat: (conversationId: string, clearCurrentView: boolean) => void
  reportClearError: (conversationId: string, message: string) => void
  focusPopout: (conversationId: string) => void
  occupyPopout: (conversationId: string) => void
  prepareSelection: (focusMessageId: string | null, fresh: boolean) => void
  showConversation: (conversation: Conversation, context: {
    renderRequestId: number
    selection: boolean
  }) => void
  resetConversation: () => void
  discardConversation: (conversationId: string, error: Error, selection: boolean) => void
}

interface ReloadOptions {
  force?: boolean
  transitionRequestId?: number
  loadPoppedOut?: boolean
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
    const decision = ports.requestClearChat(conversationId)
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
      const conversation = await ports.readConversation(conversationId)
      const transition = getConversationTransitionSnapshot()
      if (!canCommitResult() || (transition.loading && transition.targetConversationId !== conversationId)) return
      const renderRequestId = transitionRequestId
        ?? (transition.loading && transition.targetConversationId === conversationId ? transition.requestId : 0)
      ports.showConversation(conversation, { renderRequestId, selection: false })
      if (renderRequestId > 0 && conversation.messages.length === 0) {
        window.requestAnimationFrame(() => completeConversationTransition(conversationId, renderRequestId))
      }
    } catch (value) {
      const transition = getConversationTransitionSnapshot()
      if (!canCommitResult() || (transition.loading && transition.targetConversationId !== conversationId)) return
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
    return reloadConversation(conversationId, { force: true, transitionRequestId: requestId })
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

  const selectConversation = async (conversationId: string, hint?: ConversationLoadHint) => {
    const alreadyOpen = ports.currentConversationId() === conversationId
      && ports.currentConversation()?.id === conversationId
    if (alreadyOpen) {
      const inFlight = getConversationTransitionSnapshot()
      if (inFlight.loading && inFlight.targetConversationId !== conversationId) leaveConversation()
      const navigationLease = captureConversationNavigation()
      const ownership = await awaitCurrentConversationNavigation(
        ports.listPopouts(),
        () => isCurrentConversationNavigation(navigationLease),
      )
      if (ownership.status === 'stale') return
      if (ownership.value.has(conversationId)) {
        ports.focusPopout(conversationId)
        ports.occupyPopout(conversationId)
        return
      }
      ports.prepareSelection(hint?.focusMessageId ?? null, false)
      syncConversationRoute(conversationId)
      return
    }
    const requestId = beginConversationTransition(conversationId, hint)
    const ownership = await awaitCurrentConversationNavigation(
      ports.listPopouts(),
      () => isCurrentConversationTransition(requestId, conversationId),
    )
    if (ownership.status === 'stale') return
    if (ownership.value.has(conversationId)) {
      ports.focusPopout(conversationId)
      ports.occupyPopout(conversationId)
      return
    }
    ports.prepareSelection(hint?.focusMessageId ?? null, true)
    try {
      const conversation = await ports.readConversation(conversationId)
      if (!isCurrentConversationTransition(requestId, conversationId)) return
      ports.showConversation(conversation, { renderRequestId: requestId, selection: true })
      if (conversation.messages.length === 0) {
        window.requestAnimationFrame(() => completeConversationTransition(conversationId, requestId))
      }
      syncConversationRoute(conversationId)
    } catch (value) {
      if (!isCurrentConversationTransition(requestId, conversationId)) return
      ports.discardConversation(conversationId, asError(value), true)
      forgetRememberedChatRoute()
      syncConversationRoute(null)
      cancelConversationTransition(requestId)
    }
  }

  const reconcilePopouts = async (previous: ReadonlySet<string>, next: ReadonlySet<string>) => {
    const currentId = ports.currentConversationId()
    if (!currentId) return
    if (next.has(currentId) && !previous.has(currentId)) {
      ports.occupyPopout(currentId)
    } else if (previous.has(currentId) && !next.has(currentId)) {
      await reloadConversation(currentId, { force: true, loadPoppedOut: true })
    }
  }

  return {
    beginConversationCreation,
    isConversationCreationCurrent,
    commitCreatedConversation,
    leaveConversation,
    startNewConversation,
    clearCurrentChat,
    resetRouteConversation,
    loadRouteConversation,
    openConversation,
    reloadConversation,
    selectConversation,
    reconcilePopouts,
    syncConversationRoute,
  }
}
