import { describe, expect, it } from 'vitest'
import { clearConversationLocalState, type ConversationLocalState } from './conversationLocalState'

function makeState(): ConversationLocalState {
  return {
    streamSnapshots: { c1: { content: 'a' } as never, c2: { content: 'b' } as never },
    streamErrors: { c1: 'err1', c2: 'err2' },
    pendingToolConfirms: { c1: [{ conversationId: 'c1' }] as never, c2: [] },
    pendingSessionConsents: { c1: { conversationId: 'c1' } as never, c2: {} as never },
    pendingUserPrompts: { c1: [{ conversationId: 'c1' }] as never, c2: [] },
  }
}

describe('clearConversationLocalState', () => {
  it('clears only the target conversation presentation and pending prompts by default', () => {
    const state = makeState()
    clearConversationLocalState(state, 'c1')
    expect(state.streamSnapshots.c1).toBeUndefined()
    expect(state.pendingToolConfirms.c1).toBeUndefined()
    expect(state.pendingSessionConsents.c1).toBeUndefined()
    expect(state.pendingUserPrompts.c1).toBeUndefined()
    expect(state.streamErrors.c1).toBe('err1')
    expect(state.streamSnapshots.c2).toBeDefined()
  })

  it('clears a target error only when explicitly requested', () => {
    const state = makeState()
    clearConversationLocalState(state, 'c1', { streamErrors: true })
    expect(state.streamErrors.c1).toBeUndefined()
    expect(state.streamErrors.c2).toBe('err2')
  })

  it('is safe to clear a missing conversation more than once', () => {
    const state = makeState()
    clearConversationLocalState(state, 'missing', { streamErrors: true })
    clearConversationLocalState(state, 'missing', { streamErrors: true })
    expect(state.streamSnapshots.c1).toBeDefined()
  })
})
