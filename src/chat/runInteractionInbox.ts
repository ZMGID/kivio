import type {
  ChatSessionConsentPayload,
  ChatToolConfirmPayload,
  ChatUserPromptPayload,
} from '../api/tauri'

export type RunInteractionEvent =
  | { kind: 'toolRequested'; payload: ChatToolConfirmPayload }
  | { kind: 'toolWithdrawn'; conversationId: string; toolCallId: string }
  | { kind: 'consentRequested'; payload: ChatSessionConsentPayload }
  | { kind: 'userPromptRequested'; payload: ChatUserPromptPayload }
  | { kind: 'userAnswered'; conversationId: string; runId: string; toolCallId: string }
  | { kind: 'runStarted' | 'runTerminal'; conversationId: string; runId: string }
  | { kind: 'drop'; conversationId: string }

export interface RunInteractionSnapshot {
  activeConversationId: string | null
  toolConfirm: ChatToolConfirmPayload | null
  toolConfirmSubmitting: boolean
  toolConfirmError: string
  sessionConsent: ChatSessionConsentPayload | null
  sessionConsentSubmitting: boolean
  sessionConsentError: string
  userPrompt: ChatUserPromptPayload | null
  pendingToolConversationIds: readonly string[]
}

interface RunInteractionTransport {
  confirmTool: (
    toolCallId: string,
    approved: boolean,
    always: boolean,
    permissionMode: string | null,
  ) => Promise<void>
  respondConsent: (conversationId: string, granted: boolean) => Promise<void>
}

const requestKey = (conversationId: string, runId: string, itemId: string) =>
  JSON.stringify([conversationId, runId, itemId])
const runKey = (conversationId: string, runId: string) => JSON.stringify([conversationId, runId])

function rememberLimited(set: Set<string>, key: string) {
  set.add(key)
  // These tokens only guard a short tail of delayed protocol packets, not history.
  if (set.size > 128) set.delete(set.values().next().value!)
}

function errorMessage(value: unknown, fallback: string): string {
  if (typeof value === 'string' && value) return value
  if (value instanceof Error && value.message) return value.message
  return fallback
}

/** Owns transient run-interaction queues and their one-shot submission rights.
 * The caller fences incoming protocol events by execution identity before observe(). */
export function createRunInteractionInbox(transport: RunInteractionTransport) {
  const tools = new Map<string, ChatToolConfirmPayload[]>()
  const consents = new Map<string, ChatSessionConsentPayload>()
  const users = new Map<string, ChatUserPromptPayload[]>()
  const toolSubmitting = new Set<string>()
  const consentSubmitting = new Set<string>()
  const toolErrors = new Map<string, string>()
  const consentErrors = new Map<string, string>()
  const startedRuns = new Set<string>()
  const terminalRuns = new Set<string>()
  const answeredTools = new Set<string>()
  const withdrawnToolIds = new Set<string>()
  const answeredUsers = new Set<string>()
  const answeredConsents = new Set<string>()
  const listeners = new Set<() => void>()
  let activeConversationId: string | null = null
  let snapshot: RunInteractionSnapshot = {
    activeConversationId: null,
    toolConfirm: null,
    toolConfirmSubmitting: false,
    toolConfirmError: '',
    sessionConsent: null,
    sessionConsentSubmitting: false,
    sessionConsentError: '',
    userPrompt: null,
    pendingToolConversationIds: [],
  }

  const hasTool = (prompt: ChatToolConfirmPayload) =>
    (tools.get(prompt.conversationId) ?? []).some((item) =>
      item.runId === prompt.runId && item.toolCallId === prompt.toolCallId)

  const publish = () => {
    const conversationId = activeConversationId
    const tool = conversationId ? tools.get(conversationId)?.[0] ?? null : null
    const consent = conversationId ? consents.get(conversationId) ?? null : null
    const user = conversationId ? users.get(conversationId)?.[0] ?? null : null
    const pendingToolConversationIds = [...tools.keys()].filter((id) => (tools.get(id)?.length ?? 0) > 0)
    const next: RunInteractionSnapshot = {
      activeConversationId: conversationId,
      toolConfirm: tool,
      toolConfirmSubmitting: tool ? toolSubmitting.has(requestKey(tool.conversationId, tool.runId, tool.toolCallId)) : false,
      toolConfirmError: tool ? toolErrors.get(requestKey(tool.conversationId, tool.runId, tool.toolCallId)) ?? '' : '',
      sessionConsent: consent,
      sessionConsentSubmitting: consent ? consentSubmitting.has(consent.conversationId) : false,
      sessionConsentError: consent ? consentErrors.get(runKey(consent.conversationId, consent.runId)) ?? '' : '',
      userPrompt: user,
      pendingToolConversationIds,
    }
    if (
      snapshot.activeConversationId === next.activeConversationId
      && snapshot.toolConfirm === next.toolConfirm
      && snapshot.toolConfirmSubmitting === next.toolConfirmSubmitting
      && snapshot.toolConfirmError === next.toolConfirmError
      && snapshot.sessionConsent === next.sessionConsent
      && snapshot.sessionConsentSubmitting === next.sessionConsentSubmitting
      && snapshot.sessionConsentError === next.sessionConsentError
      && snapshot.userPrompt === next.userPrompt
      && snapshot.pendingToolConversationIds.length === next.pendingToolConversationIds.length
      && snapshot.pendingToolConversationIds.every((id, index) => id === next.pendingToolConversationIds[index])
    ) return
    snapshot = next
    for (const listener of listeners) listener()
  }

  const clearRun = (conversationId: string, runId: string) => {
    for (const item of tools.get(conversationId) ?? []) {
      if (item.runId === runId) toolErrors.delete(requestKey(item.conversationId, item.runId, item.toolCallId))
    }
    const remainingTools = (tools.get(conversationId) ?? []).filter((item) => item.runId !== runId)
    if (remainingTools.length) tools.set(conversationId, remainingTools)
    else tools.delete(conversationId)
    if (consents.get(conversationId)?.runId === runId) consents.delete(conversationId)
    consentErrors.delete(runKey(conversationId, runId))
    const remainingUsers = (users.get(conversationId) ?? []).filter((item) => item.runId !== runId)
    if (remainingUsers.length) users.set(conversationId, remainingUsers)
    else users.delete(conversationId)
  }

  const observe = (event: RunInteractionEvent): boolean => {
    if (event.kind === 'toolRequested') {
      const prompt = event.payload
      const key = requestKey(prompt.conversationId, prompt.runId, prompt.toolCallId)
      if (terminalRuns.has(runKey(prompt.conversationId, prompt.runId))
        || answeredTools.has(key)
        || withdrawnToolIds.has(JSON.stringify([prompt.conversationId, prompt.toolCallId]))
        || hasTool(prompt)) return false
      tools.set(prompt.conversationId, [...(tools.get(prompt.conversationId) ?? []), prompt])
      toolErrors.delete(key)
    } else if (event.kind === 'toolWithdrawn') {
      rememberLimited(withdrawnToolIds, JSON.stringify([event.conversationId, event.toolCallId]))
      const previous = tools.get(event.conversationId) ?? []
      const removed = previous.filter((item) => item.toolCallId === event.toolCallId)
      const rest = previous.filter((item) => item.toolCallId !== event.toolCallId)
      for (const item of removed) {
        const key = requestKey(item.conversationId, item.runId, item.toolCallId)
        rememberLimited(answeredTools, key)
        toolErrors.delete(key)
      }
      if (rest.length) tools.set(event.conversationId, rest)
      else tools.delete(event.conversationId)
    } else if (event.kind === 'consentRequested') {
      const prompt = event.payload
      const key = runKey(prompt.conversationId, prompt.runId)
      if (terminalRuns.has(key) || answeredConsents.has(key) || consents.get(prompt.conversationId)?.runId === prompt.runId) return false
      consents.set(prompt.conversationId, prompt)
      consentErrors.delete(key)
    } else if (event.kind === 'userPromptRequested') {
      const prompt = event.payload
      const key = requestKey(prompt.conversationId, prompt.runId, prompt.toolCallId)
      if (terminalRuns.has(runKey(prompt.conversationId, prompt.runId)) || answeredUsers.has(key)) return false
      const queue = users.get(prompt.conversationId) ?? []
      if (queue.some((item) => item.runId === prompt.runId && item.toolCallId === prompt.toolCallId)) return false
      users.set(prompt.conversationId, [...queue, prompt])
    } else if (event.kind === 'userAnswered') {
      rememberLimited(answeredUsers, requestKey(event.conversationId, event.runId, event.toolCallId))
      const rest = (users.get(event.conversationId) ?? [])
        .filter((item) => item.runId !== event.runId || item.toolCallId !== event.toolCallId)
      if (rest.length) users.set(event.conversationId, rest)
      else users.delete(event.conversationId)
    } else if (event.kind === 'runStarted') {
      const key = runKey(event.conversationId, event.runId)
      if (terminalRuns.has(key) || startedRuns.has(key)) return false
      rememberLimited(startedRuns, key)
      clearRun(event.conversationId, event.runId)
    } else if (event.kind === 'runTerminal') {
      rememberLimited(terminalRuns, runKey(event.conversationId, event.runId))
      clearRun(event.conversationId, event.runId)
    } else {
      for (const item of tools.get(event.conversationId) ?? []) {
        rememberLimited(answeredTools, requestKey(item.conversationId, item.runId, item.toolCallId))
        toolErrors.delete(requestKey(item.conversationId, item.runId, item.toolCallId))
      }
      for (const item of users.get(event.conversationId) ?? []) {
        rememberLimited(answeredUsers, requestKey(item.conversationId, item.runId, item.toolCallId))
      }
      const consent = consents.get(event.conversationId)
      if (consent) {
        rememberLimited(answeredConsents, runKey(consent.conversationId, consent.runId))
        consentErrors.delete(runKey(consent.conversationId, consent.runId))
      }
      tools.delete(event.conversationId)
      users.delete(event.conversationId)
      consents.delete(event.conversationId)
    }
    publish()
    return true
  }

  const activate = (conversationId: string | null) => {
    if (activeConversationId === conversationId) return
    activeConversationId = conversationId
    if (conversationId) {
      for (const item of tools.get(conversationId) ?? []) {
        toolErrors.delete(requestKey(item.conversationId, item.runId, item.toolCallId))
      }
      const consent = consents.get(conversationId)
      if (consent) consentErrors.delete(runKey(consent.conversationId, consent.runId))
    }
    publish()
  }

  const respondTool = async ({ approved, always = false, permissionMode = null }: {
    approved: boolean
    always?: boolean
    permissionMode?: string | null
  }): Promise<boolean> => {
    const prompt = activeConversationId ? tools.get(activeConversationId)?.[0] : undefined
    if (!prompt) return false
    const key = requestKey(prompt.conversationId, prompt.runId, prompt.toolCallId)
    if (toolSubmitting.has(key)) return false
    toolSubmitting.add(key)
    toolErrors.delete(key)
    publish()
    try {
      await transport.confirmTool(prompt.toolCallId, approved, always, permissionMode)
      rememberLimited(answeredTools, key)
      const rest = (tools.get(prompt.conversationId) ?? [])
        .filter((item) => item.runId !== prompt.runId || item.toolCallId !== prompt.toolCallId)
      if (rest.length) tools.set(prompt.conversationId, rest)
      else tools.delete(prompt.conversationId)
      publish()
      return true
    } catch (error) {
      if (hasTool(prompt)) toolErrors.set(key, errorMessage(error, '提交审批失败，请重试'))
      publish()
      return false
    } finally {
      toolSubmitting.delete(key)
      publish()
    }
  }

  const respondConsent = async (granted: boolean): Promise<boolean> => {
    const prompt = activeConversationId ? consents.get(activeConversationId) : undefined
    if (!prompt || consentSubmitting.has(prompt.conversationId)) return false
    const key = runKey(prompt.conversationId, prompt.runId)
    consentSubmitting.add(prompt.conversationId)
    consentErrors.delete(key)
    publish()
    try {
      await transport.respondConsent(prompt.conversationId, granted)
      rememberLimited(answeredConsents, key)
      if (consents.get(prompt.conversationId)?.runId === prompt.runId) consents.delete(prompt.conversationId)
      publish()
      return true
    } catch (error) {
      if (consents.get(prompt.conversationId)?.runId === prompt.runId) {
        consentErrors.set(key, errorMessage(error, '提交会话授权失败，请重试'))
      }
      publish()
      return false
    } finally {
      consentSubmitting.delete(prompt.conversationId)
      publish()
    }
  }

  return {
    activate,
    observe,
    respondTool,
    respondConsent,
    getSnapshot: () => snapshot,
    subscribe: (listener: () => void) => {
      listeners.add(listener)
      return () => { listeners.delete(listener) }
    },
  }
}
