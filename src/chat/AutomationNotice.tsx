import { Button } from '../components/Button'
import { useT } from '../components/i18n'
import { automationHash, setHash } from './chatRoutes'
import { canonicalToolName } from './segments'
import { normalizeToolCallStatus } from './toolStatus'
import type { ToolCallRecord } from './types'

import { READ_TOOLS, isAutomationNoticeTool } from './automationNoticeTools'

function object(value: unknown): Record<string, unknown> | null {
  if (typeof value === 'string') {
    try { return object(JSON.parse(value)) } catch { return null }
  }
  return value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : null
}

function text(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

/** Mutation receipts are quiet timeline notices. Execution keeps its existing live view. */
export function AutomationNotice({ toolCall }: { toolCall: ToolCallRecord }) {
  const t = useT()
  const name = canonicalToolName(toolCall)
  if (!isAutomationNoticeTool(name)) return null
  const schedule = name.startsWith('schedule_')
  const args = object(toolCall.arguments ?? toolCall.args ?? toolCall.input)
  const result = object(toolCall.structured_content ?? toolCall.structuredContent)
    ?? object(toolCall.result ?? toolCall.output ?? toolCall.result_preview ?? toolCall.resultPreview)
  const saved = object(result?.automation) ?? object(result?.created) ?? object(result?.updated)
  const requested = object(args?.automation)
  const taskName = text(saved?.name) || text(requested?.name) || text(args?.name)
  const id = text(saved?.id) || text(requested?.id) || text(args?.id)
  const status = normalizeToolCallStatus(toolCall.status)
  const failed = status === 'error' || Boolean(toolCall.error) || result?.ok === false
    || result?.type === 'automation_validation' || Boolean(result?.error)
  const cancelled = status === 'cancelled' || status === 'skipped'
  const ongoing = status === 'pending' || status === 'running'
  const read = READ_TOOLS.has(name)
  if (read && !failed && !cancelled && !ongoing) return null

  const labels = schedule ? {
    created: t.chatScheduleNoticeCreated,
    updated: t.chatScheduleNoticeUpdated,
    enabled: t.chatScheduleNoticeEnabled,
    paused: t.chatScheduleNoticePaused,
    deleted: t.chatScheduleNoticeDeleted,
    reading: t.chatScheduleNoticeReading,
    saving: t.chatScheduleNoticeSaving,
    changing: t.chatScheduleNoticeChanging,
    deleting: t.chatScheduleNoticeDeleting,
    failed: t.chatScheduleNoticeFailed,
    cancelled: t.chatScheduleNoticeCancelled,
  } : {
    created: t.chatAutomationNoticeCreated,
    updated: t.chatAutomationNoticeUpdated,
    enabled: t.chatAutomationNoticeEnabled,
    paused: t.chatAutomationNoticePaused,
    deleted: t.chatAutomationNoticeDeleted,
    reading: t.chatAutomationNoticeReading,
    saving: t.chatAutomationNoticeSaving,
    changing: t.chatAutomationNoticeChanging,
    deleting: t.chatAutomationNoticeDeleting,
    failed: t.chatAutomationNoticeFailed,
    cancelled: t.chatAutomationNoticeCancelled,
  }
  const deleting = name.endsWith('_delete')
  const dryRun = result?.dryRun === true || args?.dry_run === true
  let label: string
  if (failed) label = labels.failed
  else if (cancelled) label = labels.cancelled
  else if (ongoing) {
    label = read ? labels.reading : deleting ? labels.deleting
      : dryRun ? t.chatAutomationNoticeValidating
        : name === 'automation_set_enabled' || name === 'schedule_update' ? labels.changing : labels.saving
  } else if (deleting) label = labels.deleted
  else if (dryRun) label = t.chatAutomationNoticeValidated
  else if (name === 'automation_set_enabled' || (name === 'schedule_update' && typeof args?.enabled === 'boolean')) {
    const enabled = saved?.enabled ?? args?.enabled
    label = enabled === false ? labels.paused : enabled === true ? labels.enabled : labels.updated
  } else if (name === 'schedule_create' || result?.created === true) label = labels.created
  else if (name === 'schedule_update' || result?.created === false) label = labels.updated
  else label = t.chatAutomationNoticeSaved

  const summary = <span className="flex min-w-0 max-w-full flex-wrap justify-center gap-x-1.5 gap-y-1 whitespace-normal text-[13px] text-[var(--text-muted)] [overflow-wrap:anywhere]">
    <span>{label}</span>
    {taskName && <><span aria-hidden="true">·</span><span>{taskName}</span></>}
  </span>
  const rawError = toolCall.error ?? result?.error
  const errorObject = object(rawError)
  const reason = text(errorObject?.message) || text(errorObject?.error) || (typeof rawError === 'string' && !rawError.trim().startsWith('{') && !rawError.trim().startsWith('[') ? rawError.trim() : '')
  const canOpen = !failed && !cancelled && !ongoing && !deleting && !dryRun && (schedule || Boolean(id))
  return <div className="my-3 flex min-w-0 flex-col items-center gap-1" role={failed ? 'alert' : undefined}>
    {canOpen ? <Button variant="ghost" size="sm" className="max-w-full" aria-label={`${label}${taskName ? ` · ${taskName}` : ''}`}
      onClick={() => setHash(schedule ? '#chat/schedules' : automationHash(id))}>{summary}</Button> : summary}
    {failed && reason && <p className="max-w-full text-center text-[12px] text-[var(--text-danger)] [overflow-wrap:anywhere]">{reason.length > 240 ? `${reason.slice(0, 240)}…` : reason}</p>}
  </div>
}
