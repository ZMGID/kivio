import { useCallback, type MutableRefObject } from 'react'
import { persistLastChatModelToSettings, saveLastThinkingLevel, saveLastWebSearchMode } from '../composerPreferences'
import { saveLastModel } from '../../data/chatModelPreference'
import { chatApi } from '../api'
import { withExternalModel } from '../externalModelEffort'
import { saveLastAgentRuntime } from '../lastAgentRuntime'
import type {
  AdditionalDirectory,
  AgentRuntimeConfig,
  Conversation,
  ModelRef,
  ThinkingLevel,
  WebSearchMode,
} from '../types'
import type { ComposerDraftSetters } from './useComposerDraft'

export interface UseConversationMetaMutationsOptions {
  /** 读最新会话用 ref 而不是 state：handler 身份不随会话对象换引用而变。 */
  currentConversationRef: MutableRefObject<Conversation | null>
  /** 当前生效的运行时（有会话取会话的，否则取草稿）；外部 CLI 的模型 / 沙盒 / 预设都在它上面改。 */
  activeAgentRuntime: AgentRuntimeConfig
  /** 没有会话时只改草稿；草稿在首次发送创建会话时落地。传 `useComposerDraft().setters`（稳定）。 */
  draft: Pick<
    ComposerDraftSetters,
    | 'setProviderModel' | 'setThinkingLevel' | 'setWebSearchMode' | 'setReplyModels'
    | 'setKnowledgeBaseIds' | 'setForceKnowledgeSearch' | 'setAdditionalDirectories' | 'setAgentRuntime'
  >
  draftForceKnowledgeSearch: boolean
  /** 纯元数据更新：合并后端元数据但保留 messages 引用（不击穿气泡 memo）。 */
  applyConversationMeta: (updated: Conversation) => void
  /** 会改 goal / 运行时这类可能带消息的更新：只在仍是当前会话时采纳。 */
  applyConversationIfCurrent: (expectedId: string, conversation: Conversation) => boolean
  setStreamErrorForConversation: (conversationId: string, message: string) => void
}

function errorMessage(err: unknown, fallback: string): string {
  return typeof err === 'string' ? err : (err as Error).message || fallback
}

/** 会话元数据（模型 / 思考等级 / 联网 / 多答 / 知识库 / 附加目录 / 运行时 / Goal）的写入口。 */
export function useConversationMetaMutations({
  currentConversationRef,
  activeAgentRuntime,
  draft,
  draftForceKnowledgeSearch,
  applyConversationMeta,
  applyConversationIfCurrent,
  setStreamErrorForConversation,
}: UseConversationMetaMutationsOptions) {
  /**
   * 「先改草稿，有会话再落库」的公共骨架。`onError` 为空时静默（知识库这类不值得打断的更新）。
   */
  const updateMeta = useCallback(async (
    updates: Parameters<typeof chatApi.updateConversation>[1],
    logLabel: string,
    errorFallback?: string,
  ) => {
    const conversation = currentConversationRef.current
    if (!conversation) return
    const conversationId = conversation.id
    try {
      const updated = await chatApi.updateConversation(conversationId, updates)
      applyConversationMeta(updated)
    } catch (err) {
      console.error(`Failed to ${logLabel}:`, err)
      if (errorFallback) setStreamErrorForConversation(conversationId, errorMessage(err, errorFallback))
    }
  }, [applyConversationMeta, currentConversationRef, setStreamErrorForConversation])

  const changeModel = useCallback(async (providerId: string, model: string) => {
    draft.setProviderModel(providerId, model)
    saveLastModel(providerId, model)
    void persistLastChatModelToSettings(providerId, model)
    await updateMeta({ providerId, model }, 'change model', '模型切换失败')
  }, [draft, updateMeta])

  const changeThinkingLevel = useCallback(async (level: ThinkingLevel | null) => {
    draft.setThinkingLevel(level)
    saveLastThinkingLevel(level) // 记住为全局默认，不再回落到 high
    await updateMeta({ thinkingLevel: level }, 'change thinking level', '思考等级切换失败')
  }, [draft, updateMeta])

  // 会话级三态联网搜索：持久化到会话（欢迎页先存草稿），并记住为全局默认——之后所有
  // 新会话 / 未显式设置的会话自动沿用（与思考等级同款）。
  const setWebSearchMode = useCallback(async (mode: WebSearchMode) => {
    draft.setWebSearchMode(mode)
    saveLastWebSearchMode(mode)
    await updateMeta({ webSearchMode: mode }, 'change web search mode', '联网搜索模式切换失败')
  }, [draft, updateMeta])

  // 多模型一问多答：上限 4 由 UI 侧约束。
  const changeReplyModels = useCallback(async (models: ModelRef[]) => {
    draft.setReplyModels(models)
    await updateMeta({ replyModels: models }, 'update reply models', '多答模型更新失败')
  }, [draft, updateMeta])

  const changeKnowledgeBaseIds = useCallback(async (ids: string[]) => {
    draft.setKnowledgeBaseIds(ids)
    await updateMeta({ knowledgeBaseIds: ids }, 'update knowledge bases')
  }, [draft, updateMeta])

  const changeAdditionalDirectories = useCallback(async (directories: AdditionalDirectory[]) => {
    draft.setAdditionalDirectories(directories)
    await updateMeta({ additionalDirectories: directories }, 'update additional directories', '附加目录更新失败')
  }, [draft, updateMeta])

  const toggleForceKnowledgeSearch = useCallback(async () => {
    const conversation = currentConversationRef.current
    const next = !(conversation
      ? (conversation.force_knowledge_search ?? conversation.forceKnowledgeSearch ?? false)
      : draftForceKnowledgeSearch)
    draft.setForceKnowledgeSearch(next)
    await updateMeta({ forceKnowledgeSearch: next }, 'update force knowledge search')
  }, [currentConversationRef, draft, draftForceKnowledgeSearch, updateMeta])

  const changeRuntime = useCallback(async (runtime: AgentRuntimeConfig) => {
    draft.setAgentRuntime(runtime)
    saveLastAgentRuntime(runtime)
    const conversation = currentConversationRef.current
    if (!conversation) return
    const conversationId = conversation.id
    try {
      // 切运行时前先暂停进行中的 Goal：另一套运行时接不上它的循环。
      const goal = conversation.goal_state ?? conversation.goalState
      if (goal && !['completed', 'cancelled', 'paused'].includes(goal.status)) {
        const paused = await chatApi.pauseGoal(conversationId)
        applyConversationIfCurrent(conversationId, paused)
      }
      const updated = await chatApi.setAgentRuntime(conversationId, runtime)
      applyConversationIfCurrent(conversationId, updated)
    } catch (err) {
      console.error('Failed to change agent runtime:', err)
      setStreamErrorForConversation(conversationId, errorMessage(err, 'Agent 切换失败'))
    }
  }, [applyConversationIfCurrent, currentConversationRef, draft, setStreamErrorForConversation])

  // 下面三个都经 changeRuntime：没有会话时草稿也要更新（首次发送创建会话时应用）。
  const changeExternalModel = useCallback(async (model: string, reasoning?: string | null) => {
    await changeRuntime(withExternalModel(activeAgentRuntime, model, reasoning))
  }, [activeAgentRuntime, changeRuntime])

  const changeExternalSandbox = useCallback(async (sandbox: string) => {
    await changeRuntime({ ...activeAgentRuntime, kind: 'external', externalSandbox: sandbox })
  }, [activeAgentRuntime, changeRuntime])

  const changeExternalPreset = useCallback(async (preset: string) => {
    await changeRuntime({ ...activeAgentRuntime, kind: 'external', externalAgentPreset: preset })
  }, [activeAgentRuntime, changeRuntime])

  /** 审批卡里「本次及以后」选择的沙盒档位：写会话，只有仍是当前会话时才同步草稿 / 全局默认。 */
  const persistApprovedExternalSandbox = useCallback(async (
    conversationId: string,
    runtime: AgentRuntimeConfig,
    sandbox: string,
  ) => {
    const next: AgentRuntimeConfig = { ...runtime, kind: 'external', externalSandbox: sandbox }
    try {
      const updated = await chatApi.setAgentRuntime(conversationId, next)
      if (applyConversationIfCurrent(conversationId, updated)) {
        draft.setAgentRuntime(next)
        saveLastAgentRuntime(next)
      }
    } catch (error) {
      console.error('Failed to persist the post-approval permission mode:', error)
      setStreamErrorForConversation(conversationId, errorMessage(error, '权限模式保存失败'))
    }
  }, [applyConversationIfCurrent, draft, setStreamErrorForConversation])

  const runGoalMutation = useCallback(async (
    mutation: (conversationId: string) => Promise<Conversation>,
    continueWhenActive = false,
  ) => {
    const conversationId = currentConversationRef.current?.id
    if (!conversationId) return
    try {
      const updated = await mutation(conversationId)
      applyConversationIfCurrent(conversationId, updated)
      const goal = updated.goal_state ?? updated.goalState
      if (continueWhenActive && goal && (goal.status === 'active' || goal.status === 'verifying')) {
        void chatApi.continueGoal(conversationId).then((result) => {
          applyConversationIfCurrent(conversationId, result)
        }).catch((error) => {
          setStreamErrorForConversation(conversationId, error instanceof Error ? error.message : String(error))
        })
      }
    } catch (error) {
      setStreamErrorForConversation(conversationId, errorMessage(error, 'Goal 操作失败'))
      throw error
    }
  }, [applyConversationIfCurrent, currentConversationRef, setStreamErrorForConversation])

  const editGoal = useCallback((objective: string) => runGoalMutation(
    (conversationId) => chatApi.editGoal(conversationId, objective),
    true,
  ), [runGoalMutation])
  const pauseGoal = useCallback(() => runGoalMutation(chatApi.pauseGoal), [runGoalMutation])
  const resumeGoal = useCallback(() => runGoalMutation(chatApi.resumeGoal, true), [runGoalMutation])
  const cancelGoal = useCallback(() => runGoalMutation(chatApi.cancelGoal), [runGoalMutation])

  return {
    changeModel,
    changeThinkingLevel,
    setWebSearchMode,
    changeReplyModels,
    changeKnowledgeBaseIds,
    changeAdditionalDirectories,
    toggleForceKnowledgeSearch,
    changeRuntime,
    changeExternalModel,
    changeExternalSandbox,
    changeExternalPreset,
    persistApprovedExternalSandbox,
    editGoal,
    pauseGoal,
    resumeGoal,
    cancelGoal,
  }
}
