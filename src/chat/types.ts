// Chat 前端类型定义

export const TOOL_CALL_STATUSES = [
  'pending',
  'running',
  'success',
  'completed',
  'error',
  'skipped',
  'cancelled',
] as const

export type ToolCallStatus = (typeof TOOL_CALL_STATUSES)[number]

export interface SkillMeta {
  id: string
  name: string
  description: string
  source?: 'builtin' | 'user' | 'external' | string
  path?: string
  recommended_tools?: string[]
  recommendedTools?: string[]
  disable_model_invocation?: boolean
  disableModelInvocation?: boolean
  files?: SkillFileEntry[]
  enabled?: boolean
}

export interface SkillFileEntry {
  relative_path?: string
  relativePath?: string
  kind?: string
  size_bytes?: number
  sizeBytes?: number
}

export interface SkillDetail extends SkillMeta {
  content?: string
  body?: string
  frontmatter?: Record<string, unknown>
  updated_at?: number
  updatedAt?: number
}

export interface ToolCallRecord {
  id: string
  conversationId?: string
  runId?: string
  messageId?: string
  toolCallId?: string
  call_id?: string
  callId?: string
  tool_name?: string
  toolName?: string
  name?: string
  server_id?: string
  serverId?: string
  server_name?: string
  serverName?: string
  source?: string
  status?: ToolCallStatus
  started_at?: number
  startedAt?: number
  completed_at?: number
  completedAt?: number
  duration_ms?: number
  durationMs?: number
  arguments?: unknown
  args?: unknown
  input?: unknown
  argument_preview?: string
  argumentPreview?: string
  argumentsPreview?: string
  result?: unknown
  output?: unknown
  result_preview?: string
  resultPreview?: string
  error?: string
  round?: number
  sensitive?: boolean
  requires_confirmation?: boolean
  requiresConfirmation?: boolean
  artifacts?: ChatToolArtifact[]
}

export interface ChatToolArtifact {
  name: string
  mime_type?: string
  mimeType?: string
  data_url?: string
  dataUrl?: string
  size_bytes?: number | null
  sizeBytes?: number | null
}

export interface ChatMessage {
  id: string
  role: 'user' | 'assistant'
  content: string
  attachments?: Attachment[]
  reasoning?: string
  tool_calls?: ToolCallRecord[]
  toolCalls?: ToolCallRecord[]
  api_messages?: unknown[]
  apiMessages?: unknown[]
  model_messages?: unknown[]
  modelMessages?: unknown[]
  active_skill_id?: string | null
  activeSkillId?: string | null
  timestamp: number
}

export interface Attachment {
  id: string
  type: 'image' | 'file'
  name: string
  path: string
}

export interface PendingAttachment {
  id: string
  type: 'image' | 'file'
  name: string
  path: string
}

export interface ChatProject {
  id: string
  name: string
  description?: string | null
  color?: string | null
  created_at: number
  updated_at: number
  createdAt?: number
  updatedAt?: number
}

export type AssistantToolPreset = 'inherit' | 'none' | 'skills' | 'all' | string

export interface ChatAssistant {
  id: string
  name: string
  description?: string
  icon?: string
  color?: string
  system_prompt?: string
  systemPrompt?: string
  provider_id?: string
  providerId?: string
  model?: string
  skill_id?: string | null
  skillId?: string | null
  tool_preset?: AssistantToolPreset
  toolPreset?: AssistantToolPreset
  conversation_starters?: string[]
  conversationStarters?: string[]
  greeting?: string
  enabled?: boolean
  archived?: boolean
  built_in?: boolean
  builtIn?: boolean
  created_at: number
  updated_at: number
  createdAt?: number
  updatedAt?: number
}

export interface ChatAssistantSnapshot {
  id: string
  name: string
  description?: string
  system_prompt?: string
  systemPrompt?: string
  provider_id?: string
  providerId?: string
  model?: string
  skill_id?: string | null
  skillId?: string | null
  tool_preset?: AssistantToolPreset
  toolPreset?: AssistantToolPreset
  conversation_starters?: string[]
  conversationStarters?: string[]
  greeting?: string
}

export type ContextUsageStatus =
  | 'normal'
  | 'warning'
  | 'critical'
  | 'compressed'
  | 'stale'
  | 'unknown'
  | string

export interface ContextUsageSegment {
  id: string
  label: string
  estimated_tokens?: number
  estimatedTokens?: number
  color?: string | null
}

export interface ConversationContextSummary {
  id: string
  content: string
  source_message_ids?: string[]
  sourceMessageIds?: string[]
  source_until_message_id?: string
  sourceUntilMessageId?: string
  token_estimate_before?: number
  tokenEstimateBefore?: number
  token_estimate_after?: number
  tokenEstimateAfter?: number
  created_at?: number
  createdAt?: number
  provider_id?: string
  providerId?: string
  model?: string
  stale?: boolean
}

export interface ConversationContextState {
  estimated_input_tokens?: number
  estimatedInputTokens?: number
  context_window_tokens?: number | null
  contextWindowTokens?: number | null
  context_window_estimated?: boolean
  contextWindowEstimated?: boolean
  usage_ratio?: number | null
  usageRatio?: number | null
  status?: ContextUsageStatus
  segments?: ContextUsageSegment[]
  last_measured_at?: number
  lastMeasuredAt?: number
  last_compressed_at?: number | null
  lastCompressedAt?: number | null
  compressed_message_count?: number
  compressedMessageCount?: number
  summary?: ConversationContextSummary | null
  warning?: string | null
  warningMessage?: string | null
}

export interface Conversation {
  id: string
  title: string
  provider_id: string
  model: string
  messages: ChatMessage[]
  active_skill_id?: string | null
  activeSkillId?: string | null
  assistant_id?: string | null
  assistantId?: string | null
  assistant_snapshot?: ChatAssistantSnapshot | null
  assistantSnapshot?: ChatAssistantSnapshot | null
  created_at: number
  updated_at: number
  pinned?: boolean
  folder?: string
  context_state?: ConversationContextState
  contextState?: ConversationContextState
}

export interface ConversationListItem {
  id: string
  title: string
  preview: string
  provider_id: string
  model: string
  message_count: number
  created_at: number
  updated_at: number
  pinned?: boolean
  folder?: string
  assistant_id?: string | null
  assistantId?: string | null
  assistant_name?: string | null
  assistantName?: string | null
}

export interface ConversationGroup {
  title: string
  conversations: ConversationListItem[]
}
