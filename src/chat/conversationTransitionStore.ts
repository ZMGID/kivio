import { useSyncExternalStore } from 'react'
import type { Conversation } from './types'
import { measureChatSurface } from './chatPerformanceProbe'

export interface ConversationTransitionSnapshot {
  requestId: number
  targetConversationId: string | null
  loading: boolean
  preview?: Conversation
}

export interface ConversationLoadHint {
  /** 全局搜索跳转：打开会话后滚到这条消息并短暂高亮。 */
  focusMessageId?: string
  focusReasoning?: boolean
}

export interface ConversationNavigationLease {
  requestId: number
  targetConversationId: string | null
}

let requestSequence = 0
let snapshot: ConversationTransitionSnapshot = {
  requestId: 0,
  targetConversationId: null,
  loading: false,
}

const listeners = new Set<() => void>()
// Click-to-first-commit timing for the newest transition; superseded ones are never reported.
let finishSwitchSample: (() => void) | null = null

function emit() {
  for (const listener of listeners) listener()
}

export function beginConversationTransition(conversationId: string): number {
  const requestId = ++requestSequence
  snapshot = { requestId, targetConversationId: conversationId, loading: true }
  finishSwitchSample = measureChatSurface('conversation-switch', null, conversationId)
  emit()
  return requestId
}

export function completeConversationTransition(conversationId: string, requestId: number) {
  if (
    snapshot.requestId !== requestId
    || snapshot.targetConversationId !== conversationId
    || !snapshot.loading
  ) return
  snapshot = { requestId, targetConversationId: conversationId, loading: false }
  finishSwitchSample?.()
  finishSwitchSample = null
  emit()
}

/** A display-only window, automatically revoked by navigation or completion. */
export function previewConversationTransition(requestId: number, conversation: Conversation | undefined) {
  if (snapshot.requestId !== requestId || !snapshot.loading || snapshot.preview === conversation) return
  if (conversation && snapshot.targetConversationId !== conversation.id) return
  snapshot = { ...snapshot, preview: conversation }
  emit()
}

export function cancelConversationTransition(requestId: number) {
  if (snapshot.requestId !== requestId) return
  snapshot = { requestId, targetConversationId: null, loading: false }
  emit()
}

export function invalidateConversationTransition() {
  const requestId = ++requestSequence
  snapshot = { requestId, targetConversationId: null, loading: false }
  emit()
}

export function isCurrentConversationTransition(requestId: number, conversationId: string): boolean {
  return snapshot.requestId === requestId && snapshot.targetConversationId === conversationId
}

/** Capture the current navigation generation for background refreshes that do
 * not start a new transition. Leaving the current route invalidates the lease
 * without cancelling the conversation's backend execution. */
export function captureConversationNavigation(): ConversationNavigationLease {
  return {
    requestId: snapshot.requestId,
    targetConversationId: snapshot.targetConversationId,
  }
}

export function isCurrentConversationNavigation(lease: ConversationNavigationLease): boolean {
  return snapshot.requestId === lease.requestId
    && snapshot.targetConversationId === lease.targetConversationId
}

export type ConversationNavigationResult<T> =
  | { status: 'current'; value: T }
  | { status: 'stale' }

/** Await an asynchronous navigation prerequisite and validate ownership again
 * after it settles. The work itself is not cancelled; only its UI commit right
 * expires when navigation moves elsewhere. */
export async function awaitCurrentConversationNavigation<T>(
  pending: Promise<T>,
  isCurrent: () => boolean,
): Promise<ConversationNavigationResult<T>> {
  const value = await pending
  return isCurrent() ? { status: 'current', value } : { status: 'stale' }
}

export function getConversationTransitionSnapshot(): ConversationTransitionSnapshot {
  return snapshot
}

function subscribe(listener: () => void) {
  listeners.add(listener)
  return () => { listeners.delete(listener) }
}

const getPreview = () => snapshot.preview

export function usePendingConversationPreview() {
  return useSyncExternalStore(subscribe, getPreview, getPreview)
}

export function useConversationTransition(): ConversationTransitionSnapshot {
  return useSyncExternalStore(
    subscribe,
    getConversationTransitionSnapshot,
    getConversationTransitionSnapshot,
  )
}
