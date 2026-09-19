/** Serialized Goal state shared by the backend bridge and chat presentation. */
export type GoalStatus = 'active' | 'verifying' | 'waiting' | 'paused' | 'blocked' | 'completed' | 'cancelled'

export interface GoalCriterion {
  id: string
  text: string
  verified: boolean
  evidence?: string | null
  evidence_kind?: 'model_self_check' | 'tool_result' | 'source' | 'artifact' | string | null
  evidenceKind?: 'model_self_check' | 'tool_result' | 'source' | 'artifact' | string | null
  evidence_ref?: string | null
  evidenceRef?: string | null
}

export interface GoalState {
  id: string
  version: number
  objective: string
  status: GoalStatus
  criteria: GoalCriterion[]
  status_reason?: string | null
  statusReason?: string | null
  progress_summary?: string | null
  progressSummary?: string | null
  progress_revision?: number
  progressRevision?: number
  active_run_id?: string | null
  activeRunId?: string | null
  automatic_runs?: number
  automaticRuns?: number
  no_progress_runs?: number
  noProgressRuns?: number
  last_response_fingerprint?: string | null
  lastResponseFingerprint?: string | null
  input_tokens?: number | null
  inputTokens?: number | null
  output_tokens?: number | null
  outputTokens?: number | null
  total_tokens?: number | null
  totalTokens?: number | null
  completed_at?: number | null
  completedAt?: number | null
  completed_message_id?: string | null
  completedMessageId?: string | null
  created_at?: number
  createdAt?: number
  updated_at?: number
  updatedAt?: number
}
