import { resolveProviderWebSearchMode } from '../api/tauri'
import { agentRuntimesEqual, chatApi, normalizeAgentRuntime } from './api'
import type {
  AdditionalDirectory,
  AgentRuntimeConfig,
  Conversation,
  ModelRef,
  ThinkingLevel,
  WebSearchMode,
} from './types'

type Stage = 'create' | 'reservation' | 'runtime' | 'knowledgeBase' | 'forceKnowledgeSearch'
  | 'additionalDirectories' | 'thinkingLevel' | 'webSearchMode' | 'replyModels'

export interface SendPreparationIntent {
  conversation: Conversation | null
  override: boolean
  forceNew: boolean
  providerId: string
  model: string
  projectName: string | null
  projectId: string | null
  setId: string | null
  draft: {
    agentRuntime: AgentRuntimeConfig
    knowledgeBaseIds: string[]
    forceKnowledgeSearch: boolean
    additionalDirectories: AdditionalDirectory[]
    thinkingLevel: ThinkingLevel | null
    webSearchMode: WebSearchMode | null | undefined
    rememberedWebSearchMode: WebSearchMode | undefined
    replyModels: ModelRef[]
  }
  providerOAuthTypes: Record<string, string>
}

export type SendPreparationResult =
  | { ok: true; conversation: Conversation; created: boolean }
  | { ok: false; stage: Stage; error: Error; conversation: Conversation | null; created: boolean }

type Persistence = Pick<typeof chatApi, 'createConversation' | 'setAgentRuntime' | 'updateConversation'>
type OnProgress = (phase: 'created' | 'updated', conversation: Conversation) => boolean | void

function asError(value: unknown): Error {
  return value instanceof Error ? value : new Error(typeof value === 'string' ? value : '会话准备失败')
}

function sameDirectories(left: AdditionalDirectory[], right: AdditionalDirectory[]) {
  return left.length === right.length && left.every((entry, index) => (
    entry.path === right[index]?.path && (entry.name ?? '') === (right[index]?.name ?? '')
  ))
}

/** Prepare the authoritative conversation before accepting user input. A
 * failed patch returns the last persisted partial conversation so the caller
 * can keep the draft visible and retry without losing earlier writes. */
export async function prepareConversationForSend(
  intent: SendPreparationIntent,
  persistence: Persistence,
  onProgress: OnProgress,
  onCreating?: () => void,
): Promise<SendPreparationResult> {
  let conversation = intent.override ? intent.conversation : intent.forceNew ? null : intent.conversation
  let created = false
  if (
    conversation
    && !intent.override
    && conversation.messages.length === 0
    && !(conversation.assistant_id ?? conversation.assistantId)
    && (conversation.provider_id !== intent.providerId || conversation.model !== intent.model)
  ) conversation = null

  const failed = (stage: Stage, value: unknown): SendPreparationResult => ({
    ok: false, stage, error: asError(value), conversation, created,
  })

  if (!conversation) {
    try {
      onCreating?.()
      conversation = await persistence.createConversation(
        intent.providerId || undefined,
        intent.model || undefined,
        intent.projectName ?? undefined,
        intent.projectId,
        undefined,
        intent.setId,
      )
      created = true
      if (onProgress('created', conversation) === false) {
        return failed('reservation', new Error('该对话正在发送中，请稍后再试'))
      }
    } catch (error) {
      return failed('create', error)
    }
  }

  const apply = async (stage: Stage, update: () => Promise<Conversation>): Promise<SendPreparationResult | null> => {
    try {
      conversation = await update()
      onProgress('updated', conversation)
      return null
    } catch (error) {
      return failed(stage, error)
    }
  }

  const draft = intent.draft
  if (conversation.messages.length === 0 && !agentRuntimesEqual(normalizeAgentRuntime(conversation), draft.agentRuntime)) {
    const error = await apply('runtime', () => persistence.setAgentRuntime(conversation!.id, draft.agentRuntime))
    if (error) return error
  }

  const kb = conversation.knowledge_base_ids ?? conversation.knowledgeBaseIds ?? []
  if (draft.knowledgeBaseIds.length > 0 && (
    kb.length !== draft.knowledgeBaseIds.length || !kb.every((id) => draft.knowledgeBaseIds.includes(id))
  )) {
    const error = await apply('knowledgeBase', () => persistence.updateConversation(conversation!.id, {
      knowledgeBaseIds: draft.knowledgeBaseIds,
    }))
    if (error) return error
  }
  if (draft.forceKnowledgeSearch && !(conversation.force_knowledge_search ?? conversation.forceKnowledgeSearch ?? false)) {
    const error = await apply('forceKnowledgeSearch', () => persistence.updateConversation(conversation!.id, {
      forceKnowledgeSearch: true,
    }))
    if (error) return error
  }
  const directories = conversation.additional_directories ?? conversation.additionalDirectories ?? []
  if (draft.additionalDirectories.length > 0 && !sameDirectories(directories, draft.additionalDirectories)) {
    const error = await apply('additionalDirectories', () => persistence.updateConversation(conversation!.id, {
      additionalDirectories: draft.additionalDirectories,
    }))
    if (error) return error
  }
  if (draft.thinkingLevel && (conversation.thinking_level ?? conversation.thinkingLevel ?? null) === null) {
    const error = await apply('thinkingLevel', () => persistence.updateConversation(conversation!.id, {
      thinkingLevel: draft.thinkingLevel,
    }))
    if (error) return error
  }
  const desiredWebMode = resolveProviderWebSearchMode(
    draft.webSearchMode ?? draft.rememberedWebSearchMode,
    intent.providerOAuthTypes[conversation.provider_id],
  )
  if (desiredWebMode && (conversation.web_search_mode ?? conversation.webSearchMode ?? null) === null) {
    const error = await apply('webSearchMode', () => persistence.updateConversation(conversation!.id, {
      webSearchMode: desiredWebMode,
    }))
    if (error) return error
  }
  const replies = conversation.reply_models ?? conversation.replyModels ?? []
  if (draft.replyModels.length > 0 && (
    replies.length !== draft.replyModels.length
    || !replies.every((ref, index) => (
      ref.provider_id === draft.replyModels[index]?.provider_id
      && ref.model === draft.replyModels[index]?.model
    ))
  )) {
    const error = await apply('replyModels', () => persistence.updateConversation(conversation!.id, {
      replyModels: draft.replyModels,
    }))
    if (error) return error
  }

  return { ok: true, conversation, created }
}
