import { useCallback, useMemo, useState } from 'react'
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

  // 一包稳定的 setter：整体传给下游 hook 时不必逐个列依赖，身份也不随 value 变。
  const setters = useMemo(() => ({
    setProviderModel,
    resetConversationContext,
    setProviderId: (next: string) => update('providerId', next),
    setModel: (next: string) => update('model', next),
    setKnowledgeBaseIds: (next: string[]) => update('knowledgeBaseIds', next),
    setForceKnowledgeSearch: (next: boolean) => update('forceKnowledgeSearch', next),
    setAdditionalDirectories: (next: AdditionalDirectory[]) => update('additionalDirectories', next),
    setThinkingLevel: (next: ThinkingLevel | null) => update('thinkingLevel', next),
    setWebSearchMode: (next: WebSearchMode | undefined) => update('webSearchMode', next),
    setReplyModels: (next: ModelRef[]) => update('replyModels', next),
    setAgentRuntime: (next: AgentRuntimeConfig) => update('agentRuntime', next),
  }), [resetConversationContext, setProviderModel, update])

  return { value, setters, ...setters }
}

export type ComposerDraftSetters = ReturnType<typeof useComposerDraft>['setters']
