import { Check, Circle, CirclePause, CirclePlay, Pencil, Target, X } from 'lucide-react'
import type { GoalState } from './types'

export function GoalCard({ goal, onEdit, onPause, onResume, onCancel }: {
  goal: GoalState
  onEdit: (objective: string) => void | Promise<void>
  onPause: () => void | Promise<void>
  onResume: () => void | Promise<void>
  onCancel: () => void | Promise<void>
}) {
  const verified = goal.criteria.filter((item) => item.verified).length
  const running = goal.status === 'active' || goal.status === 'verifying'
  const terminal = goal.status === 'completed' || goal.status === 'cancelled'
  const usage = goal.total_tokens ?? goal.totalTokens
  const edit = () => {
    const value = window.prompt('编辑 Goal', goal.objective)?.trim()
    if (value && value !== goal.objective) void onEdit(value)
  }
  return (
    <div className="shrink-0 px-6 pt-2" data-chat-goal-card>
      <div className="mx-auto flex w-full max-w-4xl items-start gap-2 rounded-lg border border-violet-200/80 bg-violet-50/80 px-3 py-2 text-[12px] text-violet-950 dark:border-violet-400/20 dark:bg-violet-400/10 dark:text-violet-100">
        <Target size={15} className="mt-0.5 shrink-0" />
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="font-semibold">Goal</span>
            <span className="rounded-full bg-violet-200/70 px-1.5 py-0.5 text-[10px] dark:bg-violet-300/15">{goal.status}</span>
            {goal.criteria.length > 0 && <span className="text-violet-600 dark:text-violet-300">已验证 {verified}/{goal.criteria.length} 项</span>}
            {usage != null && <span className="text-violet-500 dark:text-violet-300">{usage.toLocaleString()} tokens</span>}
          </div>
          <div className="mt-0.5 truncate font-medium" title={goal.objective}>{goal.objective}</div>
          {goal.criteria.length > 0 && (
            <ul className="mt-1.5 space-y-1" aria-label="Goal 验收清单">
              {goal.criteria.map((criterion) => (
                <li key={criterion.id} className="flex items-start gap-1.5 text-[11px] text-violet-800 dark:text-violet-200">
                  {criterion.verified
                    ? <Check size={12} className="mt-0.5 shrink-0 text-emerald-600 dark:text-emerald-400" />
                    : <Circle size={10} className="mt-0.5 shrink-0 opacity-60" />}
                  <span className={criterion.verified ? 'line-through opacity-75' : ''}>{criterion.text}</span>
                  {criterion.verified && (criterion.evidence_kind ?? criterion.evidenceKind) && (
                    <span className="shrink-0 opacity-60">{(criterion.evidence_kind ?? criterion.evidenceKind) === 'model_self_check' ? '模型自检' : '检查通过'}</span>
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
        </div>
        {!terminal && <button type="button" onClick={edit} aria-label="编辑 Goal" className="rounded p-1 hover:bg-violet-200/70 dark:hover:bg-violet-300/15"><Pencil size={13} /></button>}
        {running ? (
          <button type="button" onClick={() => void onPause()} aria-label="暂停 Goal" className="rounded p-1 hover:bg-violet-200/70 dark:hover:bg-violet-300/15"><CirclePause size={14} /></button>
        ) : !terminal ? (
          <button type="button" onClick={() => void onResume()} aria-label="继续 Goal" className="rounded p-1 hover:bg-violet-200/70 dark:hover:bg-violet-300/15"><CirclePlay size={14} /></button>
        ) : null}
        {!terminal && <button type="button" onClick={() => void onCancel()} aria-label="终止 Goal" className="rounded p-1 hover:bg-violet-200/70 dark:hover:bg-violet-300/15"><X size={14} /></button>}
      </div>
    </div>
  )
}
