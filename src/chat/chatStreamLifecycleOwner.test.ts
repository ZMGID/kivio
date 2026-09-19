import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { ChatStreamPayload } from '../api/tauri'
import { createChatExecutionOwner } from './chatExecutionOwner'
import { createChatPopoutOwnershipOwner } from './chatPopoutOwnershipOwner'
import { createChatStreamLifecycleOwner } from './chatStreamLifecycleOwner'
import { beginConversationTransition, captureConversationNavigation, isCurrentConversationNavigation } from './conversationTransitionStore'
import { getActiveGroup, resetGroups } from './groupStreamingStore'
import { createStreamPreviewOwner } from './streamPreviewOwner'
import { reset as resetStreamStore } from './streamingStore'

const packet = (runId: string, type: string, recovery?: {
  groupId: string; groupSize: number; armIndex: number
}): ChatStreamPayload => ({
  conversationId: 'a', runId, messageId: `message-${runId}`, type,
  recovery: recovery ? { ...recovery, providerId: 'p', model: 'm' } : null,
} as ChatStreamPayload)

describe('chat stream lifecycle owner', () => {
  beforeEach(() => {
    resetGroups()
    resetStreamStore()
  })

  it('defers a local terminal while its invoke has not settled', () => {
    const execution = createChatExecutionOwner()
    const preview = createStreamPreviewOwner()
    const owner = createChatStreamLifecycleOwner(execution, preview)
    execution.begin({ conversationId: 'a', kind: 'send', startedAt: 1 })
    preview.begin('a', 1)
    expect(owner.receive(packet('local', 'run_started'))).toMatchObject({ kind: 'started' })
    expect(owner.receive(packet('local', 'run_completed'))).toMatchObject({
      kind: 'deferred', terminal: { turnEpoch: execution.turnEpoch('a') },
    })
    expect(execution.snapshot('a').inFlight).toBe(true)
    preview.dispose()
  })

  it('cancels a recovered run once, fences late content, and still accepts its terminal', async () => {
    const execution = createChatExecutionOwner()
    const preview = createStreamPreviewOwner()
    const owner = createChatStreamLifecycleOwner(execution, preview)
    preview.activate('a')
    owner.receive(packet('run-a', 'run_started'))
    let resolveCancel: () => void = () => {}
    const cancel = vi.fn(() => new Promise<void>((resolve) => { resolveCancel = resolve }))
    const pending = vi.fn()
    const cancelling = owner.cancelRun('a', cancel, pending)

    expect(pending).toHaveBeenCalledTimes(1)
    expect(cancel).toHaveBeenCalledTimes(1)
    expect(preview.summary('a')?.streaming).toBe(false)
    expect(owner.receive(packet('run-a', 'text_delta')).kind).toBe('ignored')
    expect((await owner.cancelRun('a', cancel)).kind).toBe('ignored')

    resolveCancel()
    expect((await cancelling).kind).toBe('cancelled')
    expect(owner.receive(packet('run-a', 'run_cancelled')).kind).toBe('ready')
    preview.dispose()
  })

  it('reopens the same run after backend cancellation fails', async () => {
    const execution = createChatExecutionOwner()
    const preview = createStreamPreviewOwner()
    const owner = createChatStreamLifecycleOwner(execution, preview)
    preview.activate('a')
    owner.receive(packet('run-a', 'run_started'))
    const result = await owner.cancelRun('a', async () => { throw new Error('transport failed') })

    expect(result).toMatchObject({ kind: 'failed', error: new Error('transport failed') })
    expect(preview.summary('a')?.streaming).toBe(true)
    expect(execution.allowsStreamPayload(packet('run-a', 'text_delta'))).toBe(true)
    preview.dispose()
  })

  it('waits for every recovered group arm before allowing one terminal settlement', () => {
    const execution = createChatExecutionOwner()
    const preview = createStreamPreviewOwner()
    const owner = createChatStreamLifecycleOwner(execution, preview)
    expect(owner.receive(packet('one', 'run_started', { groupId: 'group-a', groupSize: 2, armIndex: 0 })))
      .toMatchObject({ kind: 'started' })
    expect(owner.receive(packet('two', 'run_started', { groupId: 'group-a', groupSize: 2, armIndex: 1 })))
      .toMatchObject({ kind: 'started' })
    expect(owner.receive(packet('one', 'run_completed'))).toMatchObject({ kind: 'pending' })
    const final = owner.receive(packet('two', 'run_completed'))
    expect(final.kind).toBe('ready')
    expect(execution.snapshot('a').inFlight).toBe(true)
    preview.dispose()
  })

  it('counts hidden popout group arms even when the window docks between terminals', async () => {
    const execution = createChatExecutionOwner()
    const preview = createStreamPreviewOwner()
    const owner = createChatStreamLifecycleOwner(execution, preview)
    const hidden = { project: false }
    expect(owner.receive(packet('one', 'run_started', { groupId: 'popped', groupSize: 2, armIndex: 0 }), hidden).kind).toBe('started')
    expect(owner.receive(packet('two', 'run_started', { groupId: 'popped', groupSize: 2, armIndex: 1 }), hidden).kind).toBe('started')
    expect(preview.summary('a')).toBeNull()
    expect(getActiveGroup('a')).toBeUndefined()
    expect(owner.receive(packet('one', 'run_completed'), hidden).kind).toBe('pending')
    const final = owner.receive(packet('two', 'run_completed'), hidden)
    if (final.kind !== 'ready') throw new Error('Expected the last arm to settle')
    expect(await owner.settleExternalTerminal(final.permit, async () => null, () => {})).toBe(true)
    expect(execution.snapshot('a').inFlight).toBe(false)
    expect(preview.summary('a')).toBeNull()
    preview.dispose()
  })

  it('keeps a popped-out group hidden and unsettled when it docks before its first terminal', async () => {
    const execution = createChatExecutionOwner()
    const preview = createStreamPreviewOwner()
    const lifecycle = createChatStreamLifecycleOwner(execution, preview)
    const popout = createChatPopoutOwnershipOwner({
      listConversationPopouts: vi.fn(), openConversationPopout: vi.fn(), closeConversationPopout: vi.fn(),
    })
    popout.changed(['a'])
    const receive = (event: ChatStreamPayload) => {
      const ownership = popout.observeRun(event)
      return lifecycle.receive(event, { project: !ownership.suppressMainProjection })
    }
    receive(packet('one', 'run_started', { groupId: 'popped', groupSize: 2, armIndex: 0 }))
    receive(packet('two', 'run_started', { groupId: 'popped', groupSize: 2, armIndex: 1 }))
    popout.changed([])
    expect(receive(packet('one', 'run_completed')).kind).toBe('pending')
    expect(execution.snapshot('a').inFlight).toBe(true)
    const last = receive(packet('two', 'run_completed'))
    if (last.kind !== 'ready') throw new Error('Expected last arm to settle')
    expect(await lifecycle.settleExternalTerminal(last.permit, async () => null, () => {})).toBe(true)
    expect(execution.snapshot('a').inFlight).toBe(false)
    expect(getActiveGroup('a')).toBeUndefined()
    preview.dispose()
  })

  it('rejects an old terminal and old asynchronous reload after a new recovered run starts', async () => {
    const execution = createChatExecutionOwner()
    const preview = createStreamPreviewOwner()
    const owner = createChatStreamLifecycleOwner(execution, preview)
    owner.receive(packet('old', 'run_started', { groupId: 'old-group', groupSize: 1, armIndex: 0 }))
    const old = owner.receive(packet('old', 'run_completed'))
    if (old.kind !== 'ready') throw new Error('Expected old terminal to be ready')
    let resolveReload!: (value: string) => void
    const commit = vi.fn()
    const settling = owner.settleExternalTerminal(
      old.permit,
      () => new Promise<string>((resolve) => { resolveReload = resolve }),
      commit,
    )
    owner.receive(packet('new', 'run_started', { groupId: 'new-group', groupSize: 1, armIndex: 0 }))
    expect(owner.receive(packet('old', 'run_completed'))).toMatchObject({ kind: 'ignored' })
    resolveReload('old conversation')
    expect(await settling).toBe(false)
    expect(commit).not.toHaveBeenCalled()
    expect(execution.snapshot('a').inFlight).toBe(true)
    preview.dispose()
  })

  it('retires a finished run without presenting its late read after A-B-A navigation', async () => {
    const execution = createChatExecutionOwner()
    const preview = createStreamPreviewOwner()
    const owner = createChatStreamLifecycleOwner(execution, preview)
    beginConversationTransition('a')
    const navigationLease = captureConversationNavigation()
    owner.receive(packet('old', 'run_started'))
    const terminal = owner.receive(packet('old', 'run_completed'))
    if (terminal.kind !== 'ready') throw new Error('Expected terminal to settle')
    let resolveRead!: (value: string) => void
    const present = vi.fn()
    const settling = owner.settleExternalTerminal(
      terminal.permit,
      () => new Promise<string>((resolve) => { resolveRead = resolve }),
      (outcome) => { if (isCurrentConversationNavigation(navigationLease)) present(outcome) },
    )
    beginConversationTransition('b')
    beginConversationTransition('a')
    resolveRead('obsolete A')
    expect(await settling).toBe(true)
    expect(present).not.toHaveBeenCalled()
    expect(execution.snapshot('a').inFlight).toBe(false)
    preview.dispose()
  })

  it('settles a terminal-only recovered run without creating a blank live preview', async () => {
    const execution = createChatExecutionOwner()
    const preview = createStreamPreviewOwner()
    const owner = createChatStreamLifecycleOwner(execution, preview)
    const result = owner.receive(packet('orphan', 'run_completed'))
    if (result.kind !== 'ready') throw new Error('Expected terminal-only run to settle')
    expect(preview.summary('a')).toBeNull()
    const commit = vi.fn()
    expect(await owner.settleExternalTerminal(result.permit, async () => 'loaded', commit)).toBe(true)
    expect(commit).toHaveBeenCalledWith({ kind: 'loaded', value: 'loaded' })
    expect(execution.snapshot('a').inFlight).toBe(false)
    preview.dispose()
  })

  it('clears the old run before a terminal commit callback starts a new run', async () => {
    const execution = createChatExecutionOwner()
    const preview = createStreamPreviewOwner()
    const owner = createChatStreamLifecycleOwner(execution, preview)
    owner.receive(packet('old', 'run_started'))
    const result = owner.receive(packet('old', 'run_completed'))
    if (result.kind !== 'ready') throw new Error('Expected old terminal to settle')
    const committed = await owner.settleExternalTerminal(result.permit, async () => 'loaded', () => {
      expect(execution.begin({ conversationId: 'a', kind: 'send', startedAt: 2 })).not.toBeNull()
    })
    expect(committed).toBe(true)
    expect(execution.snapshot('a').inFlight).toBe(true)
    preview.dispose()
  })

  it('settles a failed authoritative read with an explicit retryable error outcome', async () => {
    const execution = createChatExecutionOwner()
    const preview = createStreamPreviewOwner()
    const owner = createChatStreamLifecycleOwner(execution, preview)
    owner.receive(packet('old', 'run_started'))
    const result = owner.receive(packet('old', 'run_completed'))
    if (result.kind !== 'ready') throw new Error('Expected terminal to settle')
    const commit = vi.fn()
    const settled = await owner.settleExternalTerminal(
      result.permit,
      async (): Promise<string> => { throw new Error('read failed') },
      commit,
    )
    expect(settled).toBe(true)
    expect(commit).toHaveBeenCalledWith({ kind: 'failed', error: new Error('read failed') })
    expect(execution.snapshot('a').inFlight).toBe(false)
    expect(execution.begin({ conversationId: 'a', kind: 'send', startedAt: 2 })).not.toBeNull()
    preview.dispose()
  })

  it('releases a recovered group after a failed read once all arms are terminal', async () => {
    const execution = createChatExecutionOwner()
    const preview = createStreamPreviewOwner()
    const owner = createChatStreamLifecycleOwner(execution, preview)
    owner.receive(packet('one', 'run_started', { groupId: 'group-a', groupSize: 2, armIndex: 0 }))
    owner.receive(packet('two', 'run_started', { groupId: 'group-a', groupSize: 2, armIndex: 1 }))
    owner.receive(packet('one', 'run_failed'))
    const result = owner.receive(packet('two', 'run_completed'))
    if (result.kind !== 'ready') throw new Error('Expected recovered group to settle')
    const commit = vi.fn()
    expect(await owner.settleExternalTerminal(
      result.permit,
      async (): Promise<string> => { throw new Error('read failed') },
      commit,
    )).toBe(true)
    expect(commit).toHaveBeenCalledWith({ kind: 'failed', error: new Error('read failed') })
    expect(execution.snapshot('a').inFlight).toBe(false)
    preview.dispose()
  })
})
