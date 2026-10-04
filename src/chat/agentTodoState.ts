import type { AgentTodoState, Conversation } from './types'

/**
 * The todo list arrives by its own event, ahead of the conversation refetches that
 * were already in flight. `todoRevision` records which conversation revision the
 * live list belongs to, so a refetch read before that write keeps the newer list
 * instead of rolling it back. Only the list is protected; the rest of the refetch
 * still applies.
 */
export type ConversationWithTodoRevision = Conversation & { todoRevision?: number }

export function patchTodoState<T extends Conversation>(
  conversation: T,
  todoState: AgentTodoState,
  revision?: number,
): T {
  return {
    ...conversation,
    agent_todo_state: todoState,
    agentTodoState: todoState,
    todoRevision: revision ?? (conversation as ConversationWithTodoRevision).todoRevision,
  }
}

export function keepNewerTodoState<T extends Conversation>(incoming: T, previous: Conversation | null): T {
  const todoRevision = (previous as ConversationWithTodoRevision | null)?.todoRevision
  if (!previous || previous.id !== incoming.id || todoRevision === undefined) return incoming
  if (incoming.revision >= todoRevision) return incoming
  const todoState = previous.agent_todo_state ?? previous.agentTodoState
  return todoState ? patchTodoState(incoming, todoState, todoRevision) : incoming
}
