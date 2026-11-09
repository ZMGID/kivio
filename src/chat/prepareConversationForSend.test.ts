import { describe, expect, it, vi } from 'vitest'
import { createChatSendReservations } from './chatSendReservations'
import { prepareConversationForSend } from './prepareConversationForSend'
import type { Conversation } from './types'

function conversation(id: string): Conversation {
  return {
    id, revision: 1, title: id, provider_id: 'provider', model: 'model',
    messages: [], created_at: 1, updated_at: 1,
    agent_runtime: { kind: 'builtin' },
  }
}

function intent(overrides: Record<string, unknown> = {}) {
  return {
    conversation: null,
    override: false,
    forceNew: false,
    providerId: 'provider',
    model: 'model',
    projectName: null,
    projectId: null,
    setId: null,
    draft: {
      agentRuntime: { kind: 'builtin' as const },
      knowledgeBaseIds: ['kb-a'],
      forceKnowledgeSearch: false,
      additionalDirectories: [],
      thinkingLevel: null,
      webSearchMode: null,
      rememberedWebSearchMode: undefined,
      replyModels: [],
    },
    providerOAuthTypes: {},
    ...overrides,
  }
}

describe('prepare conversation for send', () => {
  it('creates the chosen worktree for a new project chat before the first message', async () => {
    const created = conversation('new-wt')
    const withWorktree = { ...created, worktree: { path: '/repo-worktrees/kivio-wt', branch: 'kivio/wt' } }
    const createConversationWorktree = vi.fn().mockResolvedValue(withWorktree)
    const persistence = {
      createConversation: vi.fn().mockResolvedValue(created),
      setAgentRuntime: vi.fn(), updateConversation: vi.fn(), createConversationWorktree,
    }
    const result = await prepareConversationForSend(intent({
      projectId: 'p1', projectName: 'Repo',
      draft: { ...intent().draft, knowledgeBaseIds: [], worktree: true },
    }), persistence, vi.fn())
    expect(createConversationWorktree).toHaveBeenCalledWith('new-wt')
    expect(result).toEqual({ ok: true, conversation: withWorktree, created: true })
  })

  it('keeps the draft when worktree creation fails and skips it outside projects', async () => {
    const created = conversation('new-wt-fail')
    const createConversationWorktree = vi.fn().mockRejectedValue(new Error('branch exists'))
    const persistence = {
      createConversation: vi.fn().mockResolvedValue(created),
      setAgentRuntime: vi.fn(), updateConversation: vi.fn(), createConversationWorktree,
    }
    const failed = await prepareConversationForSend(intent({
      projectId: 'p1', draft: { ...intent().draft, knowledgeBaseIds: [], worktree: true },
    }), persistence, vi.fn())
    expect(failed).toMatchObject({ ok: false, stage: 'worktree', conversation: created, created: true })

    createConversationWorktree.mockClear()
    await prepareConversationForSend(intent({
      draft: { ...intent().draft, knowledgeBaseIds: [], worktree: true },
    }), persistence, vi.fn())
    expect(createConversationWorktree).not.toHaveBeenCalled()
  })

  it('retains an empty bound CLI conversation when draft defaults disagree', async () => {
    const bound: Conversation = {
      ...conversation('imported-empty'), provider_id: '', model: '', runtime_locked: true,
      agent_runtime: { kind: 'external', externalAgentId: 'claude' },
    }
    const persistence = {
      createConversation: vi.fn().mockResolvedValue(conversation('replacement')),
      setAgentRuntime: vi.fn(), updateConversation: vi.fn(),
    }
    const result = await prepareConversationForSend(intent({
      conversation: bound, draft: { ...intent().draft, knowledgeBaseIds: [] },
    }), persistence, vi.fn())
    expect(result).toEqual({ ok: true, conversation: bound, created: false })
    expect(persistence.createConversation).not.toHaveBeenCalled()
    expect(persistence.setAgentRuntime).not.toHaveBeenCalled()
  })

  it('returns a created partial conversation and recoverable error when a draft patch fails', async () => {
    const created = conversation('new-a')
    const onProgress = vi.fn()
    const updateConversation = vi.fn(async () => { throw new Error('disk unavailable') })
    const result = await prepareConversationForSend(intent(), {
      createConversation: async () => created,
      setAgentRuntime: vi.fn(),
      updateConversation,
    }, onProgress)

    expect(result).toMatchObject({ ok: false, stage: 'knowledgeBase', conversation: created, created: true })
    expect(result.ok ? null : result.error.message).toBe('disk unavailable')
    expect(onProgress).toHaveBeenCalledWith('created', created)
    expect(updateConversation).toHaveBeenCalledTimes(1)
  })

  it('retries an external force-new request against its partial conversation without creating another', async () => {
    const created = conversation('new-a')
    const patched = { ...created, revision: 2, knowledge_base_ids: ['kb-a'] }
    const createConversation = vi.fn().mockResolvedValue(created)
    const updateConversation = vi.fn()
      .mockRejectedValueOnce(new Error('disk unavailable'))
      .mockResolvedValueOnce(patched)
    const persistence = { createConversation, setAgentRuntime: vi.fn(), updateConversation }
    const first = await prepareConversationForSend(intent({ forceNew: true }), persistence, vi.fn())
    expect(first).toMatchObject({ ok: false, conversation: created })
    const retry = await prepareConversationForSend(intent({
      forceNew: true, override: true, conversation: first.conversation,
    }), persistence, vi.fn())
    expect(retry).toMatchObject({ ok: true, conversation: patched })
    expect(createConversation).toHaveBeenCalledTimes(1)
  })

  it('keeps a late draft update off a newly selected conversation while background preparation completes', async () => {
    let finishPatch!: (value: Conversation) => void
    const patched = { ...conversation('a'), knowledge_base_ids: ['kb-a'] }
    const patch = new Promise<Conversation>((resolve) => { finishPatch = resolve })
    let visible = 'a'
    const onProgress = vi.fn((_phase: string, value: Conversation) => {
      if (visible === value.id) visible = `committed:${value.id}`
    })
    const preparing = prepareConversationForSend(intent({ conversation: conversation('a') }), {
      createConversation: vi.fn(),
      setAgentRuntime: vi.fn(),
      updateConversation: () => patch,
    }, onProgress)
    visible = 'b'
    finishPatch(patched)
    const result = await preparing

    expect(result).toMatchObject({ ok: true, conversation: patched })
    expect(visible).toBe('b')
  })

  it('reserves the blank composer before creating so a repeated send cannot create twice', async () => {
    let finishCreate!: (value: Conversation) => void
    const creating = new Promise<Conversation>((resolve) => { finishCreate = resolve })
    const createConversation = vi.fn(() => creating)
    const reservations = createChatSendReservations()
    const first = reservations.claim(null)!
    const preparing = prepareConversationForSend(intent({ draft: { ...intent().draft, knowledgeBaseIds: [] } }), {
      createConversation,
      setAgentRuntime: vi.fn(),
      updateConversation: vi.fn(),
    }, vi.fn())
    expect(reservations.claim(null)).toBeNull()
    finishCreate(conversation('new-a'))
    const result = await preparing
    expect(result.ok).toBe(true)
    expect(createConversation).toHaveBeenCalledTimes(1)
    first.release()
  })
})
