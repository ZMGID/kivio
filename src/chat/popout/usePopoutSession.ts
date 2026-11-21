import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react'
import { api } from '../../api/tauri'
import { notifyError } from '../../components/appNoticeStore'
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
import { keepNewerTodoState, patchTodoState } from '../agentTodoState'
import { keepNewerContextMeasurement } from '../contextPanel'
import { findSubagentToolIndex, mergeSubagentProgress, messageToolCalls, userPromptEventToRecord, isStreamTerminal } from '../streamApply'
import { createChatExecutionOwner } from '../chatExecutionOwner'
import { createChatStreamLifecycleOwner, type StreamLifecycleResult } from '../chatStreamLifecycleOwner'
import { createStreamPreviewOwner } from '../streamPreviewOwner'
import { createRunInteractionInbox } from '../runInteractionInbox'
import {
  reset as resetStreamStore,
  setCoarse as setStreamCoarse,
  useStreamCoarse,
} from '../streamingStore'
import type { MessageListProps } from '../MessageList'
import type { AgentPlanState, AgentTodoState, Conversation, GoalState, PendingAttachment, ThinkingLevel } from '../types'
import { insertTextIntoComposer } from '../composerInsert'
import { usePopoutComposer } from './usePopoutComposer'
import { usePopoutWorkdir } from './usePopoutWorkdir'
import { useDetectedExternalAgents } from '../permissionModes'
import type { ChatHookPayload } from '../../api/tauri'
import type { Lang } from '../../components/i18n'

const EMPTY_MESSAGES: Conversation['messages'] = []

export function usePopoutSession(conversationId: string, lang: Lang) {
  const [conversation, setConversation] = useState<Conversation | null>(null)
  // A queued React update is not yet a visible replacement for the live preview.
  const committedConversationRef = useRef(conversation)
  useLayoutEffect(() => { committedConversationRef.current = conversation }, [conversation])
  const [loadError, setLoadError] = useState('')
  const [hookWarning, setHookWarning] = useState<ChatHookPayload | null>(null)
  const [approvalPolicy, setApprovalPolicy] = useState('readonly_auto_sensitive_confirm')

  const conversationIdRef = useRef(conversationId)
  conversationIdRef.current = conversationId
  const viewActiveRef = useRef(false)
  const incomingReadEpoch = useRef(0)
  const [executionOwner] = useState(createChatExecutionOwner)
  const [previewOwner] = useState(createStreamPreviewOwner)
  const [streamLifecycleOwner] = useState(() => createChatStreamLifecycleOwner(executionOwner, previewOwner))
  useSyncExternalStore(executionOwner.subscribe, executionOwner.getRevision)
  const [interactionInbox] = useState(() => createRunInteractionInbox({
    confirmTool: api.chatConfirmToolCall,
    respondConsent: api.chatRespondSessionConsent,
  }))
  const interactionSnapshot = useSyncExternalStore(interactionInbox.subscribe, interactionInbox.getSnapshot)
  const cancellingInteractions = executionOwner.isCancellationRequested(conversationId)
  const pendingToolConfirm = cancellingInteractions ? null : interactionSnapshot.toolConfirm
  const pendingSessionConsent = cancellingInteractions ? null : interactionSnapshot.sessionConsent
  const pendingUserPrompt = cancellingInteractions ? null : interactionSnapshot.userPrompt
  const streamCoarse = useStreamCoarse()
  const acceptPersistedConversation = useCallback((next: Conversation) => {
    setConversation((current) => current?.id === next.id && current.revision > next.revision
      ? current
      : keepNewerContextMeasurement(keepNewerTodoState(next, current), current))
  }, [])

  useEffect(() => {
    incomingReadEpoch.current += 1
    viewActiveRef.current = true
    previewOwner.attach()
    previewOwner.activate(conversationId)
    interactionInbox.activate(conversationId)
    let cancelled = false
    setLoadError('')
    void chatApi.getConversation(conversationId).then((conv) => {
      if (cancelled) return
      acceptPersistedConversation(conv)
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
      incomingReadEpoch.current += 1
      viewActiveRef.current = false
      executionOwner.observe({ kind: 'drop', conversationId })
      interactionInbox.observe({ kind: 'drop', conversationId })
      previewOwner.dispose()
      resetStreamStore()
    }
  }, [acceptPersistedConversation, conversationId, executionOwner, interactionInbox, previewOwner])

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
    if (isStreamTerminal(payload)) {
      interactionInbox.observe({ kind: 'runTerminal', conversationId: payload.conversationId, runId: payload.runId })
    }
    if (result.kind === 'started') {
      setHookWarning(null)
      interactionInbox.observe({ kind: 'runStarted', conversationId: payload.conversationId, runId: payload.runId })
      if (payload.snapshotPendingInteractions) {
        interactionInbox.reconcileRun(payload.conversationId, payload.runId, payload.snapshotPendingInteractions)
      }
    } else if (result.kind === 'ready') {
      previewOwner.freeze(payload.conversationId)
      settleExternalRun(result)
    }
  }, [interactionInbox, previewOwner, settleExternalRun, streamLifecycleOwner])


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
    const id = payload.parentConversationId
    if (id !== conversationIdRef.current) return
    const inFlight = executionOwner.snapshot(id).inFlight
    const preview = previewOwner.summary(id)
    if (inFlight && preview
      && (payload.parentRunId
        ? preview.runId === payload.parentRunId
        : Boolean(payload.parentToolCallId && preview.toolCalls.some((tool) => tool.id === payload.parentToolCallId)))
      && executionOwner.allowsStreamPayload({ conversationId: id, runId: payload.parentRunId })
      && executionOwner.observe({ kind: 'runEvent', conversationId: id, runId: payload.parentRunId })
      && previewOwner.projectDisplay({ kind: 'subagent', payload }).accepted) return
    setConversation((current) => {
      if (!current || current.id !== id || conversationIdRef.current !== id) return current
      let changed = false
      const messages = current.messages.map((message) => {
        const tools = messageToolCalls(message)
        const index = payload.parentToolCallId && payload.externalSessionId === undefined
          ? tools.findIndex((tool) => tool.id === payload.parentToolCallId)
          : inFlight ? -1 : findSubagentToolIndex(tools, payload)
        if (index < 0) return message
        changed = true
        const nextTools = tools.map((tool, i) => i === index ? mergeSubagentProgress(tool, payload) : tool)
        return { ...message, toolCalls: nextTools, tool_calls: nextTools }
      })
      return changed ? { ...current, messages } : current
    })
  }, [executionOwner, previewOwner])
  useTauriEvent(api.onChatToolConfirm, (payload) => {
    if (payload.conversationId !== conversationIdRef.current
      || !executionOwner.observe({ kind: 'runEvent', conversationId: payload.conversationId, runId: payload.runId })) return
    interactionInbox.observe({ kind: 'toolRequested', payload })
  }, [executionOwner, interactionInbox])

  useTauriEvent(api.onChatToolConfirmWithdraw, (payload) => {
    if (payload.conversationId !== conversationIdRef.current) return
    interactionInbox.observe({ kind: 'toolWithdrawn', ...payload })
  }, [interactionInbox])

  useTauriEvent(api.onChatSessionConsent, (payload) => {
    if (payload.conversationId !== conversationIdRef.current
      || !executionOwner.observe({ kind: 'runEvent', conversationId: payload.conversationId, runId: payload.runId })) return
    interactionInbox.observe({ kind: 'consentRequested', payload })
  }, [executionOwner, interactionInbox])

  useTauriEvent(api.onChatUserPrompt, (payload) => {
    if (payload.conversationId !== conversationIdRef.current
      || !executionOwner.observe({ kind: 'runEvent', conversationId: payload.conversationId, runId: payload.runId })) return
    interactionInbox.observe({ kind: 'userPromptRequested', payload })
  }, [executionOwner, interactionInbox])

  useTauriEvent(api.onChatUserPromptResolved, (payload) => {
    if (payload.conversationId !== conversationIdRef.current) return
    interactionInbox.observe({ kind: 'userAnswered', ...payload })
  }, [interactionInbox])

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
      ? patchTodoState(current, todoState, payload.revision)
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

  const refreshAgentMessages = useCallback((id: string) => {
    if (id !== conversationIdRef.current) return
    const epoch = incomingReadEpoch.current
    void chatApi.getConversation(id).then(updated => {
      if (epoch === incomingReadEpoch.current && id === conversationIdRef.current && viewActiveRef.current) acceptPersistedConversation(updated)
    }).catch(error => console.error('Failed to refresh agent messages:', error))
  }, [acceptPersistedConversation])
  useTauriEvent(api.onChatSubagentIrc, message => {
    if (!message.observation) refreshAgentMessages(message.conversationId)
  }, [refreshAgentMessages])
  useTauriEvent(api.onChatSubagentActivity, refreshAgentMessages, [refreshAgentMessages])

  const settlementPorts = useMemo<Parameters<typeof executionOwner.finish>[2]>(() => ({
    completeWithConversation: (id, persisted) => {
      if (conversationIdRef.current !== id) return
      interactionInbox.observe({ kind: 'drop', conversationId: id })
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
  }), [acceptPersistedConversation, interactionInbox, previewOwner])

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
        optimistic: { content: trimmed, attachments },
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
    )
    if (result.kind === 'failed') {
      console.error('Failed to cancel stream:', result.error)
      setStreamCoarse({ streamError: result.error.message })
    }
    if (result.kind !== 'ignored' && result.kind !== 'superseded') setStreamCoarse({ cancelling: false })
  }, [conversationId, executionOwner, streamLifecycleOwner])

  const resolveToolConfirm = useCallback((
    approved: boolean,
    always = false,
    permissionMode: string | null = null,
  ) => pendingToolConfirm
    ? interactionInbox.respondTool({ interactionId: pendingToolConfirm.interactionId, approved, always, permissionMode })
    : Promise.resolve(false), [interactionInbox, pendingToolConfirm])

  const resolveSessionConsent = useCallback((granted: boolean) => interactionInbox.respondConsent(granted), [interactionInbox])

  const handleModelChange = useCallback(async (providerId: string, model: string) => {
    const next = await chatApi.updateConversation(conversationId, { providerId, model })
    acceptPersistedConversation(next)
  }, [acceptPersistedConversation, conversationId])

  const handleThinkingLevelChange = useCallback(async (level: ThinkingLevel | null) => {
    const next = await chatApi.updateConversation(conversationId, { thinkingLevel: level })
    acceptPersistedConversation(next)
  }, [acceptPersistedConversation, conversationId])

  const handleRuntimeChange = useCallback(async (runtime: AgentRuntimeConfig) => {
    if (conversation && agentRuntimesEqual(normalizeAgentRuntime(conversation), runtime)) return
    const goal = conversation?.goal_state ?? conversation?.goalState
    if (goal && !['completed', 'cancelled', 'paused'].includes(goal.status)) {
      acceptPersistedConversation(await chatApi.pauseGoal(conversationId))
    }
    const next = await chatApi.setAgentRuntime(conversationId, runtime)
    acceptPersistedConversation(next)
  }, [acceptPersistedConversation, conversation, conversationId])

  const handleExternalModelChange = useCallback(async (model: string, reasoning?: string | null, serviceTier?: string | null) => {
    const current = normalizeAgentRuntime(conversation)
    await handleRuntimeChange(withExternalModel(current, model, reasoning, serviceTier))
  }, [conversation, handleRuntimeChange])

  const handleSandboxChange = useCallback(async (value: string) => {
    await handleRuntimeChange({ ...normalizeAgentRuntime(conversation), kind: 'external', externalSandbox: value })
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
  const detectedExternalAgents = useDetectedExternalAgents(conversationId)
  const externalAgent = detectedExternalAgents.find((agent) => agent.id === runtime.externalAgentId)

  const runGoalMutation = useCallback(async (
    mutation: (id: string) => Promise<Conversation>,
    continueWhenActive = false,
  ) => {
    const updated = await mutation(conversationId)
    acceptPersistedConversation(updated)
    const goal = updated.goal_state ?? updated.goalState
    if (continueWhenActive && goal && (goal.status === 'active' || goal.status === 'verifying')) {
      void chatApi.continueGoal(conversationId).then(acceptPersistedConversation).catch((error) => {
        setStreamCoarse({ streamError: error instanceof Error ? error.message : String(error) })
      })
    }
  }, [acceptPersistedConversation, conversationId])

  const displayMessages = executionOwner.overlayMessages(conversation?.id, conversation?.messages ?? [])
  const handleSwitchBranch = useCallback(async (branchId: string): Promise<boolean> => {
    if (usesExternalRuntime || streamCoarse.streaming || streamCoarse.streamFrozen || streamCoarse.cancelling
      || executionOwner.snapshot(conversationId).inFlight) return false
    try {
      const updated = await chatApi.switchBranch(conversationId, branchId)
      if (!viewActiveRef.current || conversationIdRef.current !== conversationId) return false
      acceptPersistedConversation(updated)
      setStreamCoarse({ streamError: '' })
      return true
    } catch (error) {
      notifyError(error instanceof Error ? error.message : String(error))
      return false
    }
  }, [acceptPersistedConversation, conversationId, executionOwner, streamCoarse.cancelling,
    streamCoarse.streamFrozen, streamCoarse.streaming, usesExternalRuntime])
  const workdir = usePopoutWorkdir(
    conversationId,
    conversation?.project_id ?? conversation?.projectId ?? null,
    `${runtime.kind}:${runtime.externalAgentId ?? ''}`,
    conversation?.worktree?.path,
  )

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
    detectedExternalAgents,
    onSend: handleSend,
    onCancel: handleCancel,
    cancelVisible: streamCoarse.streaming,
    cancelling: streamCoarse.cancelling,
    disabled: streamCoarse.streaming || executionOwner.snapshot(conversationId).inFlight,
    workdir,
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
    externalAgent,
    usesChatRuntime,
    usesExternalRuntime,
    approvalPolicy,
    workdir,
    inputBarProps,
    messageListProps,
    streamError: streamCoarse.streamError,
    pendingToolConfirm,
    pendingSessionConsent,
    pendingUserPrompt,
    pendingUserPromptRecord,
    userPromptDraft: interactionSnapshot.userPromptDraft,
    toolConfirmError: interactionSnapshot.toolConfirmError,
    sessionConsentError: interactionSnapshot.sessionConsentError,
    toolConfirmSubmitting: interactionSnapshot.toolConfirmSubmitting,
    sessionConsentSubmitting: interactionSnapshot.sessionConsentSubmitting,
    resolveToolConfirm,
    resolveSessionConsent,
    dismissUserPrompt: () => {
      if (pendingUserPrompt) interactionInbox.observe({ kind: 'userAnswered', ...pendingUserPrompt })
    },
    hookWarning,
    dismissHookWarning: () => setHookWarning(null),
    handleModelChange,
    handleThinkingLevelChange,
    handleRuntimeChange,
    handleExternalModelChange,
    handleApprovalPolicyChange,
    handleSandboxChange,
    handleSwitchBranch,
    editGoal: (objective: string) => runGoalMutation((id) => chatApi.editGoal(id, objective), true),
    pauseGoal: () => runGoalMutation(chatApi.pauseGoal),
    resumeGoal: () => runGoalMutation(chatApi.resumeGoal, true),
    cancelGoal: () => runGoalMutation(chatApi.cancelGoal),
  }
}
