import { useCallback, useSyncExternalStore } from 'react'
import { draftKey, getComposerDraft, subscribeComposerDraft, updateComposerDraft } from './composerDraft'
import { findComposerCommands } from './slashCommands'
import type { ChatMessage, GoalState } from './types'

const goalCommands = [{ slash: '/goal' }]

/** The pending mode is derived from the sendable draft, never a second mode state. */
export function useGoalDraft(conversationId?: string | null): boolean {
  const key = draftKey(conversationId)
  const snapshot = useCallback(
    () => findComposerCommands(getComposerDraft(key)?.input ?? '', goalCommands).length > 0,
    [key],
  )
  return useSyncExternalStore(subscribeComposerDraft, snapshot)
}

export function setGoalDraftMode(conversationId: string | null | undefined, enabled: boolean): void {
  updateComposerDraft(draftKey(conversationId), (draft) => {
    let input = draft.input
    for (const match of findComposerCommands(input, goalCommands).reverse()) {
      input = input.slice(0, match.start) + input.slice(match.end).replace(/^[ \t]/, '')
    }
    return { ...draft, input: enabled ? `/goal ${input}` : input }
  })
}

export function goalCompletedAt(goal: GoalState): number | undefined {
  if (goal.status !== 'completed') return undefined
  // Older saved goals predate the dedicated completion timestamp.
  return goal.completed_at ?? goal.completedAt ?? goal.updated_at ?? goal.updatedAt
}

/** Keep the completion receipt until the next user turn, without deleting Goal history. */
export function composerGoal(
  goal: GoalState | null | undefined,
  messages: readonly Pick<ChatMessage, 'id' | 'role' | 'timestamp'>[],
): GoalState | undefined {
  if (!goal) return undefined
  if (goal.status !== 'completed' && goal.status !== 'cancelled') return goal
  const completionMessageId = goal.completed_message_id ?? goal.completedMessageId
  const completionIndex = completionMessageId
    ? messages.findIndex((message) => message.id === completionMessageId)
    : -1
  if (completionIndex >= 0) {
    return messages.slice(completionIndex + 1).some((message) => message.role === 'user')
      ? undefined : goal
  }
  const finishedAt = goalCompletedAt(goal) ?? goal.updated_at ?? goal.updatedAt
  return finishedAt != null && messages.some((message) => message.role === 'user' && message.timestamp > finishedAt)
    ? undefined : goal
}
