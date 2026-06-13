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
  triggers?: string[]
  argument_hint?: string | null
  argumentHint?: string | null
  arguments?: string[]
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
  trace_id?: string | null
  traceId?: string | null
  span_id?: string | null
  spanId?: string | null
  structured_content?: unknown
  structuredContent?: unknown
}

export type AskUserPhase = 'awaiting' | 'answered' | 'skipped' | 'timeout' | 'cancelled'

export interface AskUserOption {
  id: string
  label: string
  description?: string | null
}

export interface AskUserQuestion {
  id: string
  prompt: string
  options: AskUserOption[]
  allow_multiple?: boolean
  allowMultiple?: boolean
  allow_custom?: boolean
  allowCustom?: boolean
}

export interface AskUserPromptPayload {
  title?: string | null
  questions: AskUserQuestion[]
}

export interface AskUserAnswer {
  selected_option_ids?: string[]
  selectedOptionIds?: string[]
  custom_text?: string | null
  customText?: string | null
}

export interface AskUserStructuredContent {
  askUser?: {
    phase?: AskUserPhase | string
    title?: string | null
    questions?: AskUserQuestion[]
    answers?: Record<string, AskUserAnswer>
  }
}

export interface ChatToolArtifact {
  name: string
  mime_type?: string
  mimeType?: string
  data_url?: string
  dataUrl?: string
  size_bytes?: number | null
  sizeBytes?: number | null
  path?: string | null
  filePath?: string | null
  localPath?: string | null
}

export type ChatMessageSegmentKind = 'text' | 'reasoning' | 'tool'

export type ChatMessageSegmentPhase = 'auxiliary' | 'plain' | 'tool_loop' | 'synthesis'

export interface ChatMessageSegment {
  id: string
  kind: ChatMessageSegmentKind
  phase: ChatMessageSegmentPhase
  order: number
  step_number?: number | null
  stepNumber?: number | null
  round?: number | null
  text?: string | null
  tool_call_id?: string | null
  toolCallId?: string | null
}

export interface ChatMessage {
  id: string
  role: 'user' | 'assistant'
  content: string
  attachments?: Attachment[]
  reasoning?: string
  artifacts?: ChatToolArtifact[]
  tool_calls?: ToolCallRecord[]
  toolCalls?: ToolCallRecord[]
  segments?: ChatMessageSegment[]
  api_messages?: unknown[]
  apiMessages?: unknown[]
  model_messages?: unknown[]
  modelMessages?: unknown[]
  active_skill_id?: string | null
  activeSkillId?: string | null
  run_entry?: 'send' | 'regenerate' | string | null
  runEntry?: 'send' | 'regenerate' | string | null
  stream_outcome?: 'completed' | 'cancelled' | 'error' | string | null
  streamOutcome?: 'completed' | 'cancelled' | 'error' | string | null
  /** Provider 报告的本条回复真实 token 用量（规划/合成/压缩累计）；不报告时缺省。 */
  usage?: MessageUsage | null
  timestamp: number
}

export interface MessageUsage {
  input_tokens?: number | null
  inputTokens?: number | null
  output_tokens?: number | null
  outputTokens?: number | null
  total_tokens?: number | null
  totalTokens?: number | null
  cached_input_tokens?: number | null
  cachedInputTokens?: number | null
  cache_creation_input_tokens?: number | null
  cacheCreationInputTokens?: number | null
  reasoning_tokens?: number | null
  reasoningTokens?: number | null
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
  root_path?: string | null
  rootPath?: string | null
  created_at: number
  updated_at: number
  createdAt?: number
  updatedAt?: number
}

export type AssistantToolPreset = 'inherit' | 'none' | 'skills' | 'all' | string

export interface AssistantQuickCommand {
  id: string
  name: string
  slash: string
  description?: string
  placeholder?: string
  prompt?: string
  starter_text?: string
  starterText?: string
  requires_suite_enabled?: boolean
  requiresSuiteEnabled?: boolean
  enabled?: boolean
}

export interface AssistantDataConnector {
  id: string
  name: string
  kind?: 'builtin_tool' | 'mcp' | 'skill_tool' | 'memory' | 'file' | 'web' | string
  description?: string
  tool_ids?: string[]
  toolIds?: string[]
  server_id?: string | null
  serverId?: string | null
  required?: boolean
  enabled?: boolean
  configured?: boolean
}

export interface AssistantKnowledgeSkill {
  id: string
  name: string
  description?: string
  trigger_phrases?: string[]
  triggerPhrases?: string[]
  skill_id?: string | null
  skillId?: string | null
  prompt?: string
  recommended_tools?: string[]
  recommendedTools?: string[]
  requires_connectors?: string[]
  requiresConnectors?: string[]
  enabled?: boolean
}

export interface ChatAssistant {
  id: string
  name: string
  description?: string
  icon?: string
  color?: string
  source?: 'builtin' | 'user' | 'imported' | string
  author?: string
  version?: string
  category?: string
  tags?: string[]
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
  quick_commands?: AssistantQuickCommand[]
  quickCommands?: AssistantQuickCommand[]
  data_connectors?: AssistantDataConnector[]
  dataConnectors?: AssistantDataConnector[]
  knowledge_skills?: AssistantKnowledgeSkill[]
  knowledgeSkills?: AssistantKnowledgeSkill[]
  enabled?: boolean
  installed?: boolean
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
  source?: 'builtin' | 'user' | 'imported' | string
  version?: string
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
  quick_commands?: AssistantQuickCommand[]
  quickCommands?: AssistantQuickCommand[]
  data_connectors?: AssistantDataConnector[]
  dataConnectors?: AssistantDataConnector[]
  knowledge_skills?: AssistantKnowledgeSkill[]
  knowledgeSkills?: AssistantKnowledgeSkill[]
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

export type AgentTodoStatus = 'pending' | 'in_progress' | 'completed' | 'cancelled'

export interface AgentTodoItem {
  id: string
  content: string
  description?: string | null
  status: AgentTodoStatus
  blocks?: string[]
  blocked_by?: string[]
  owner?: string | null
}

export interface AgentTodoState {
  items?: AgentTodoItem[]
  updated_at?: number
  updatedAt?: number
}

export type AgentPlanMode = 'act' | 'plan' | 'orchestrate'
export type AgentPlanStatus = 'empty' | 'draft' | 'approved'

export interface AgentPlanState {
  mode?: AgentPlanMode
  status?: AgentPlanStatus
  plan?: string | null
  updated_at?: number
  updatedAt?: number
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
  project_id?: string | null
  projectId?: string | null
  context_state?: ConversationContextState
  contextState?: ConversationContextState
  agent_todo_state?: AgentTodoState
  agentTodoState?: AgentTodoState
  agent_plan_state?: AgentPlanState
  agentPlanState?: AgentPlanState
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
  project_id?: string | null
  projectId?: string | null
  assistant_id?: string | null
  assistantId?: string | null
  assistant_name?: string | null
  assistantName?: string | null
}

export interface ConversationGroup {
  title: string
  conversations: ConversationListItem[]
}
