import { useCallback, useState } from 'react'
import type {
  AdditionalDirectory,
  AgentRuntimeConfig,
  ModelRef,
  ThinkingLevel,
  WebSearchMode,
} from '../types'

export interface ComposerDraft {
  providerId: string
  model: string
  knowledgeBaseIds: string[]
  forceKnowledgeSearch: boolean
  additionalDirectories: AdditionalDirectory[]
  thinkingLevel: ThinkingLevel | null
  webSearchMode: WebSearchMode | undefined
  replyModels: ModelRef[]
  agentRuntime: AgentRuntimeConfig
}

type ComposerDraftInitial = Pick<
  ComposerDraft,
  'providerId' | 'model' | 'thinkingLevel' | 'agentRuntime'
>

type ConversationDraftIdentity = Pick<
  ComposerDraft,
  'providerId' | 'model' | 'agentRuntime'
>

export function useComposerDraft(initial: ComposerDraftInitial) {
  const [value, setValue] = useState<ComposerDraft>(() => ({
    ...initial,
    knowledgeBaseIds: [],
    forceKnowledgeSearch: false,
    additionalDirectories: [],
    webSearchMode: undefined,
    replyModels: [],
  }))

  const update = useCallback(<K extends keyof ComposerDraft>(key: K, next: ComposerDraft[K]) => {
    setValue((current) => ({ ...current, [key]: next }))
  }, [])

  const setProviderModel = useCallback((providerId: string, model: string) => {
    setValue((current) => ({ ...current, providerId, model }))
  }, [])

  const resetConversationContext = useCallback((identity: ConversationDraftIdentity) => {
    setValue((current) => ({
      ...current,
      ...identity,
      knowledgeBaseIds: [],
      forceKnowledgeSearch: false,
      additionalDirectories: [],
    }))
  }, [])

  return {
    value,
    setProviderModel,
    resetConversationContext,
    setProviderId: useCallback((next: string) => update('providerId', next), [update]),
    setModel: useCallback((next: string) => update('model', next), [update]),
    setKnowledgeBaseIds: useCallback((next: string[]) => update('knowledgeBaseIds', next), [update]),
    setForceKnowledgeSearch: useCallback((next: boolean) => update('forceKnowledgeSearch', next), [update]),
    setAdditionalDirectories: useCallback(
      (next: AdditionalDirectory[]) => update('additionalDirectories', next),
      [update],
    ),
    setThinkingLevel: useCallback(
      (next: ThinkingLevel | null) => update('thinkingLevel', next),
      [update],
    ),
    setWebSearchMode: useCallback(
      (next: WebSearchMode | undefined) => update('webSearchMode', next),
      [update],
    ),
    setReplyModels: useCallback((next: ModelRef[]) => update('replyModels', next), [update]),
    setAgentRuntime: useCallback(
      (next: AgentRuntimeConfig) => update('agentRuntime', next),
      [update],
    ),
  }
}
