import { describe, expect, it } from 'vitest'
import { keepNewerTodoState, patchTodoState } from './agentTodoState'
import type { AgentTodoState, Conversation } from './types'

const conversation = (revision: number, todo: AgentTodoState): Conversation => ({
  id: 'c', title: 't', revision, messages: [], created_at: 0, updated_at: 0,
  agent_todo_state: todo, agentTodoState: todo,
} as unknown as Conversation)

const oldList: AgentTodoState = { items: [{ id: '1', content: 'a', status: 'in_progress' }] }
const newList: AgentTodoState = { items: [{ id: '1', content: 'a', status: 'completed' }, { id: '2', content: 'b', status: 'in_progress' }] }

describe('agent todo state', () => {
  it('keeps a live list over a refetch read before that write', () => {
    const live = patchTodoState(conversation(4, oldList), newList, 5)
    const staleRefetch = conversation(4, oldList)
    expect(keepNewerTodoState(staleRefetch, live).agentTodoState).toEqual(newList)
  })

  it('accepts the refetch once it reaches the write revision', () => {
    const live = patchTodoState(conversation(4, oldList), newList, 5)
    const fresh = conversation(6, { items: [] })
    expect(keepNewerTodoState(fresh, live).agentTodoState).toEqual({ items: [] })
  })

  it('does not hold a list from a run event without a revision', () => {
    const live = patchTodoState(conversation(4, oldList), newList)
    expect(keepNewerTodoState(conversation(4, oldList), live).agentTodoState).toEqual(oldList)
  })
})
