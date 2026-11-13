import { useId, useState } from 'react'
import { useT, type I18n } from '../components/i18n'
import { Check, ChevronDown, ChevronRight, Circle, CirclePause, CirclePlay, Pencil, Target, X } from 'lucide-react'
import { Button, IconButton } from '../components/Button'
import { TextArea } from '../settings/public/controls'
import type { GoalState } from './types'
import { formatTokens } from '../utils/tokens'
import { goalCompletedAt } from './goalPresentation'

const evidenceLabels: Record<string, keyof I18n> = {
  model_self_check: 'chatGoalEvidenceSelfCheck', tool_result: 'chatGoalEvidenceToolResult',
  source: 'chatGoalEvidenceSource', artifact: 'chatGoalEvidenceArtifact',
}

export function GoalCard({ goal, onEdit, onPause, onResume, onCancel }: {
  goal: GoalState
  onEdit: (objective: string) => void | Promise<void>
  onPause: () => void | Promise<void>
  onResume: () => void | Promise<void>
  onCancel: () => void | Promise<void>
}) {
  const t = useT()
  const [expandedGoal, setExpandedGoal] = useState<string | null>(null)
  const detailsId = useId()
  const goalKey = `${goal.id}:${goal.version}`
  const expanded = expandedGoal === goalKey
  const [editing, setEditing] = useState<{ key: string; text: string } | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const isEditing = editing?.key === goalKey
  const verified = goal.criteria.filter((item) => item.verified).length
  const running = goal.status === 'active' || goal.status === 'verifying'
  const terminal = goal.status === 'completed' || goal.status === 'cancelled'
  const usage = goal.total_tokens ?? goal.totalTokens
  const completedAt = goalCompletedAt(goal)
  const completedDate = completedAt != null && Number.isFinite(completedAt)
    ? new Date(completedAt * 1000) : null
  const act = async (action: () => void | Promise<void>) => {
    setBusy(true)
    setError('')
    try { await action() } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause))
    } finally { setBusy(false) }
  }
  const edit = () => { setEditing({ key: goalKey, text: goal.objective }); setError('') }
  const save = () => void act(async () => {
    const objective = editing?.text.trim()
    if (!objective) return
    await onEdit(objective)
    setEditing(null)
  })
  return (
    <div className="min-w-0 flex-1" data-chat-goal-card>
      <div className="flex w-full items-start gap-2 rounded-md px-1 py-0.5 text-[12px] text-neutral-700">
        <Target size={15} className="mt-0.5 shrink-0" />
        <div className="min-w-0 flex-1">
          <button type="button" onClick={() => setExpandedGoal(expanded ? null : goalKey)}
            aria-label={expanded ? t.chatGoalCollapse : t.chatGoalExpand} aria-expanded={expanded}
            aria-controls={detailsId} className="flex w-full items-center gap-2 text-left">
            {expanded ? <ChevronDown size={13} className="shrink-0" /> : <ChevronRight size={13} className="shrink-0" />}
            <span className="font-semibold">Goal</span>
            <span className="shrink-0 rounded-full bg-violet-200/70 px-1.5 py-0.5 text-[10px] dark:bg-violet-300/15">{goal.status}</span>
            {goal.criteria.length > 0 && <span className="shrink-0 whitespace-nowrap text-violet-600 dark:text-violet-300">{t.chatGoalVerified.replace('{done}', String(verified)).replace('{total}', String(goal.criteria.length))}</span>}
            {usage != null && <span title={`${usage.toLocaleString()} tokens`} className="hidden shrink-0 whitespace-nowrap text-violet-500 sm:inline dark:text-violet-300">{formatTokens(usage)} tokens</span>}
            {completedDate && <time dateTime={completedDate.toISOString()} title={completedDate.toLocaleString()}
              className="shrink-0 whitespace-nowrap text-neutral-500">
              {t.chatGoalCompletedAt} {completedDate.toLocaleString(undefined, { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hour12: false })}
            </time>}
            <span className="min-w-0 flex-1 truncate opacity-75" title={goal.objective}>{goal.objective}</span>
          </button>
          {isEditing && <div className="mt-2" role="group" aria-label={t.chatGoalEdit}>
            <label htmlFor={`${detailsId}-editor`} className="mb-1 block text-xs font-medium">{t.chatGoalContent}</label>
            <TextArea id={`${detailsId}-editor`} autoFocus value={editing.text} maxLength={4000}
              disabled={busy} rows={4} onChange={(text) => setEditing({ key: goalKey, text })}
              onKeyDown={(event) => {
                if (event.key === 'Escape' && !busy) { event.preventDefault(); setEditing(null) }
                if (event.key === 'Enter' && (event.ctrlKey || event.metaKey) && !busy && editing.text.trim()) {
                  event.preventDefault(); save()
                }
              }} />
            <div className="mt-2 flex items-center justify-between gap-2">
              <span className="text-[11px] text-neutral-500">{goal.status === 'paused' ? t.chatGoalSaveKeepsPaused : t.chatGoalSaveContinues} · {t.chatGoalSaveShortcut}</span>
              <div className="flex gap-2">
                <Button size="sm" variant="ghost" disabled={busy} onClick={() => setEditing(null)}>{t.chatGoalCancelEdit}</Button>
                <Button size="sm" variant="primary" disabled={busy || !editing.text.trim() || editing.text.trim() === goal.objective}
                  onClick={save}>{busy ? t.chatGoalSaving : t.chatGoalSave}</Button>
              </div>
            </div>
          </div>}
          {error && <div role="alert" className="mt-2 text-xs text-danger">{error}</div>}
          {expanded && !isEditing && <div id={detailsId} className="mt-2 max-h-48 overflow-y-auto">
          <div className="whitespace-pre-wrap break-words font-medium">{goal.objective}</div>
          {goal.criteria.length > 0 && (
            <ul className="mt-1.5 space-y-1" aria-label={t.chatGoalChecklist}>
              {goal.criteria.map((criterion) => (
                <li key={criterion.id} className="flex items-start gap-1.5 text-[11px] text-violet-800 dark:text-violet-200">
                  {criterion.verified
                    ? <Check size={12} className="mt-0.5 shrink-0 text-success" />
                    : <Circle size={10} className="mt-0.5 shrink-0 opacity-60" />}
                  <span className={criterion.verified ? 'line-through opacity-75' : ''}>{criterion.text}</span>
                  {criterion.verified && (criterion.evidence_kind ?? criterion.evidenceKind) && (
                    <span className="shrink-0 opacity-60">{t[evidenceLabels[criterion.evidence_kind ?? criterion.evidenceKind ?? ''] ?? 'chatGoalEvidence']}</span>
                  )}
                </li>
              ))}
            </ul>
          )}
          {(goal.progress_summary ?? goal.progressSummary ?? goal.status_reason ?? goal.statusReason) && (
            <div className="mt-1 line-clamp-2 text-[11px] text-violet-700 dark:text-violet-200">
              {goal.progress_summary ?? goal.progressSummary ?? goal.status_reason ?? goal.statusReason}
            </div>
          )}
          </div>}
        </div>
        {!terminal && <IconButton size="xs" disabled={busy} onClick={edit} label={t.chatGoalEdit}><Pencil /></IconButton>}
        {running ? (
          <IconButton size="xs" disabled={busy} onClick={() => void act(onPause)} label={t.chatGoalPause}><CirclePause /></IconButton>
        ) : !terminal ? (
          <IconButton size="xs" disabled={busy} onClick={() => void act(onResume)} label={t.chatGoalResume}><CirclePlay /></IconButton>
        ) : null}
        {!terminal && <IconButton size="xs" disabled={busy} onClick={() => void act(onCancel)} label={t.chatGoalStop}><X /></IconButton>}
      </div>
    </div>
  )
}
