import { act, renderHook } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { chatApi } from '../api'
import { persistLastChatModelToSettings, saveLastThinkingLevel, saveLastWebSearchMode } from '../composerPreferences'
import { saveLastModel } from '../../data/chatModelPreference'
import { saveLastAgentRuntime } from '../lastAgentRuntime'
import type { AgentRuntimeConfig, Conversation } from '../types'
import { useConversationMetaMutations } from './useConversationMetaMutations'

vi.mock('../api', () => ({
  chatApi: {
    updateConversation: vi.fn(),
    setAgentRuntime: vi.fn(),
    pauseGoal: vi.fn(),
    resumeGoal: vi.fn(),
    cancelGoal: vi.fn(),
    editGoal: vi.fn(),
    continueGoal: vi.fn(),
  },
}))
vi.mock('../composerPreferences', () => ({
  persistLastChatModelToSettings: vi.fn(async () => {}),
  saveLastThinkingLevel: vi.fn(),
  saveLastWebSearchMode: vi.fn(),
}))
vi.mock('../../data/chatModelPreference', () => ({ saveLastModel: vi.fn() }))
vi.mock('../lastAgentRuntime', () => ({ saveLastAgentRuntime: vi.fn() }))

const mockUpdate = vi.mocked(chatApi.updateConversation)
const mockSetRuntime = vi.mocked(chatApi.setAgentRuntime)
const mockPause = vi.mocked(chatApi.pauseGoal)
const mockResume = vi.mocked(chatApi.resumeGoal)
const mockContinue = vi.mocked(chatApi.continueGoal)

const BUILTIN: AgentRuntimeConfig = { kind: 'builtin' }
const EXTERNAL: AgentRuntimeConfig = { kind: 'external', externalAgentId: 'claude', externalModel: 'opus' }

const conversation = (overrides: Partial<Conversation> = {}): Conversation => ({
  id: 'c1', revision: 1, title: 't', provider_id: 'p', model: 'm', messages: [], created_at: 1, updated_at: 1,
  ...overrides,
} as Conversation)

function setup(current: Conversation | null = conversation(), activeAgentRuntime = BUILTIN) {
  const currentConversationRef = { current }
  const draft = {
    setProviderModel: vi.fn(),
    setThinkingLevel: vi.fn(),
    setWebSearchMode: vi.fn(),
    setReplyModels: vi.fn(),
    setKnowledgeBaseIds: vi.fn(),
    setForceKnowledgeSearch: vi.fn(),
    setAdditionalDirectories: vi.fn(),
    setAgentRuntime: vi.fn(),
  }
  const applyConversationMeta = vi.fn()
  const applyConversationIfCurrent = vi.fn((expectedId: string) => currentConversationRef.current?.id === expectedId)
  const setStreamErrorForConversation = vi.fn()
  const rendered = renderHook(() => useConversationMetaMutations({
    currentConversationRef,
    activeAgentRuntime,
    draft,
    draftForceKnowledgeSearch: false,
    applyConversationMeta,
    applyConversationIfCurrent,
    setStreamErrorForConversation,
  }))
  return { ...rendered, currentConversationRef, draft, applyConversationMeta, applyConversationIfCurrent, setStreamErrorForConversation }
}

beforeEach(() => {
  vi.clearAllMocks()
  vi.spyOn(console, 'error').mockImplementation(() => {})
})

describe('useConversationMetaMutations: metadata fields', () => {
  it('updates draft + global preference + conversation when changing model', async () => {
    const updated = conversation({ revision: 2, model: 'gpt' })
    mockUpdate.mockResolvedValue(updated)
    const { result, draft, applyConversationMeta } = setup()
    await act(async () => { await result.current.changeModel('p2', 'gpt') })
    expect(draft.setProviderModel).toHaveBeenCalledWith('p2', 'gpt')
    expect(saveLastModel).toHaveBeenCalledWith('p2', 'gpt')
    expect(persistLastChatModelToSettings).toHaveBeenCalledWith('p2', 'gpt')
    expect(mockUpdate).toHaveBeenCalledWith('c1', { providerId: 'p2', model: 'gpt' })
    expect(applyConversationMeta).toHaveBeenCalledWith(updated)
  })

  it('only touches the draft when there is no conversation yet', async () => {
    const { result, draft } = setup(null)
    await act(async () => {
      await result.current.changeThinkingLevel('low')
      await result.current.setWebSearchMode('builtin')
      await result.current.changeReplyModels([{ provider_id: 'p', model: 'a' }])
    })
    expect(draft.setThinkingLevel).toHaveBeenCalledWith('low')
    expect(saveLastThinkingLevel).toHaveBeenCalledWith('low')
    expect(draft.setWebSearchMode).toHaveBeenCalledWith('builtin')
    expect(saveLastWebSearchMode).toHaveBeenCalledWith('builtin')
    expect(draft.setReplyModels).toHaveBeenCalledTimes(1)
    expect(mockUpdate).not.toHaveBeenCalled()
  })

  it('surfaces failures for user-visible fields but stays silent for knowledge-base mounts', async () => {
    mockUpdate.mockRejectedValue(new Error('nope'))
    const { result, setStreamErrorForConversation } = setup()
    await act(async () => { await result.current.changeAdditionalDirectories([]) })
    expect(setStreamErrorForConversation).toHaveBeenCalledWith('c1', 'nope')
    setStreamErrorForConversation.mockClear()
    await act(async () => { await result.current.changeKnowledgeBaseIds(['kb']) })
    expect(setStreamErrorForConversation).not.toHaveBeenCalled()
  })

  it('toggles force knowledge search off the conversation value, falling back to the draft', async () => {
    mockUpdate.mockResolvedValue(conversation())
    const { result, draft } = setup(conversation({ force_knowledge_search: true } as Partial<Conversation>))
    await act(async () => { await result.current.toggleForceKnowledgeSearch() })
    expect(draft.setForceKnowledgeSearch).toHaveBeenCalledWith(false)
    expect(mockUpdate).toHaveBeenCalledWith('c1', { forceKnowledgeSearch: false })

    const blank = setup(null)
    await act(async () => { await blank.result.current.toggleForceKnowledgeSearch() })
    expect(blank.draft.setForceKnowledgeSearch).toHaveBeenCalledWith(true)
  })

  it('reads the conversation at call time, not at render time', async () => {
    mockUpdate.mockResolvedValue(conversation({ id: 'c2' }))
    const { result, currentConversationRef } = setup()
    currentConversationRef.current = conversation({ id: 'c2' })
    await act(async () => { await result.current.changeModel('p', 'm') })
    expect(mockUpdate).toHaveBeenCalledWith('c2', expect.anything())
  })
})

describe('useConversationMetaMutations: runtime', () => {
  it('pauses an active goal before switching runtime and remembers the pick', async () => {
    const paused = conversation({ goal_state: { status: 'paused' } } as Partial<Conversation>)
    mockPause.mockResolvedValue(paused)
    mockSetRuntime.mockResolvedValue(conversation({ revision: 3 }))
    const { result, draft, applyConversationIfCurrent } = setup(
      conversation({ goal_state: { status: 'active' } } as Partial<Conversation>),
    )
    await act(async () => { await result.current.changeRuntime(EXTERNAL) })
    expect(draft.setAgentRuntime).toHaveBeenCalledWith(EXTERNAL)
    expect(saveLastAgentRuntime).toHaveBeenCalledWith(EXTERNAL)
    expect(mockPause).toHaveBeenCalledWith('c1')
    expect(mockSetRuntime).toHaveBeenCalledWith('c1', EXTERNAL)
    expect(applyConversationIfCurrent).toHaveBeenCalledTimes(2)
  })

  it('does not pause a goal that is already settled', async () => {
    mockSetRuntime.mockResolvedValue(conversation())
    const { result } = setup(conversation({ goal_state: { status: 'completed' } } as Partial<Conversation>))
    await act(async () => { await result.current.changeRuntime(BUILTIN) })
    expect(mockPause).not.toHaveBeenCalled()
  })

  it('derives external model / sandbox / preset changes from the active runtime', async () => {
    const { result } = setup(null, EXTERNAL)
    await act(async () => {
      await result.current.changeExternalModel('sonnet', 'high')
      await result.current.changeExternalSandbox('yolo')
      await result.current.changeExternalPreset('fast')
    })
    const calls = vi.mocked(saveLastAgentRuntime).mock.calls.map(([runtime]) => runtime)
    expect(calls[0]).toMatchObject({ kind: 'external', externalAgentId: 'claude', externalModel: 'sonnet', externalReasoning: 'high' })
    expect(calls[1]).toMatchObject({ externalSandbox: 'yolo', externalModel: 'opus' })
    expect(calls[2]).toMatchObject({ externalAgentPreset: 'fast' })
    expect(mockSetRuntime).not.toHaveBeenCalled()
  })

  it('persists an approved sandbox and syncs the draft only while still current', async () => {
    mockSetRuntime.mockResolvedValue(conversation())
    const { result, draft } = setup()
    await act(async () => { await result.current.persistApprovedExternalSandbox('c1', EXTERNAL, 'workspace') })
    expect(mockSetRuntime).toHaveBeenCalledWith('c1', { ...EXTERNAL, externalSandbox: 'workspace' })
    expect(draft.setAgentRuntime).toHaveBeenCalledTimes(1)

    draft.setAgentRuntime.mockClear()
    await act(async () => { await result.current.persistApprovedExternalSandbox('other', EXTERNAL, 'workspace') })
    expect(draft.setAgentRuntime).not.toHaveBeenCalled()
  })
})

describe('useConversationMetaMutations: goal', () => {
  it('continues the goal after resume when it is active again', async () => {
    mockResume.mockResolvedValue(conversation({ goal_state: { status: 'active' } } as Partial<Conversation>))
    mockContinue.mockResolvedValue(conversation({ revision: 5 }))
    const { result, applyConversationIfCurrent } = setup()
    await act(async () => { await result.current.resumeGoal() })
    expect(mockContinue).toHaveBeenCalledWith('c1')
    expect(applyConversationIfCurrent).toHaveBeenCalledTimes(2)
  })

  it('does not continue after pause / cancel', async () => {
    mockPause.mockResolvedValue(conversation({ goal_state: { status: 'paused' } } as Partial<Conversation>))
    const { result } = setup()
    await act(async () => { await result.current.pauseGoal() })
    expect(mockContinue).not.toHaveBeenCalled()
  })

  it('reports and rethrows goal failures so the caller can keep its own UI state', async () => {
    mockPause.mockRejectedValue(new Error('goal down'))
    const { result, setStreamErrorForConversation } = setup()
    await expect(act(async () => { await result.current.pauseGoal() })).rejects.toThrow('goal down')
    expect(setStreamErrorForConversation).toHaveBeenCalledWith('c1', 'goal down')
  })
})
