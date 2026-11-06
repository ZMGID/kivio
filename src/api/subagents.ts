import { invoke } from '@tauri-apps/api/core'
import type { SubagentActivity, SubagentRoster, SubagentSendReceipt, SubagentTranscriptCursor, SubagentTranscriptPage, SubagentView } from '../generated/chatProtocol'

export type { SubagentActivity, SubagentRoster, SubagentTranscriptEntry, SubagentView } from '../generated/chatProtocol'

export type SubagentTarget = { conversationId: string; agentId: string; expectedGeneration: number }

export const subagentsApi = {
  snapshot: (conversationId: string) => invoke<SubagentRoster>('chat_subagent_snapshot', { conversationId }),
  waiting: (conversationId: string) => invoke<boolean>('chat_subagent_waiting', { conversationId }),
  send: (target: SubagentTarget, content: string) => invoke<SubagentSendReceipt>('chat_subagent_send', { ...target, content }),
  kill: (target: SubagentTarget) => invoke<SubagentView>('chat_subagent_kill', target),
  activity: (conversationId: string) => invoke<SubagentActivity[]>('chat_subagent_activity', { conversationId }),
  transcript: (target: SubagentTarget, cursor: SubagentTranscriptCursor | null) => invoke<SubagentTranscriptPage>('chat_subagent_transcript', { ...target, cursor }),
}

export function record(value: unknown): Record<string, unknown> {
  return isRecord(value) ? value : {}
}

function isRecord(value: unknown): value is Record<string, unknown> { return value !== null && typeof value === 'object' && !Array.isArray(value) }

export function parsedRecord(value: unknown): Record<string, unknown> {
  if (typeof value !== 'string') return record(value)
  try { return record(JSON.parse(value)) } catch { return {} }
}

export function text(value: unknown): string { return typeof value === 'string' ? value : '' }

export type SubagentProgress = {
  index?: number | null
  status?: string | null
  agent?: string | null
  task?: string | null
  description?: string | null
  currentTool?: string | null
  currentToolIntent?: string | null
  currentToolArgs?: string | null
  currentToolStartMs?: number | null
  recentTools?: { tool: string; args: string; intent?: string | null; isError?: boolean | null; endMs: number }[] | null
  recentOutput?: string[] | null
  steps?: string[] | null
  preview?: string | null
  toolCount?: number | null
  requests?: number | null
  tokens?: number | null
  contextTokens?: number | null
  contextWindow?: number | null
  cost?: number | null
  durationMs?: number | null
  completionPercent?: number | null
  stage?: string | null
  watch?: string | null
  error?: string | null
}

const stringFields = ['status', 'agent', 'task', 'description', 'currentTool', 'currentToolIntent', 'currentToolArgs', 'preview', 'stage', 'watch', 'error'] satisfies (keyof SubagentProgress)[]
const numberFields = ['index', 'currentToolStartMs', 'toolCount', 'requests', 'tokens', 'contextTokens', 'contextWindow', 'cost', 'durationMs', 'completionPercent'] satisfies (keyof SubagentProgress)[]

// The desktop CSP forbids Ajv's runtime code generation.
function validateProgress(value: unknown): value is SubagentProgress {
  if (!isRecord(value)) return false
  return stringFields.every(key => value[key] == null || typeof value[key] === 'string')
    && numberFields.every(key => value[key] == null || (typeof value[key] === 'number' && Number.isFinite(value[key]) && value[key] >= 0))
    && [value.recentOutput, value.steps].every(lines => lines == null || (Array.isArray(lines) && lines.every(line => typeof line === 'string')))
    && (value.recentTools == null || (Array.isArray(value.recentTools) && value.recentTools.every(tool => isRecord(tool)
      && typeof tool.tool === 'string' && typeof tool.args === 'string'
      && (tool.intent == null || typeof tool.intent === 'string') && (tool.isError == null || typeof tool.isError === 'boolean')
      && typeof tool.endMs === 'number' && Number.isFinite(tool.endMs))))
}

export function subagentProgress(value: unknown): SubagentProgress | null {
  return validateProgress(value) ? value : null
}
