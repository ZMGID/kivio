import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react'
import { api } from '../../api/tauri'
import { withExternalModel } from '../externalModelEffort'
import { syncChatProtocol } from '../../api/chatProtocol'
import { getSettingsCached, updateSettingsCached } from '../../api/settingsCache'
import {
  agentRuntimesEqual,
  chatApi,
  normalizeAgentRuntime,
  type AgentRuntimeConfig,
} from '../api'
import { useTauriEvent } from '../hooks/useTauriEvent'
import { userPromptEventToRecord } from '../streamApply'
import { createChatExecutionOwner } from '../chatExecutionOwner'
import { createChatStreamLifecycleOwner, type StreamLifecycleResult } from '../chatStreamLifecycleOwner'
import { createStreamPreviewOwner } from '../streamPreviewOwner'
import {
  reset as resetStreamStore,
  setCoarse as setStreamCoarse,
  useStreamCoarse,
} from '../streamingStore'
import type { MessageListProps } from '../MessageList'
import type { AgentPlanState, AgentTodoState, Conversation, GoalState, PendingAttachment, ThinkingLevel } from '../types'
import { insertTextIntoComposer } from '../composerInsert'
import { usePopoutComposer } from './usePopoutComposer'
import type {
  ChatSessionConsentPayload,
  ChatToolConfirmPayload,
  ChatUserPromptPayload,
  ChatHookPayload,
} from '../../api/tauri'
import type { Lang } from '../../components/i18n'

const EMPTY_MESSAGES: Conversation['messages'] = []

export function usePopoutSession(conversationId: string, lang: Lang) {
  const [conversation, setConversation] = useState<Conversation | null>(null)
  // A queued React update is not yet a visible replacement for the live preview.
  const committedConversationRef = useRef(conversation)
  useLayoutEffect(() => { committedConversationRef.current = conversation }, [conversation])
  const [loadError, setLoadError] = useState('')
  const [pendingToolConfirm, setPendingToolConfirm] = useState<ChatToolConfirmPayload | null>(null)
  const [pendingSessionConsent, setPendingSessionConsent] = useState<ChatSessionConsentPayload | null>(null)
  const [pendingUserPrompt, setPendingUserPrompt] = useState<ChatUserPromptPayload | null>(null)
  const [hookWarning, setHookWarning] = useState<ChatHookPayload | null>(null)
  const [toolConfirmError, setToolConfirmError] = useState('')
  const [sessionConsentError, setSessionConsentError] = useState('')
  const [toolConfirmSubmitting, setToolConfirmSubmitting] = useState(false)
  const [sessionConsentSubmitting, setSessionConsentSubmitting] = useState(false)
  const [approvalPolicy, setApprovalPolicy] = useState('readonly_auto_sensitive_confirm')

  const conversationIdRef = useRef(conversationId)
  conversationIdRef.current = conversationId
  const viewActiveRef = useRef(false)
  const [executionOwner] = useState(createChatExecutionOwner)
  const [previewOwner] = useState(createStreamPreviewOwner)
  const [streamLifecycleOwner] = useState(() => createChatStreamLifecycleOwner(executionOwner, previewOwner))
  useSyncExternalStore(executionOwner.subscribe, executionOwner.getRevision)
  const pendingToolConfirmsRef = useRef<ChatToolConfirmPayload[]>([])
  const pendingUserPromptsRef = useRef<ChatUserPromptPayload[]>([])
  const streamCoarse = useStreamCoarse()
  const acceptPersistedConversation = useCallback((next: Conversation) => {
    setConversation((current) => current?.id === next.id && current.revision > next.revision
      ? current
      : next)
  }, [])

  useEffect(() => {
    viewActiveRef.current = true
    previewOwner.attach()
    previewOwner.activate(conversationId)
    let cancelled = false
    setLoadError('')
    void chatApi.getConversation(conversationId).then((conv) => {
      if (cancelled) return
      setConversation(conv)
      void syncChatProtocol(conversationId).catch(() => {})
    }).catch((err) => {
      if (cancelled) return
      setLoadError(typeof err === 'string' ? err : (err as Error).message || '加载对话失败')
    })
    void getSettingsCached().then((settings) => {
      if (!cancelled && settings.chatTools?.approvalPolicy) {
        setApprovalPolicy(settings.chatTools.approvalPolicy)
      }
    }).catch(() => {})
    return () => {
      cancelled = true
      viewActiveRef.current = false
      executionOwner.observe({ kind: 'drop', conversationId })
      previewOwner.dispose()
      resetStreamStore()
    }
  }, [conversationId, executionOwner, previewOwner])

  // The preview is replaced only after React commits the authoritative twin.
  useEffect(() => {
    if (conversation) previewOwner.reconcile(conversation.id, conversation.messages)
  }, [conversation, previewOwner])

  const settleExternalRun = useCallback((ready: Extract<StreamLifecycleResult, { kind: 'ready' }>) => {
    const id = ready.terminal.conversationId
    void streamLifecycleOwner.settleExternalTerminal(
      ready.permit,
      () => chatApi.getConversation(id),
      (outcome) => {
        if (conversationIdRef.current !== id) return
        if (outcome.kind === 'failed') {
          previewOwner.complete(id, { kind: 'error' })
          setStreamCoarse({ streamError: `回复已结束，但会话回载失败：${outcome.error.message}` })
          return
        }
        acceptPersistedConversation(outcome.value)
        if (ready.terminal.reason === 'error') {
          previewOwner.complete(id, { kind: 'error' })
          setStreamCoarse({ streamError: '回复生成失败，请稍后重试。' })
        } else {
          previewOwner.complete(id, { kind: 'persisted', committedMessages: committedConversationRef.current?.messages ?? EMPTY_MESSAGES })
        }
      },
    )
  }, [acceptPersistedConversation, previewOwner, streamLifecycleOwner])

  useTauriEvent(api.onChatStream, (payload) => {
    if (payload.conversationId !== conversationIdRef.current) return
    const result = streamLifecycleOwner.receive(payload)
    if (result.kind === 'started') {
      setHookWarning(null)
      setPendingSessionConsent(null)
      pendingToolConfirmsRef.current = []
      pendingUserPromptsRef.current = []
      setPendingToolConfirm(null)
    } else if (result.kind === 'ready') {
      previewOwner.freeze(payload.conversationId)
      settleExternalRun(result)
    }
  }, [previewOwner, settleExternalRun, streamLifecycleOwner])

  useTauriEvent(api.onChatTool, (payload) => {
    if (payload.conversationId !== conversationIdRef.current
      || !executionOwner.snapshot(payload.conversationId).inFlight
      || !executionOwner.allowsStreamPayload(payload)) return
    if (!executionOwner.observe({
      kind: 'runEvent', conversationId: payload.conversationId, runId: payload.runId,
    })) return
    previewOwner.projectDisplay({ kind: 'tool', payload })
  }, [executionOwner, previewOwner])

  useTauriEvent(api.onChatSubagent, (payload) => {
    if (payload.parentConversationId !== conversationIdRef.current
      || !executionOwner.snapshot(payload.parentConversationId).inFlight
      || !executionOwner.allowsStreamPayload({
        conversationId: payload.parentConversationId, runId: payload.parentRunId,
      })) return
    if (!executionOwner.observe({
      kind: 'runEvent', conversationId: payload.parentConversationId, runId: payload.parentRunId,
    })) return
    previewOwner.projectDisplay({ kind: 'subagent', payload })
  }, [executionOwner, previewOwner])
  useTauriEvent(api.onChatToolConfirm, (payload) => {
    if (payload.conversationId !== conversationIdRef.current) return
    const queue = pendingToolConfirmsRef.current
    if (!queue.some((item) => item.toolCallId === payload.toolCallId)) queue.push(payload)
    setPendingToolConfirm(queue[0] ?? null)
    setToolConfirmError('')
  }, [])

  useTauriEvent(api.onChatToolConfirmWithdraw, (payload) => {
    if (payload.conversationId !== conversationIdRef.current) return
    const rest = pendingToolConfirmsRef.current.filter((item) => item.toolCallId !== payload.toolCallId)
    pendingToolConfirmsRef.current = rest
    setPendingToolConfirm((current) => (
      current?.toolCallId === payload.toolCallId ? rest[0] ?? null : current
    ))
  }, [])

  useTauriEvent(api.onChatSessionConsent, (payload) => {
    if (payload.conversationId !== conversationIdRef.current) return
    setPendingSessionConsent(payload)
    setSessionConsentError('')
  }, [])

  useTauriEvent(api.onChatUserPrompt, (payload) => {
    if (payload.conversationId !== conversationIdRef.current) return
    const queue = pendingUserPromptsRef.current
    if (!queue.some((item) => item.toolCallId === payload.toolCallId)) queue.push(payload)
    setPendingUserPrompt(queue[0] ?? null)
  }, [])

  useTauriEvent(api.onChatHook, (payload) => {
    if (payload.conversationId !== conversationIdRef.current) return
    setHookWarning(payload)
  }, [])

  useTauriEvent(api.onChatQueuedTextsRestored, (payload) => {
    if (payload.conversationId !== conversationIdRef.current) return
    const text = payload.texts.map((item) => item.trim()).filter(Boolean).join('\n\n')
    if (text) insertTextIntoComposer(text)
  }, [])

  useTauriEvent(api.onChatStatusNote, (payload) => {
    if (payload.conversationId !== conversationIdRef.current
      || !executionOwner.snapshot(payload.conversationId).inFlight
      || !executionOwner.allowsStreamPayload(payload)) return
    if (!executionOwner.observe({
      kind: 'runEvent', conversationId: payload.conversationId, runId: payload.runId,
    })) return
    previewOwner.projectDisplay({ kind: 'status', payload })
  }, [executionOwner, previewOwner])

  useTauriEvent(api.onChatTodo, (payload) => {
    if (payload.conversationId !== conversationIdRef.current) return
    const todoState = payload.todoState as AgentTodoState
    setConversation((current) => current
      ? { ...current, agent_todo_state: todoState, agentTodoState: todoState }
      : current)
  }, [])

  useTauriEvent(api.onChatPlan, (payload) => {
    if (payload.conversationId !== conversationIdRef.current) return
    const planState = payload.planState as AgentPlanState
    setConversation((current) => current
      ? { ...current, agent_plan_state: planState, agentPlanState: planState }
      : current)
  }, [])

  useTauriEvent(api.onChatGoal, (payload) => {
    if (payload.conversationId !== conversationIdRef.current) return
    const goal = payload.goalState as GoalState | null
    setConversation((current) => current
      ? { ...current, goal_state: goal ?? undefined, goalState: goal ?? undefined }
      : current)
  }, [])

  useTauriEvent(api.onChatTitle, ({ conversationId: id }) => {
    if (id !== conversationIdRef.current) return
    void chatApi.getConversation(id).then((updated) => {
      if (id === conversationIdRef.current && viewActiveRef.current) acceptPersistedConversation(updated)
    }).catch((error) => console.error('Failed to refresh generated title:', error))
  }, [acceptPersistedConversation])

  const settlementPorts = useMemo<Parameters<typeof executionOwner.finish>[2]>(() => ({
    completeWithConversation: (id, persisted) => {
      if (conversationIdRef.current !== id) return
      acceptPersistedConversation(persisted)
      previewOwner.complete(id, { kind: 'persisted', committedMessages: committedConversationRef.current?.messages ?? EMPTY_MESSAGES })
    },
    completeTerminal: async (terminal) => {
      const id = terminal.conversationId
      try {
        const persisted = await chatApi.getConversation(id)
        if (conversationIdRef.current !== id) return
        acceptPersistedConversation(persisted)
        if (terminal.reason === 'error') {
          previewOwner.complete(id, { kind: 'error' })
          setStreamCoarse({ streamError: '回复生成失败，请稍后重试。' })
        } else {
          previewOwner.complete(id, { kind: 'persisted', committedMessages: committedConversationRef.current?.messages ?? EMPTY_MESSAGES })
        }
      } catch (error) {
        if (conversationIdRef.current !== id) return
        previewOwner.complete(id, { kind: 'error' })
        setStreamCoarse({
          streamError: `回复已结束，但会话回载失败：${error instanceof Error ? error.message : String(error)}`,
        })
      }
    },
    abandonPreview: (id) => previewOwner.complete(id, { kind: 'error' }),
    settleQueue: () => {},
  }), [acceptPersistedConversation, previewOwner])

  const handleSend = useCallback(async (
    content: string,
    attachments: PendingAttachment[] = [],
    options?: { onAccepted?: () => void; attachmentSkillId?: string | null },
  ) => {
    const trimmed = content.trim()
    if (!trimmed && attachments.length === 0) return false
    const conv = conversation
    if (!conv || executionOwner.snapshot(conversationId).inFlight) return false
    const claim = executionOwner.claimSend(conversationId)
    if (!claim) return false
    let lease: ReturnType<typeof executionOwner.begin> = null
    try {
      if (!executionOwner.bindSend(claim, conversationId)) return false
      const replyArms = conv.reply_models ?? conv.replyModels ?? []
      const planMode = conv.agent_plan_state?.mode ?? conv.agentPlanState?.mode ?? 'act'
      const fanOut = replyArms.length >= 2 && planMode === 'act'
      const startedAt = Date.now()
      lease = executionOwner.begin({
        conversationId, kind: 'send', startedAt, claim,
        optimistic: { content: trimmed, attachments, stored: conv.messages },
        group: fanOut ? {
          groupId: `grp-local-${startedAt}`,
          arms: replyArms.map((ref) => ({ providerId: ref.provider_id, model: ref.model })),
        } : undefined,
      })
      if (!lease) return false
      setHookWarning(null)
      setStreamCoarse({ streamError: '' })
      previewOwner.begin(conversationId, startedAt, fanOut ? 'group' : 'single')
      options?.onAccepted?.()
      const outcome = await executionOwner.submitPreparedRun({
        lease, content: trimmed, attachments,
        attachmentSkillId: options && 'attachmentSkillId' in options
          ? options.attachmentSkillId ?? null
          : conv.active_skill_id ?? conv.activeSkillId ?? null,
      }, {
        ...settlementPorts,
        onOutcome: (result) => {
          if (!viewActiveRef.current || conversationIdRef.current !== conversationId) return
          if (result.kind !== 'persisted') {
            if (result.kind === 'persisted_error') acceptPersistedConversation(result.conversation)
            setStreamCoarse({ streamError: result.error.message })
          }
        },
      })
      return outcome.kind !== 'not_committed'
    } finally {
      if (lease) await executionOwner.finish(lease, null, settlementPorts)
      executionOwner.abandonSend(claim)
    }
  }, [acceptPersistedConversation, conversation, conversationId, executionOwner, previewOwner, settlementPorts])
  const handleCancel = useCallback(async () => {
    if (!executionOwner.snapshot(conversationId).inFlight) return
    const result = await streamLifecycleOwner.cancelRun(
      conversationId,
      () => chatApi.cancelStream(conversationId),
      () => setStreamCoarse({ cancelling: true }),
    )
    if (result.kind === 'failed') {
      console.error('Failed to cancel stream:', result.error)
      setStreamCoarse({ streamError: result.error.message })
    }
    if (result.kind !== 'ignored' && result.kind !== 'superseded') setStreamCoarse({ cancelling: false })
  }, [conversationId, executionOwner, streamLifecycleOwner])

  const resolveToolConfirm = useCallback(async (
    approved: boolean,
    always = false,
    permissionMode: string | null = null,
  ) => {
    const prompt = pendingToolConfirm
    if (!prompt) return
    setToolConfirmSubmitting(true)
    setToolConfirmError('')
    try {
      await api.chatConfirmToolCall(prompt.toolCallId, approved, always, permissionMode)
      const rest = pendingToolConfirmsRef.current.filter((item) => item.toolCallId !== prompt.toolCallId)
      pendingToolConfirmsRef.current = rest
      setPendingToolConfirm(rest[0] ?? null)
    } catch (error) {
      setToolConfirmError(typeof error === 'string' ? error : (error as Error).message || '提交审批失败')
    } finally {
      setToolConfirmSubmitting(false)
    }
  }, [pendingToolConfirm])

  const resolveSessionConsent = useCallback(async (granted: boolean) => {
    const prompt = pendingSessionConsent
    if (!prompt) return
    setSessionConsentSubmitting(true)
    try {
      await api.chatRespondSessionConsent(prompt.conversationId, granted)
      setPendingSessionConsent(null)
    } catch (error) {
      setSessionConsentError(typeof error === 'string' ? error : (error as Error).message || '提交会话授权失败')
    } finally {
      setSessionConsentSubmitting(false)
    }
  }, [pendingSessionConsent])

  const handleModelChange = useCallback(async (providerId: string, model: string) => {
    const next = await chatApi.updateConversation(conversationId, { providerId, model })
    setConversation(next)
  }, [conversationId])

  const handleThinkingLevelChange = useCallback(async (level: ThinkingLevel | null) => {
    const next = await chatApi.updateConversation(conversationId, { thinkingLevel: level })
    setConversation(next)
  }, [conversationId])

  const handleRuntimeChange = useCallback(async (runtime: AgentRuntimeConfig) => {
    if (conversation && agentRuntimesEqual(normalizeAgentRuntime(conversation), runtime)) return
    const goal = conversation?.goal_state ?? conversation?.goalState
    if (goal && !['completed', 'cancelled', 'paused'].includes(goal.status)) {
      setConversation(await chatApi.pauseGoal(conversationId))
    }
    const next = await chatApi.setAgentRuntime(conversationId, runtime)
    setConversation(next)
  }, [conversation, conversationId])

  const handleExternalModelChange = useCallback(async (model: string, reasoning?: string | null) => {
    const current = normalizeAgentRuntime(conversation)
    await handleRuntimeChange(withExternalModel(current, model, reasoning))
  }, [conversation, handleRuntimeChange])

  const handleApprovalPolicyChange = useCallback(async (nextApprovalPolicy: string) => {
    setApprovalPolicy(nextApprovalPolicy)
    try {
      await updateSettingsCached((settings) => ({
        ...settings,
        chatTools: {
          ...settings.chatTools,
          approvalPolicy: nextApprovalPolicy,
        },
      }))
    } catch (err) {
      console.error('Failed to update approval policy:', err)
    }
  }, [])

  const pendingUserPromptRecord = pendingUserPrompt ? userPromptEventToRecord(pendingUserPrompt) : null
  const runtime = normalizeAgentRuntime(conversation)
  const usesChatRuntime = runtime.kind === 'chat'
  const usesExternalRuntime = runtime.kind === 'external'

  const runGoalMutation = useCallback(async (
    mutation: (id: string) => Promise<Conversation>,
    continueWhenActive = false,
  ) => {
    const updated = await mutation(conversationId)
    setConversation(updated)
    const goal = updated.goal_state ?? updated.goalState
    if (continueWhenActive && goal && (goal.status === 'active' || goal.status === 'verifying')) {
      void chatApi.continueGoal(conversationId).then(setConversation).catch((error) => {
        setStreamCoarse({ streamError: error instanceof Error ? error.message : String(error) })
      })
    }
  }, [conversationId])

  const displayMessages = executionOwner.overlayMessages(conversation?.id, conversation?.messages ?? [])

  const inputBarProps = usePopoutComposer({
    conversation,
    setConversation,
    conversationId,
    lang,
    displayMessages,
    streaming: streamCoarse.streaming,
    usesChatRuntime,
    usesExternalRuntime,
    runtime,
    onSend: handleSend,
    onCancel: handleCancel,
    cancelVisible: streamCoarse.streaming,
    cancelling: streamCoarse.cancelling,
    disabled: streamCoarse.streaming || executionOwner.snapshot(conversationId).inFlight,
  })

  const messageListProps: MessageListProps = {
    conversationId,
    messages: displayMessages,
    lang,
    sessionProviderId: conversation?.provider_id,
    sessionModel: conversation?.model,
  }

  return {
    conversation,
    loadError,
    runtime,
    usesChatRuntime,
    usesExternalRuntime,
    approvalPolicy,
    inputBarProps,
    messageListProps,
    streamError: streamCoarse.streamError,
    pendingToolConfirm,
    pendingSessionConsent,
    pendingUserPrompt,
    pendingUserPromptRecord,
    toolConfirmError,
    sessionConsentError,
    toolConfirmSubmitting,
    sessionConsentSubmitting,
    resolveToolConfirm,
    resolveSessionConsent,
    dismissUserPrompt: () => {
      const current = pendingUserPrompt
      const rest = pendingUserPromptsRef.current.filter((item) => (
        current ? item.toolCallId !== current.toolCallId : true
      ))
      pendingUserPromptsRef.current = rest
      setPendingUserPrompt(rest[0] ?? null)
    },
    hookWarning,
    dismissHookWarning: () => setHookWarning(null),
    handleModelChange,
    handleThinkingLevelChange,
    handleRuntimeChange,
    handleExternalModelChange,
    handleApprovalPolicyChange,
    editGoal: (objective: string) => runGoalMutation((id) => chatApi.editGoal(id, objective), true),
    pauseGoal: () => runGoalMutation(chatApi.pauseGoal),
    resumeGoal: () => runGoalMutation(chatApi.resumeGoal, true),
    cancelGoal: () => runGoalMutation(chatApi.cancelGoal),
  }
}
