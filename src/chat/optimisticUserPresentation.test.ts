import { describe, expect, it } from 'vitest'
import { createOptimisticUserPresentation } from './optimisticUserPresentation'
import type { ChatMessage } from './types'

const stored = (id: string, content: string, timestamp: number): ChatMessage => ({
  id, role: 'user', content, timestamp,
})

describe('optimistic user presentation', () => {
  it('shows an accepted send only in its conversation until the persisted twin arrives', () => {
    const owner = createOptimisticUserPresentation()
    const token = owner.begin('a', 'hello', [], 100_000)
    expect(owner.overlay('a', [])).toEqual([token.message])
    expect(owner.overlay('b', [])).toEqual([])
    expect(owner.overlay('a', [stored(token.message.id, 'hello', 100)])).toHaveLength(1)
  })

  it('does not let a late settle from a previous send clear the next send', () => {
    const owner = createOptimisticUserPresentation()
    const first = owner.begin('a', 'first', [], 100_000)
    const second = owner.begin('a', 'second', [], 101_000)
    owner.settle('a', first.token)
    expect(owner.overlay('a', [])).toEqual([second.message])
    owner.settle('a', second.token)
    expect(owner.overlay('a', [])).toEqual([])
  })

  it('keeps a background send visible if the view navigates away and back', () => {
    const owner = createOptimisticUserPresentation()
    owner.begin('a', 'background', [], 100_000)
    expect(owner.overlay('b', [])).toEqual([])
    expect(owner.overlay('a', [])).toMatchObject([{ content: 'background' }])
  })

  it('does not mistake an earlier identical user message for the new send', () => {
    const owner = createOptimisticUserPresentation()
    const previous = stored('previous', 'again', 100)
    const pending = owner.begin('a', 'again', [], 101_000)
    expect(owner.overlay('a', [previous])).toEqual([previous, pending.message])
    expect(owner.overlay('a', [previous, stored('new', 'again', 101)])).toHaveLength(3)
    expect(owner.overlay('a', [previous, stored(pending.message.id, 'again', 101)])).toHaveLength(2)
  })

  it('keeps a repeated prompt with different attachments until its own id is stored', () => {
    const owner = createOptimisticUserPresentation()
    const older: ChatMessage = { ...stored('older', 'inspect', 100), attachments: [
      { id: 'a', type: 'file', name: 'old.txt', path: 'old.txt' },
    ] }
    const pending = owner.begin('a', 'inspect', [
      { id: 'b', type: 'file', name: 'new.txt', path: 'new.txt' },
    ], 101_000)
    expect(owner.overlay('a', [older])).toHaveLength(2)
    expect(owner.overlay('a', [older, stored('unrelated', 'inspect', 101)])).toHaveLength(3)
    expect(owner.overlay('a', [older, stored(pending.message.id, 'inspect', 101)])).toHaveLength(2)
  })
})
