import { describe, expect, it, vi } from 'vitest'
import { createChatExecutionOwner } from './chatExecutionOwner'
import { createStreamPreviewOwner } from './streamPreviewOwner'
import { createChatSendController, type SendPresentationEvent } from './chatSendController'
import type { Conversation } from './types'

const conversation = (id: string, extras: Partial<Conversation> = {}): Conversation => ({
  id, revision: 1, title: id, provider_id: 'provider', model: 'model',
  messages: [], created_at: 1, updated_at: 1, agent_runtime: { kind: 'builtin' }, ...extras,
})

function preparation(current: Conversation | null) {
  return {
    conversation: current, override: false, forceNew: false,
    providerId: 'provider', model: 'model', projectName: null, projectId: null, setId: null,
    draft: {
      agentRuntime: { kind: 'builtin' as const }, knowledgeBaseIds: [], forceKnowledgeSearch: false,
      additionalDirectories: [], thinkingLevel: null, webSearchMode: null,
      rememberedWebSearchMode: undefined, replyModels: [],
    },
    providerOAuthTypes: {},
  }
}

function harness(
  persistence: Parameters<typeof createChatSendController>[0]['persistence'],
  currentConversationId: () => string | null = () => 'a',
  onPresent?: (event: SendPresentationEvent) => void,
) {
  const executionOwner = createChatExecutionOwner(undefined, persistence)
  const previewOwner = createStreamPreviewOwner()
  const events: SendPresentationEvent[] = []
  const settlementPorts = {
    completeWithConversation: vi.fn(), completeTerminal: vi.fn().mockResolvedValue(undefined),
    abandonPreview: vi.fn(), settleQueue: vi.fn(),
  }
  const controller = createChatSendController({
    executionOwner, previewOwner, persistence, settlementPorts,
    presentation: {
      currentConversationId,
      present: (event) => { events.push(event); onPresent?.(event) },
    },
  })
  return { controller, executionOwner, previewOwner, events, settlementPorts }
}

describe('chat send controller', () => {
  it('a rejected duplicate send cannot invalidate the first pending creation commit', async () => {
    let resolveCreate!: (value: Conversation) => void
    const persistence = {
      createConversation: vi.fn(() => new Promise<Conversation>((resolve) => { resolveCreate = resolve })),
      updateConversation: vi.fn(), setAgentRuntime: vi.fn(),
      sendMessage: vi.fn().mockResolvedValue(conversation('created')),
    }
    const executionOwner = createChatExecutionOwner(undefined, persistence)
    const previewOwner = createStreamPreviewOwner()
    let generation = 0
    let shown: string | null = null
    const controller = createChatSendController({
      executionOwner, previewOwner, persistence,
      settlementPorts: {
        completeWithConversation: vi.fn(), completeTerminal: vi.fn().mockResolvedValue(undefined),
        abandonPreview: vi.fn(), settleQueue: vi.fn(),
      },
      presentation: {
        currentConversationId: () => null,
        beginCreation: () => {
          const ownedGeneration = ++generation
          return {
            isCurrent: () => generation === ownedGeneration,
            commit: (value: Conversation) => {
              if (generation !== ownedGeneration) return false
              shown = value.id
              return true
            },
          }
        },
        present: (event) => {
          if (event.kind === 'created') event.creation?.commit(event.conversation)
        },
      },
    })
    const intent = {
      content: 'hello', attachments: [], preparation: preparation(null),
      attachmentSkillId: null, disabledReason: '',
    }
    const first = controller.send(intent)
    await vi.waitFor(() => expect(persistence.createConversation).toHaveBeenCalledTimes(1))
    const duplicate = await controller.send(intent)
    expect(duplicate.kind).toBe('not_committed')

    resolveCreate(conversation('created'))
    await first
    expect(shown).toBe('created')
    previewOwner.dispose()
  })

  it('claims a creation commit when an empty conversation is replaced for a changed model', async () => {
    const original = conversation('old', { model: 'old-model' })
    const replacement = conversation('replacement')
    const persistence = {
      createConversation: vi.fn().mockResolvedValue(replacement),
      updateConversation: vi.fn(), setAgentRuntime: vi.fn(),
      sendMessage: vi.fn().mockResolvedValue(replacement),
    }
    const executionOwner = createChatExecutionOwner(undefined, persistence)
    const previewOwner = createStreamPreviewOwner()
    let shown: string | null = null
    const controller = createChatSendController({
      executionOwner, previewOwner, persistence,
      settlementPorts: {
        completeWithConversation: vi.fn(), completeTerminal: vi.fn().mockResolvedValue(undefined),
        abandonPreview: vi.fn(), settleQueue: vi.fn(),
      },
      presentation: {
        currentConversationId: () => 'old',
        beginCreation: () => ({
          isCurrent: () => true,
          commit: (value) => { shown = value.id; return true },
        }),
        present: (event) => {
          if (event.kind === 'created') event.creation?.commit(event.conversation)
        },
      },
    })

    await controller.send({
      content: 'hello', attachments: [], preparation: preparation(original),
      attachmentSkillId: null, disabledReason: '',
    })

    expect(shown).toBe('replacement')
    previewOwner.dispose()
  })

  it('selects canonical multi-answer arms after prepare and settles the queue after the persisted result', async () => {
    const original = conversation('a', {
      reply_models: [{ provider_id: 'p1', model: 'm1' }, { provider_id: 'p2', model: 'm2' }],
    })
    const persisted = conversation('a', {
      ...original, messages: [{ id: 'user-1', role: 'user', content: 'hello', timestamp: 1 }],
    })
    const sendMessage = vi.fn().mockResolvedValue(persisted)
    const persistence = { createConversation: vi.fn(), updateConversation: vi.fn(), setAgentRuntime: vi.fn(), sendMessage }
    const executionOwner = createChatExecutionOwner(undefined, persistence)
    const previewOwner = createStreamPreviewOwner()
    const events: SendPresentationEvent[] = []
    const order: string[] = []
    const controller = createChatSendController({
      executionOwner, previewOwner, persistence, now: () => 100,
      settlementPorts: {
        completeWithConversation: vi.fn(), completeTerminal: vi.fn().mockResolvedValue(undefined),
        abandonPreview: vi.fn(), settleQueue: vi.fn(() => { order.push('queue') }),
      },
      presentation: {
        currentConversationId: () => 'a',
        present: (event) => { events.push(event); if (event.kind === 'outcome') order.push('outcome') },
      },
    })
    const onAccepted = vi.fn(() => { expect(executionOwner.snapshot('a').inFlight).toBe(true) })

    const result = await controller.send({
      content: ' hello ', attachments: [], preparation: preparation(original),
      attachmentSkillId: null, disabledReason: '', onAccepted,
    })

    expect(result.kind).toBe('persisted')
    expect(events.find((event) => event.kind === 'started')).toMatchObject({ kind: 'started', fanOut: true })
    expect(onAccepted).toHaveBeenCalledTimes(1)
    expect(sendMessage).toHaveBeenCalledTimes(1)
    expect(order).toEqual(['outcome', 'queue'])
    expect(executionOwner.snapshot('a').inFlight).toBe(false)
    previewOwner.dispose()
  })

  it('keeps a force-new partial conversation for retry without clearing input before draft persistence', async () => {
    const report = vi.spyOn(console, 'error').mockImplementation(() => {})
    const created = conversation('new-a')
    const patched = conversation('new-a', { knowledge_base_ids: ['kb-a'] })
    const createConversation = vi.fn().mockResolvedValue(created)
    const updateConversation = vi.fn()
      .mockRejectedValueOnce(new Error('disk unavailable'))
      .mockResolvedValueOnce(patched)
    const sendMessage = vi.fn().mockResolvedValue(patched)
    const persistence = { createConversation, updateConversation, setAgentRuntime: vi.fn(), sendMessage }
    const executionOwner = createChatExecutionOwner(undefined, persistence)
    const previewOwner = createStreamPreviewOwner()
    const events: SendPresentationEvent[] = []
    const controller = createChatSendController({
      executionOwner, previewOwner, persistence,
      settlementPorts: {
        completeWithConversation: vi.fn(), completeTerminal: vi.fn().mockResolvedValue(undefined),
        abandonPreview: vi.fn(), settleQueue: vi.fn(),
      },
      presentation: { currentConversationId: () => null, present: (event) => { events.push(event) } },
    })
    const onAccepted = vi.fn()
    let partial: Conversation | null = null
    const firstPreparation = {
      ...preparation(null), forceNew: true,
      draft: { ...preparation(null).draft, knowledgeBaseIds: ['kb-a'] },
    }
    const first = await controller.send({
      content: 'hello', attachments: [], preparation: firstPreparation,
      attachmentSkillId: null, disabledReason: '', onAccepted,
      onPartialConversation: (value) => { partial = value },
    })
    expect(first).toMatchObject({ kind: 'not_committed', partialConversation: created })
    expect(onAccepted).not.toHaveBeenCalled()
    expect(events.some((event) => event.kind === 'started')).toBe(false)

    const retry = await controller.send({
      content: 'hello', attachments: [],
      preparation: { ...firstPreparation, conversation: partial, override: true },
      attachmentSkillId: null, disabledReason: '', onAccepted,
    })
    expect(retry.kind).toBe('persisted')
    expect(createConversation).toHaveBeenCalledTimes(1)
    expect(onAccepted).toHaveBeenCalledTimes(1)
    expect(sendMessage).toHaveBeenCalledTimes(1)
    report.mockRestore()
    previewOwner.dispose()
  })

  it('uses a normalized single canonical reply even when the draft requested fan-out', async () => {
    const canonical = conversation('a')
    const persistence = {
      createConversation: vi.fn(), setAgentRuntime: vi.fn(),
      updateConversation: vi.fn().mockResolvedValue(canonical),
      sendMessage: vi.fn().mockResolvedValue(canonical),
    }
    const { controller, events, previewOwner } = harness(persistence)
    const requested = {
      ...preparation(canonical),
      draft: {
        ...preparation(canonical).draft,
        replyModels: [{ provider_id: 'p1', model: 'm1' }, { provider_id: 'p2', model: 'm2' }],
      },
    }
    const result = await controller.send({
      content: 'hello', attachments: [], preparation: requested,
      attachmentSkillId: null, disabledReason: '',
    })
    expect(result.kind).toBe('persisted')
    expect(persistence.updateConversation).toHaveBeenCalledTimes(1)
    expect(events.find((event) => event.kind === 'started')).toMatchObject({ kind: 'started', fanOut: false })
    previewOwner.dispose()
  })

  it('uses the same uncommitted failure contract for canonical multi-answer sends', async () => {
    const multi = conversation('a', {
      reply_models: [{ provider_id: 'p1', model: 'm1' }, { provider_id: 'p2', model: 'm2' }],
    })
    const persistence = {
      createConversation: vi.fn(), updateConversation: vi.fn(), setAgentRuntime: vi.fn(),
      sendMessage: vi.fn().mockRejectedValue(new Error('write failed')),
    }
    const { controller, executionOwner, previewOwner, events, settlementPorts } = harness(persistence)
    const accepted = vi.fn()
    const result = await controller.send({
      content: 'hello', attachments: [], preparation: preparation(multi),
      attachmentSkillId: null, disabledReason: '', onAccepted: accepted,
    })

    expect(events.find((event) => event.kind === 'started')).toMatchObject({ kind: 'started', fanOut: true })
    expect(result).toMatchObject({ kind: 'not_committed', composerAccepted: false })
    expect(accepted).toHaveBeenCalledTimes(1)
    expect(executionOwner.snapshot('a').inFlight).toBe(false)
    expect(settlementPorts.settleQueue).toHaveBeenCalledWith('a')
    previewOwner.dispose()
  })

  it('settles an early done after a persisted single run even when outcome presentation throws', async () => {
    let resolveSend!: (value: Conversation) => void
    const persisted = conversation('a', {
      messages: [{ id: 'user-1', role: 'user', content: 'hello', timestamp: 1 }],
    })
    const persistence = {
      createConversation: vi.fn(), updateConversation: vi.fn(), setAgentRuntime: vi.fn(),
      sendMessage: vi.fn(() => new Promise<Conversation>((resolve) => { resolveSend = resolve })),
    }
    const report = vi.spyOn(console, 'error').mockImplementation(() => {})
    const { controller, executionOwner, previewOwner, settlementPorts } = harness(
      persistence, () => 'a', (event) => { if (event.kind === 'outcome') throw new Error('view failed') },
    )
    const sending = controller.send({
      content: 'hello', attachments: [], preparation: preparation(conversation('a')),
      attachmentSkillId: null, disabledReason: '',
    })
    await vi.waitFor(() => expect(persistence.sendMessage).toHaveBeenCalledTimes(1))
    executionOwner.observe({ kind: 'runEvent', conversationId: 'a', runId: 'run-a', started: true })
    executionOwner.observe({ kind: 'deferTerminal', terminal: { conversationId: 'a', runId: 'run-a', reason: 'done' } })
    resolveSend(persisted)
    const result = await sending
    expect(result).toMatchObject({ kind: 'persisted', conversation: persisted })
    expect(settlementPorts.completeWithConversation).toHaveBeenCalledTimes(1)
    expect(settlementPorts.completeTerminal).not.toHaveBeenCalled()
    expect(settlementPorts.settleQueue).toHaveBeenCalledWith('a', persisted)
    expect(executionOwner.snapshot('a').inFlight).toBe(false)
    report.mockRestore()
    previewOwner.dispose()
  })

  it('tells the composer to keep uncommitted input but clear input persisted before assistant failure', async () => {
    const report = vi.spyOn(console, 'error').mockImplementation(() => {})
    const kept = conversation('a', {
      messages: [{ id: 'user-1', role: 'user', content: 'hello', timestamp: 1 }],
    })
    const onAccepted = vi.fn()
    const afterCommitFailure = Object.assign(new Error('model failed'), { conversation: kept })
    const firstPersistence = {
      createConversation: vi.fn(), updateConversation: vi.fn(), setAgentRuntime: vi.fn(),
      sendMessage: vi.fn().mockRejectedValue(afterCommitFailure),
    }
    const first = harness(firstPersistence)
    const persistedError = await first.controller.send({
      content: 'hello', attachments: [], preparation: preparation(conversation('a')),
      attachmentSkillId: null, disabledReason: '', onAccepted,
    })
    expect(persistedError).toMatchObject({
      kind: 'persisted_error', conversation: kept, composerAccepted: true,
    })
    expect(first.settlementPorts.settleQueue).toHaveBeenCalledWith('a')
    first.previewOwner.dispose()

    const secondPersistence = {
      createConversation: vi.fn(), updateConversation: vi.fn(), setAgentRuntime: vi.fn(),
      sendMessage: vi.fn().mockRejectedValue(new Error('write failed')),
    }
    const second = harness(secondPersistence)
    const uncommitted = await second.controller.send({
      content: 'hello', attachments: [], preparation: preparation(conversation('a')),
      attachmentSkillId: null, disabledReason: '', onAccepted,
    })
    expect(uncommitted).toMatchObject({ kind: 'not_committed', composerAccepted: false })
    expect(onAccepted).toHaveBeenCalledTimes(2)
    expect(second.settlementPorts.settleQueue).toHaveBeenCalledWith('a')
    second.previewOwner.dispose()
    report.mockRestore()
  })

  it('rejects a duplicate send without disturbing the first background run after navigation', async () => {
    let resolveSend!: (value: Conversation) => void
    const persistence = {
      createConversation: vi.fn(), updateConversation: vi.fn(), setAgentRuntime: vi.fn(),
      sendMessage: vi.fn(() => new Promise<Conversation>((resolve) => { resolveSend = resolve })),
    }
    let visibleId = 'a'
    const { controller, executionOwner, previewOwner, settlementPorts } = harness(persistence, () => visibleId)
    const onAccepted = vi.fn()
    const intent = {
      content: 'hello', attachments: [], preparation: preparation(conversation('a')),
      attachmentSkillId: null, disabledReason: '', onAccepted,
    }
    const first = controller.send(intent)
    await vi.waitFor(() => expect(persistence.sendMessage).toHaveBeenCalledTimes(1))
    const duplicate = await controller.send(intent)
    expect(duplicate).toMatchObject({ kind: 'not_committed', composerAccepted: false })
    expect(onAccepted).toHaveBeenCalledTimes(1)
    visibleId = 'b'
    resolveSend(conversation('a'))
    expect(await first).toMatchObject({ kind: 'persisted', composerAccepted: true })
    expect(executionOwner.snapshot('a').inFlight).toBe(false)
    expect(settlementPorts.settleQueue).toHaveBeenCalledWith('a', conversation('a'))
    previewOwner.dispose()
  })
})
