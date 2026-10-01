export type ScheduleRule =
  | { kind: 'once'; at: number }
  | { kind: 'interval'; minutes: number; anchorAt?: number | null }
  | { kind: 'daily'; hour: number; minute: number }
  | { kind: 'weekly'; weekdays: number[]; hour: number; minute: number }
  | { kind: 'monthly'; days: number[]; hour: number; minute: number }
  | { kind: 'yearly'; month: number; day: number; hour: number; minute: number }
  | { kind: 'cron'; expr: string }

export type ScheduledTaskTarget =
  // Input only: the conversation is created and bound when the task is saved.
  | { kind: 'newConversation'; providerId?: string | null; model?: string | null; projectId?: string | null; thinkingLevel?: 'off' | 'low' | 'medium' | 'high' | null }
  | { kind: 'conversation'; conversationId: string }

export interface ScheduledTask {
  id: string
  name: string
  prompt: string
  schedule: ScheduleRule
  conversationId: string
  enabled: boolean
  status: 'active' | 'completed'
  nextRunAt: number | null
  lastRunAt: number | null
  runCount: number
  lastError: string | null
  source: 'user' | 'chat'
  createdAt: number
  updatedAt: number
}

export interface ScheduledTaskInput {
  id?: string | null
  name: string
  prompt: string
  schedule: ScheduleRule
  target: ScheduledTaskTarget
  enabled?: boolean
}

export interface ScheduledTaskRun {
  id: string
  taskId: string
  trigger: 'schedule' | 'manual'
  scheduledAt: number | null
  status: 'queued' | 'running' | 'succeeded' | 'failed' | 'skipped' | 'interrupted'
  conversationId: string | null
  error: string | null
  createdAt: number
  startedAt: number | null
  finishedAt: number | null
}

export interface ScheduledTasksChangedEvent {
  taskId: string
  run: ScheduledTaskRun | null
}
