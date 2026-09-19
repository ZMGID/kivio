import { describe, expect, it, vi } from 'vitest'
import type {
  ChatSessionConsentPayload,
  ChatToolConfirmPayload,
  ChatUserPromptPayload,
} from '../api/tauri'
import { createChatExecutionOwner } from './chatExecutionOwner'
import { createRunInteractionInbox } from './runInteractionInbox'

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail })
  return { promise, resolve, reject }
}

function tool(conversationId: string, runId: string, toolCallId: string): ChatToolConfirmPayload {
  return { conversationId, runId, toolCallId, name: 'write_file', source: 'test' }
}

function consent(conversationId: string, runId: string): ChatSessionConsentPayload {
  return { conversationId, runId }
}

function user(conversationId: string, runId: string, toolCallId: string): ChatUserPromptPayload {
  return { conversationId, runId, toolCallId, name: 'ask_user', source: 'test', prompt: { questions: [] } }
}

function setup() {
  const confirmTool = vi.fn(() => Promise.resolve())
  const respondConsent = vi.fn(() => Promise.resolve())
  const inbox = createRunInteractionInbox({ confirmTool, respondConsent })
  return { inbox, confirmTool, respondConsent }
}

describe('run interaction inbox', () => {
  it('queues concurrent tool approvals FIFO, deduplicates replay, and restores the active conversation', () => {
    const { inbox } = setup()
    inbox.activate('a')
    expect(inbox.observe({ kind: 'toolRequested', payload: tool('a', 'run-a', 'first') })).toBe(true)
    inbox.observe({ kind: 'toolRequested', payload: tool('a', 'run-a', 'second') })
    expect(inbox.observe({ kind: 'toolRequested', payload: tool('a', 'run-a', 'first') })).toBe(false)
    inbox.observe({ kind: 'toolRequested', payload: tool('b', 'run-b', 'other') })

    expect(inbox.getSnapshot().toolConfirm?.toolCallId).toBe('first')
    expect(inbox.getSnapshot().pendingToolConversationIds).toEqual(['a', 'b'])
    inbox.observe({ kind: 'toolWithdrawn', conversationId: 'a', toolCallId: 'first' })
    expect(inbox.getSnapshot().toolConfirm?.toolCallId).toBe('second')
    inbox.activate('b')
    expect(inbox.getSnapshot().toolConfirm?.toolCallId).toBe('other')
    inbox.activate('a')
    expect(inbox.getSnapshot().toolConfirm?.toolCallId).toBe('second')
  })

  it('does not resurrect a withdrawn approval when its request packet arrives late', () => {
    const { inbox } = setup()
    inbox.activate('a')
    inbox.observe({ kind: 'toolWithdrawn', conversationId: 'a', toolCallId: 'stale' })
    inbox.observe({ kind: 'toolRequested', payload: tool('a', 'r1', 'stale') })
    expect(inbox.getSnapshot().toolConfirm).toBeNull()
  })

  it('keeps tool, consent, and user prompts distinct per conversation', () => {
    const { inbox } = setup()
    inbox.observe({ kind: 'toolRequested', payload: tool('a', 'r1', 'tool-a') })
    inbox.observe({ kind: 'consentRequested', payload: consent('a', 'r1') })
    inbox.observe({ kind: 'userPromptRequested', payload: user('a', 'r1', 'user-a') })
    inbox.observe({ kind: 'userPromptRequested', payload: user('b', 'r2', 'user-b') })

    inbox.activate('a')
    expect(inbox.getSnapshot().toolConfirm?.toolCallId).toBe('tool-a')
    expect(inbox.getSnapshot().sessionConsent?.runId).toBe('r1')
    expect(inbox.getSnapshot().userPrompt?.toolCallId).toBe('user-a')
    inbox.activate('b')
    expect(inbox.getSnapshot().toolConfirm).toBeNull()
    expect(inbox.getSnapshot().sessionConsent).toBeNull()
    expect(inbox.getSnapshot().userPrompt?.toolCallId).toBe('user-b')
  })

  it('clears stale same-run interactions only on the first start and fences late terminal events', () => {
    const { inbox } = setup()
    inbox.activate('a')
    inbox.observe({ kind: 'toolRequested', payload: tool('a', 'old', 'old-tool') })
    inbox.observe({ kind: 'userPromptRequested', payload: user('a', 'old', 'old-user') })
    inbox.observe({ kind: 'runStarted', conversationId: 'a', runId: 'old' })
    expect(inbox.getSnapshot().toolConfirm).toBeNull()
    expect(inbox.getSnapshot().userPrompt).toBeNull()

    inbox.observe({ kind: 'toolRequested', payload: tool('a', 'old', 'fresh-tool') })
    inbox.observe({ kind: 'runStarted', conversationId: 'a', runId: 'old' })
    expect(inbox.getSnapshot().toolConfirm?.toolCallId).toBe('fresh-tool')
    inbox.observe({ kind: 'toolRequested', payload: tool('a', 'new', 'new-tool') })
    inbox.observe({ kind: 'runTerminal', conversationId: 'a', runId: 'old' })
    expect(inbox.getSnapshot().toolConfirm?.toolCallId).toBe('new-tool')
    expect(inbox.observe({ kind: 'toolRequested', payload: tool('a', 'old', 'too-late') })).toBe(false)
    expect(inbox.getSnapshot().toolConfirm?.toolCallId).toBe('new-tool')
  })

  it('submits a tool approval once, preserves the original target across route changes, and advances the queue', async () => {
    const { inbox, confirmTool } = setup()
    const pending = deferred<void>()
    confirmTool.mockReturnValue(pending.promise)
    inbox.activate('a')
    inbox.observe({ kind: 'toolRequested', payload: tool('a', 'r1', 'first') })
    inbox.observe({ kind: 'toolRequested', payload: tool('a', 'r1', 'second') })

    const first = inbox.respondTool({ approved: true, always: true, permissionMode: 'allow' })
    expect(inbox.getSnapshot().toolConfirmSubmitting).toBe(true)
    expect(await inbox.respondTool({ approved: false })).toBe(false)
    inbox.activate('b')
    pending.resolve()
    expect(await first).toBe(true)
    expect(confirmTool).toHaveBeenCalledOnce()
    expect(confirmTool).toHaveBeenCalledWith('first', true, true, 'allow')
    expect(inbox.getSnapshot().toolConfirm).toBeNull()
    inbox.activate('a')
    expect(inbox.getSnapshot().toolConfirm?.toolCallId).toBe('second')
    expect(inbox.getSnapshot().toolConfirmSubmitting).toBe(false)
    inbox.observe({ kind: 'toolRequested', payload: tool('a', 'r1', 'first') })
    expect(inbox.getSnapshot().toolConfirm?.toolCallId).toBe('second')
  })

  it('shows submission failure only for a still-pending prompt, then permits retry', async () => {
    const { inbox, confirmTool } = setup()
    inbox.activate('a')
    inbox.observe({ kind: 'toolRequested', payload: tool('a', 'r1', 'first') })
    confirmTool.mockRejectedValueOnce(new Error('network'))

    expect(await inbox.respondTool({ approved: true })).toBe(false)
    expect(inbox.getSnapshot().toolConfirmError).toBe('network')
    expect(inbox.getSnapshot().toolConfirm?.toolCallId).toBe('first')
    expect(await inbox.respondTool({ approved: true })).toBe(true)
    expect(inbox.getSnapshot().toolConfirm).toBeNull()
    expect(inbox.getSnapshot().toolConfirmError).toBe('')
  })

  it('does not attach a late tool failure to a replacement prompt', async () => {
    const { inbox, confirmTool } = setup()
    const pending = deferred<void>()
    confirmTool.mockReturnValue(pending.promise)
    inbox.activate('a')
    inbox.observe({ kind: 'toolRequested', payload: tool('a', 'r1', 'old') })
    const responding = inbox.respondTool({ approved: true })
    inbox.observe({ kind: 'toolWithdrawn', conversationId: 'a', toolCallId: 'old' })
    inbox.observe({ kind: 'toolRequested', payload: tool('a', 'r2', 'new') })
    pending.reject(new Error('old failed'))

    expect(await responding).toBe(false)
    expect(inbox.getSnapshot().toolConfirm?.toolCallId).toBe('new')
    expect(inbox.getSnapshot().toolConfirmError).toBe('')
  })

  it('does not let an old consent response erase a replacement run consent', async () => {
    const { inbox, respondConsent } = setup()
    const pending = deferred<void>()
    respondConsent.mockReturnValue(pending.promise)
    inbox.activate('a')
    inbox.observe({ kind: 'consentRequested', payload: consent('a', 'old') })
    const responding = inbox.respondConsent(true)
    inbox.observe({ kind: 'consentRequested', payload: consent('a', 'new') })
    expect(await inbox.respondConsent(false)).toBe(false)
    pending.resolve()

    expect(await responding).toBe(true)
    expect(inbox.getSnapshot().sessionConsent?.runId).toBe('new')
    expect(inbox.getSnapshot().sessionConsentSubmitting).toBe(false)
    inbox.observe({ kind: 'consentRequested', payload: consent('a', 'old') })
    expect(inbox.getSnapshot().sessionConsent?.runId).toBe('new')
  })

  it('keeps failed consent retryable and does not show its error on another conversation', async () => {
    const { inbox, respondConsent } = setup()
    inbox.activate('a')
    inbox.observe({ kind: 'consentRequested', payload: consent('a', 'r1') })
    respondConsent.mockRejectedValueOnce(new Error('permission transport failed'))
    expect(await inbox.respondConsent(true)).toBe(false)
    expect(inbox.getSnapshot().sessionConsentError).toBe('permission transport failed')

    inbox.activate('b')
    expect(inbox.getSnapshot().sessionConsentError).toBe('')
    inbox.activate('a')
    expect(inbox.getSnapshot().sessionConsentError).toBe('')
    expect(await inbox.respondConsent(true)).toBe(true)
    expect(inbox.getSnapshot().sessionConsent).toBeNull()
  })

  it('clears all interaction kinds for the terminal run without touching another run', () => {
    const { inbox } = setup()
    inbox.activate('a')
    inbox.observe({ kind: 'toolRequested', payload: tool('a', 'r1', 'tool-old') })
    inbox.observe({ kind: 'userPromptRequested', payload: user('a', 'r1', 'user-old') })
    inbox.observe({ kind: 'consentRequested', payload: consent('a', 'r1') })
    inbox.observe({ kind: 'toolRequested', payload: tool('a', 'r2', 'tool-new') })
    inbox.observe({ kind: 'userPromptRequested', payload: user('a', 'r2', 'user-new') })

    inbox.observe({ kind: 'runTerminal', conversationId: 'a', runId: 'r1' })

    expect(inbox.getSnapshot().toolConfirm?.toolCallId).toBe('tool-new')
    expect(inbox.getSnapshot().sessionConsent).toBeNull()
    expect(inbox.getSnapshot().userPrompt?.toolCallId).toBe('user-new')
  })

  it('dismisses only the answered user prompt once and preserves other queued questions', () => {
    const { inbox } = setup()
    inbox.activate('a')
    inbox.observe({ kind: 'userPromptRequested', payload: user('a', 'r1', 'first') })
    inbox.observe({ kind: 'userPromptRequested', payload: user('a', 'r1', 'first') })
    inbox.observe({ kind: 'userPromptRequested', payload: user('a', 'r1', 'second') })
    inbox.observe({ kind: 'userAnswered', conversationId: 'a', runId: 'r1', toolCallId: 'first' })
    inbox.observe({ kind: 'userAnswered', conversationId: 'a', runId: 'r1', toolCallId: 'first' })

    expect(inbox.getSnapshot().userPrompt?.toolCallId).toBe('second')
    inbox.observe({ kind: 'userAnswered', conversationId: 'a', runId: 'r1', toolCallId: 'second' })
    expect(inbox.getSnapshot().userPrompt).toBeNull()
    inbox.observe({ kind: 'userPromptRequested', payload: user('a', 'r1', 'first') })
    expect(inbox.getSnapshot().userPrompt).toBeNull()
  })

  it('drops every interaction for one conversation without disturbing another or resurrecting a late response', async () => {
    const { inbox, confirmTool } = setup()
    const pending = deferred<void>()
    confirmTool.mockReturnValue(pending.promise)
    inbox.activate('a')
    inbox.observe({ kind: 'toolRequested', payload: tool('a', 'r1', 'tool-a') })
    inbox.observe({ kind: 'consentRequested', payload: consent('a', 'r1') })
    inbox.observe({ kind: 'userPromptRequested', payload: user('a', 'r1', 'user-a') })
    inbox.observe({ kind: 'toolRequested', payload: tool('b', 'r2', 'tool-b') })
    const responding = inbox.respondTool({ approved: true })

    inbox.observe({ kind: 'drop', conversationId: 'a' })
    expect(inbox.getSnapshot().toolConfirm).toBeNull()
    expect(inbox.getSnapshot().sessionConsent).toBeNull()
    expect(inbox.getSnapshot().userPrompt).toBeNull()
    expect(inbox.getSnapshot().pendingToolConversationIds).toEqual(['b'])
    pending.resolve()
    await responding
    expect(inbox.getSnapshot().toolConfirm).toBeNull()
    inbox.activate('b')
    expect(inbox.getSnapshot().toolConfirm?.toolCallId).toBe('tool-b')
  })

  it('notifies subscribers with stable snapshots only when observable state changes', () => {
    const { inbox } = setup()
    const listener = vi.fn()
    const unsubscribe = inbox.subscribe(listener)
    const initial = inbox.getSnapshot()
    inbox.activate(null)
    expect(inbox.getSnapshot()).toBe(initial)
    inbox.activate('a')
    expect(listener).toHaveBeenCalledOnce()
    const active = inbox.getSnapshot()
    expect(active).not.toBe(initial)
    inbox.observe({ kind: 'toolRequested', payload: tool('a', 'r1', 'first') })
    expect(listener).toHaveBeenCalledTimes(2)
    unsubscribe()
    inbox.observe({ kind: 'toolRequested', payload: tool('a', 'r1', 'second') })
    expect(listener).toHaveBeenCalledTimes(2)
  })

  it('integrates with execution identity so cancellation blocks new approvals but terminal still clears the old one', () => {
    const { inbox } = setup()
    const execution = createChatExecutionOwner({ begin: vi.fn(), end: vi.fn() })
    const lease = execution.begin({ conversationId: 'a', kind: 'send', startedAt: 1 })
    expect(lease).not.toBeNull()
    expect(execution.observe({ kind: 'runEvent', conversationId: 'a', runId: 'r1', started: true })).toBe(true)
    inbox.observe({ kind: 'runStarted', conversationId: 'a', runId: 'r1' })
    inbox.activate('a')

    const first = tool('a', 'r1', 'first')
    expect(execution.allowsStreamPayload(first)).toBe(true)
    expect(execution.observe({ kind: 'runEvent', conversationId: 'a', runId: first.runId })).toBe(true)
    inbox.observe({ kind: 'toolRequested', payload: first })
    expect(inbox.getSnapshot().toolConfirm?.toolCallId).toBe('first')

    const permit = execution.requestCancellation('a', 'r1')
    expect(permit).not.toBeNull()
    const late = tool('a', 'r1', 'late')
    if (execution.allowsStreamPayload(late)
      && execution.observe({ kind: 'runEvent', conversationId: 'a', runId: late.runId })) {
      inbox.observe({ kind: 'toolRequested', payload: late })
    }
    expect(inbox.getSnapshot().toolConfirm?.toolCallId).toBe('first')

    const terminal = { conversationId: 'a', runId: 'r1', type: 'run_cancelled' }
    expect(execution.allowsStreamPayload(terminal)).toBe(true)
    expect(execution.observe({ kind: 'runEvent', conversationId: 'a', runId: terminal.runId })).toBe(true)
    inbox.observe({ kind: 'runTerminal', conversationId: 'a', runId: 'r1' })
    expect(inbox.getSnapshot().toolConfirm).toBeNull()
    expect(inbox.getSnapshot().pendingToolConversationIds).toEqual([])
  })
})
