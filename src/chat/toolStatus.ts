import type { ToolCallStatus } from './types'

export function normalizeToolCallStatus(status?: string): ToolCallStatus {
  switch (status) {
    case 'running':
    case 'in_progress':
    case 'calling':
    case 'executing':
      return 'running'
    case 'completed':
    case 'success':
    case 'succeeded':
      return 'completed'
    case 'error':
    case 'failed':
      return 'error'
    case 'skipped':
      return 'skipped'
    case 'cancelled':
    case 'canceled':
      return 'cancelled'
    case 'pending':
    case 'queued':
    default:
      return 'pending'
  }
}

export function isToolCallErrorStatus(status?: string): boolean {
  return normalizeToolCallStatus(status) === 'error'
}
