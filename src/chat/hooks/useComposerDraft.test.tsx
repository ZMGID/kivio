import { act, renderHook } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import type { AgentRuntimeConfig } from '../types'
import { useComposerDraft } from './useComposerDraft'

const BUILTIN: AgentRuntimeConfig = { kind: 'builtin' }
const EXTERNAL: AgentRuntimeConfig = {
  kind: 'external',
  externalAgentId: 'codex',
  externalSandbox: 'workspace-write',
}

describe('useComposerDraft', () => {
  it('owns the complete new-conversation draft as one state transition', () => {
    const { result } = renderHook(() => useComposerDraft({
      providerId: 'provider-a',
      model: 'model-a',
      agentRuntime: BUILTIN,
      thinkingLevel: 'high',
    }))

    act(() => {
      result.current.setKnowledgeBaseIds(['kb-1'])
      result.current.setForceKnowledgeSearch(true)
      result.current.setAdditionalDirectories([{ path: 'C:/work' }])
      result.current.setWebSearchMode('third_party')
      result.current.setReplyModels([{ provider_id: 'provider-b', model: 'model-b' }])
    })

    expect(result.current.value).toMatchObject({
      providerId: 'provider-a',
      model: 'model-a',
      knowledgeBaseIds: ['kb-1'],
      forceKnowledgeSearch: true,
      additionalDirectories: [{ path: 'C:/work' }],
      thinkingLevel: 'high',
      webSearchMode: 'third_party',
      replyModels: [{ provider_id: 'provider-b', model: 'model-b' }],
      agentRuntime: BUILTIN,
    })
  })

  it('resets conversation-scoped context while preserving remembered preferences', () => {
    const { result } = renderHook(() => useComposerDraft({
      providerId: 'provider-a',
      model: 'model-a',
      agentRuntime: BUILTIN,
      thinkingLevel: 'medium',
    }))

    act(() => {
      result.current.setKnowledgeBaseIds(['kb-1'])
      result.current.setForceKnowledgeSearch(true)
      result.current.setAdditionalDirectories([{ path: 'C:/work' }])
      result.current.setWebSearchMode('builtin')
      result.current.setReplyModels([{ provider_id: 'provider-b', model: 'model-b' }])
      result.current.resetConversationContext({
        providerId: 'provider-c',
        model: 'model-c',
        agentRuntime: EXTERNAL,
      })
    })

    expect(result.current.value).toEqual({
      providerId: 'provider-c',
      model: 'model-c',
      knowledgeBaseIds: [],
      forceKnowledgeSearch: false,
      additionalDirectories: [],
      thinkingLevel: 'medium',
      webSearchMode: 'builtin',
      replyModels: [{ provider_id: 'provider-b', model: 'model-b' }],
      agentRuntime: EXTERNAL,
    })
  })

  it('updates provider and model atomically', () => {
    const { result } = renderHook(() => useComposerDraft({
      providerId: '',
      model: '',
      agentRuntime: BUILTIN,
      thinkingLevel: null,
    }))

    act(() => result.current.setProviderModel('provider-a', 'model-a'))

    expect(result.current.value.providerId).toBe('provider-a')
    expect(result.current.value.model).toBe('model-a')
  })
})
