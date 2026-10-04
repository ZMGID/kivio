import { describe, expect, it } from 'vitest'
import { createLiveRowModel } from './liveRowModel'

function sync(
  model: ReturnType<typeof createLiveRowModel>,
  partial: Partial<Parameters<ReturnType<typeof createLiveRowModel>['sync']>[0]> & {
    liveActive: boolean
  },
) {
  return model.sync({
    conversationId: 'c1',
    liveGroupId: null,
    preferredTwinId: null,
    historyAssistantIds: [],
    historyGroupIds: [],
    ...partial,
  })
}

describe('createLiveRowModel', () => {
  it.each([true, false])('remembers a recovered draft id before snapshot clearing (known at start: %s)', (knownAtStart) => {
    const model = createLiveRowModel()
    const historyAssistantIds = ['older', 'recovered']
    const { liveKey } = sync(model, {
      liveActive: true,
      preferredTwinId: knownAtStart ? 'recovered' : null,
      historyAssistantIds,
    })
    sync(model, { liveActive: true, preferredTwinId: 'recovered', historyAssistantIds })
    // Completion clears the stream store; persistence replaces the existing
    // draft, so the assistant count does not increase.
    sync(model, { liveActive: false, historyAssistantIds })
    expect(model.resolveMessageKey('recovered')).toBe(liveKey)
    expect(model.resolveMessageKey('older')).toBe('older')

    const next = sync(model, { liveActive: true, historyAssistantIds })
    sync(model, { liveActive: false, historyAssistantIds: [...historyAssistantIds, 'next'] })
    expect(model.resolveMessageKey('next')).toBe(next.liveKey)
    expect(model.resolveMessageKey('recovered')).toBe(liveKey)
  })

  it('persist lag: alias still lands when history commits a build later', () => {
    const model = createLiveRowModel()

    const streaming = sync(model, { liveActive: true })
    const liveKey = streaming.liveKey!

    // Run ended but twin has not landed yet.
    sync(model, {
      liveActive: false,
      historyAssistantIds: [],
    })
    expect(model.resolveMessageKey('a1')).toBe('a1')

    sync(model, {
      liveActive: false,
      preferredTwinId: 'a1',
      historyAssistantIds: ['a1'],
    })
    expect(model.resolveMessageKey('a1')).toBe(liveKey)
  })

  it('a new turn supersedes an unresolved settle so aliases never cross turns', () => {
    const model = createLiveRowModel()

    sync(model, { liveActive: true })
    sync(model, { liveActive: false, historyAssistantIds: [] }) // twin never landed

    const second = sync(model, {
      liveActive: true,
      historyAssistantIds: [],
    })
    const secondLiveKey = second.liveKey!

    sync(model, {
      liveActive: false,
      preferredTwinId: 'a2',
      historyAssistantIds: ['a2'],
    })
    expect(model.resolveMessageKey('a2')).toBe(secondLiveKey)
  })

  it('conversation switch drops aliases', () => {
    const model = createLiveRowModel()

    const streaming = sync(model, {
      conversationId: 'c1',
      liveActive: true,
    })
    const liveKey = streaming.liveKey!
    sync(model, {
      conversationId: 'c1',
      liveActive: false,
      preferredTwinId: 'a1',
      historyAssistantIds: ['a1'],
    })
    model.sync({
      conversationId: 'c1',
      liveActive: false,
      liveGroupId: null,
      preferredTwinId: null,
      historyAssistantIds: ['a1'],
      historyGroupIds: [],
    })
    expect(model.resolveMessageKey('a1')).toBe(liveKey)

    // New conversation:
    const next = model.sync({
      conversationId: 'c2',
      liveActive: true,
      liveGroupId: null,
      preferredTwinId: null,
      historyAssistantIds: [],
      historyGroupIds: [],
    })
    expect(next.liveKey).toMatch(/^live-turn-/)
    expect(model.resolveMessageKey('a1')).toBe('a1')
  })
})
