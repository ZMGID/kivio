import { useEffect, useId, useRef, useState } from 'react'
import { ArrowUpRight, CalendarClock, Check, History, MessageSquare, Plus, Search, Trash2 } from 'lucide-react'
import { api } from '../../api/tauri'
import type { ScheduleRule as ScheduledTaskSchedule, ScheduledTask, ScheduledTaskRun, ScheduledTaskTarget } from '../../api/scheduledTaskContracts'
import { Button } from '../../components/Button'
import { confirmDialog } from '../../components/dialogQueue'
import { useLang, useT } from '../../components/i18n'
import { FieldBlock, Input, Select, TextArea, Toggle } from '../../settings/public/controls'
import { chatApi } from '../api'
import { ModelSelector } from '../ModelSelector'
import type { ChatProject, ConversationListItem, ConversationSearchHit } from '../types'
import { ScheduleDialog } from './ScheduleDialog'
import { clockTime, formatScheduleTime, friendlyScheduleTime, weekdayLabels } from './scheduleSummary'

function defaultSchedule(kind: ScheduledTaskSchedule['kind']): ScheduledTaskSchedule {
  switch (kind) {
    case 'once': return { kind, at: Math.floor(Date.now() / 1000) + 3600 }
    case 'interval': return { kind, minutes: 30 }
    case 'daily': return { kind, hour: 9, minute: 0 }
    case 'weekly': return { kind, weekdays: [1], hour: 9, minute: 0 }
    case 'monthly': return { kind, days: [1], hour: 9, minute: 0 }
    case 'yearly': return { kind, month: 1, day: 1, hour: 9, minute: 0 }
    case 'cron': return { kind, expr: '0 9 * * 1-5' }
  }
}

function localDateTime(at: number): string {
  const date = new Date(at * 1000)
  if (!Number.isFinite(date.getTime())) return ''
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}T${clockTime(date.getHours(), date.getMinutes())}`
}

type ConversationChoice = Pick<ConversationListItem, 'id' | 'title' | 'project_id' | 'projectId' | 'folder' | 'updated_at'>

export type RegisterTaskLeaveGuard = (guard: (() => Promise<boolean>) | null) => void

function TaskHistory({ taskId, onOpenConversation }: { taskId: string; onOpenConversation: (id: string) => void }) {
  const t = useT()
  const lang = useLang()
  const [runs, setRuns] = useState<ScheduledTaskRun[] | null>(null)
  const [error, setError] = useState('')
  const [page, setPage] = useState(0)
  const [deleting, setDeleting] = useState<string | null>(null)
  useEffect(() => {
    let disposed = false
    let generation = 0
    const reload = async () => {
      const current = ++generation
      try {
        const items = await api.scheduledTaskRuns(taskId)
        if (!disposed && current === generation) { setRuns(items); setError('') }
      } catch (err) { if (!disposed && current === generation) setError(String(err)) }
    }
    void reload()
    const listener = api.onScheduledTasksChanged(event => { if (event.taskId === taskId) void reload() })
    return () => { disposed = true; void listener.then(unlisten => unlisten()) }
  }, [taskId])
  const pages = Math.max(1, Math.ceil((runs?.length ?? 0) / 8))
  const currentPage = Math.min(page, pages - 1)
  const labels: Record<ScheduledTaskRun['status'], string> = {
    queued: t.chatSchedulesQueued, running: t.chatSchedulesRunning, succeeded: t.chatSchedulesSucceeded,
    failed: t.chatSchedulesFailed, skipped: t.chatSchedulesSkipped, interrupted: t.chatSchedulesInterrupted,
  }
  const statusDot: Record<ScheduledTaskRun['status'], string> = {
    queued: 'bg-amber-500', running: 'bg-blue-500', succeeded: 'bg-emerald-500',
    failed: 'bg-red-500', skipped: 'bg-neutral-400', interrupted: 'bg-neutral-400',
  }
  const remove = async (run: ScheduledTaskRun) => {
    if (!await confirmDialog({ message: t.chatSchedulesRunDeleteConfirm, confirmLabel: t.dialogDelete, danger: true })) return
    setDeleting(run.id)
    try {
      await api.scheduledTaskRunDelete(taskId, run.id)
      setRuns(items => items?.filter(item => item.id !== run.id) ?? null)
    } catch (err) { setError(String(err)) }
    finally { setDeleting(null) }
  }
  return <div className="custom-scrollbar min-h-0 flex-1 overflow-y-auto px-5 py-4">
    {error && <p role="alert" className="mb-3 text-sm text-[var(--danger)]">{error}</p>}
    {!runs ? <p className="text-sm text-[var(--color-muted-foreground)]">{t.chatSchedulesLoading}</p>
      : runs.length === 0 ? <div className="py-12 text-center text-[var(--color-muted-foreground)]"><History className="mx-auto mb-3" size={24} /><p>{t.chatSchedulesHistoryEmpty}</p></div>
      : <><div className="flex flex-col gap-2">{runs.slice(currentPage * 8, (currentPage + 1) * 8).map(run => {
        const duration = run.startedAt !== null && run.finishedAt !== null ? Math.max(0, run.finishedAt - run.startedAt) : null
        return <article key={run.id} className="rounded-lg border border-[var(--color-border)] p-3">
          <div className="flex items-center justify-between gap-3"><span className="flex items-center gap-2 text-sm font-medium"><span aria-hidden className={`h-2 w-2 shrink-0 rounded-full ${statusDot[run.status]}`} />{labels[run.status]}</span>
            <span className="text-xs text-[var(--color-muted-foreground)]">{run.trigger === 'manual' ? t.chatSchedulesTriggerManual : t.chatSchedulesTriggerSchedule}</span></div>
          <p className="mt-2 text-xs tabular-nums text-[var(--color-muted-foreground)]" title={`${t.chatSchedulesScheduledAt}: ${formatScheduleTime(run.scheduledAt, lang)}`}>
            {formatScheduleTime(run.startedAt ?? run.createdAt, lang)}{duration !== null && <> · {duration}{t.chatSchedulesSeconds}</>}</p>
          {run.error && <p className="mt-2 break-words text-xs text-[var(--danger)]">{run.error}</p>}
          <div className="mt-2 flex items-center justify-between gap-2">
            {run.conversationId ? <Button size="sm" variant="ghost" onClick={() => onOpenConversation(run.conversationId!)}>{t.chatSchedulesOpenConversation}<ArrowUpRight size={13} /></Button> : <span />}
            {run.status !== 'queued' && run.status !== 'running' && <Button size="sm" variant="ghost" disabled={deleting !== null} onClick={() => { void remove(run) }} aria-label={`${t.dialogDelete} · ${formatScheduleTime(run.createdAt, lang)}`}><Trash2 size={13} />{t.dialogDelete}</Button>}
          </div>
        </article>
      })}</div>
        {pages > 1 && <nav className="mt-4 flex items-center justify-between gap-2" aria-label={t.chatSchedulesHistory}>
          <Button size="sm" disabled={currentPage === 0} onClick={() => setPage(currentPage - 1)}>{t.chatSchedulesPreviousPage}</Button>
          <span className="text-xs text-[var(--color-muted-foreground)]">{t.chatSchedulesPage.replace('{page}', String(currentPage + 1)).replace('{pages}', String(pages))}</span>
          <Button size="sm" disabled={currentPage + 1 >= pages} onClick={() => setPage(currentPage + 1)}>{t.chatSchedulesNextPage}</Button>
        </nav>}
      </>}
  </div>
}

export function ScheduledTaskEditor({ task, template, onClose, onSaved, initialTab = 'settings', onOpenConversation, registerLeaveGuard }: {
  task: ScheduledTask | null
  template?: { name: string; prompt: string; schedule: ScheduledTaskSchedule }
  onClose: () => void
  onSaved: () => void
  initialTab?: 'settings' | 'history'
  onOpenConversation?: (id: string) => void
  registerLeaveGuard?: RegisterTaskLeaveGuard
}) {
  const t = useT()
  const lang = useLang()
  const [name, setName] = useState(task?.name ?? template?.name ?? '')
  const [prompt, setPrompt] = useState(task?.prompt ?? template?.prompt ?? '')
  const [schedule, setSchedule] = useState<ScheduledTaskSchedule>(task?.schedule ?? template?.schedule ?? defaultSchedule('daily'))
  const [intervalUnit, setIntervalUnit] = useState(() => {
    const initial = task?.schedule ?? template?.schedule
    return initial?.kind === 'interval' && initial.minutes >= 60 && initial.minutes % 60 === 0 ? 'hours' : 'minutes'
  })
  const [targetKind, setTargetKind] = useState<ScheduledTaskTarget['kind']>(task ? 'conversation' : 'newConversation')
  const [newTarget, setNewTarget] = useState<Extract<ScheduledTaskTarget, { kind: 'newConversation' }>>({ kind: 'newConversation' })
  const [existingTarget, setExistingTarget] = useState<Extract<ScheduledTaskTarget, { kind: 'conversation' }>>(
    { kind: 'conversation', conversationId: task?.conversationId ?? '' },
  )
  const [enabled, setEnabled] = useState(task?.enabled ?? true)
  const [projects, setProjects] = useState<ChatProject[]>([])
  const [query, setQuery] = useState('')
  const [results, setResults] = useState<{ query: string; items: ConversationSearchHit[] } | null>(null)
  const [selectedConversation, setSelectedConversation] = useState<ConversationChoice | null>(null)
  const [choosingConversation, setChoosingConversation] = useState(false)
  const [searchError, setSearchError] = useState('')
  const [preview, setPreview] = useState<{ schedule: ScheduledTaskSchedule; times: number[]; error: string } | null>(null)
  const [error, setError] = useState('')
  const [saving, setSaving] = useState(false)
  const [tab, setTab] = useState(initialTab)
  const [submitted, setSubmitted] = useState(false)
  const fieldId = useId()
  const savedId = useRef(task?.id)
  const scheduleDrafts = useRef<Partial<Record<ScheduledTaskSchedule['kind'], ScheduledTaskSchedule>>>({})
  const target = targetKind === 'newConversation' ? newTarget : existingTarget
  const signature = JSON.stringify({ name, prompt, schedule, target, enabled })
  const baseline = useRef(signature)
  const confirmation = useRef<Promise<boolean> | null>(null)
  const allowLeave = async () => {
    if (saving) return false
    if (signature === baseline.current) return true
    if (!confirmation.current) confirmation.current = confirmDialog({ message: t.chatSchedulesDiscardConfirm, confirmLabel: t.chatSchedulesDiscard, danger: true })
    try {
      const approved = await confirmation.current
      if (approved) baseline.current = signature
      return approved
    } finally { confirmation.current = null }
  }
  const leaveRef = useRef(allowLeave)
  leaveRef.current = allowLeave
  useEffect(() => {
    registerLeaveGuard?.(() => leaveRef.current())
    return () => registerLeaveGuard?.(null)
  }, [registerLeaveGuard])
  const requestClose = async () => { if (await allowLeave()) onClose() }
  const openConversation = async (id: string) => { if (await allowLeave()) { onClose(); onOpenConversation?.(id) } }
  const switchSchedule = (kind: ScheduledTaskSchedule['kind']) => {
    if (kind === schedule.kind) return
    scheduleDrafts.current[schedule.kind] = schedule
    const next = scheduleDrafts.current[kind] ?? defaultSchedule(kind)
    setSchedule(next)
    if (next.kind === 'interval') setIntervalUnit(next.minutes >= 60 && next.minutes % 60 === 0 ? 'hours' : 'minutes')
  }
  const offset = -new Date().getTimezoneOffset()
  const timezone = `GMT${offset < 0 ? '-' : '+'}${Math.floor(Math.abs(offset) / 60)}${Math.abs(offset) % 60 ? `:${String(Math.abs(offset) % 60).padStart(2, '0')}` : ''}`
  const conversationId = existingTarget.conversationId
  const showConversationSearch = targetKind === 'conversation' && (!conversationId || choosingConversation)

  useEffect(() => {
    let cancelled = false
    void chatApi.getProjects().then(items => { if (!cancelled) setProjects(items) })
      .catch(err => { if (!cancelled) setError(String(err)) })
    return () => { cancelled = true }
  }, [])

  useEffect(() => {
    if (!conversationId) return
    let cancelled = false
    void chatApi.getConversation(conversationId)
      .then(conversation => { if (!cancelled) setSelectedConversation(conversation) })
      .catch(() => { if (!cancelled) setSelectedConversation({ id: conversationId, title: t.chatSchedulesDeletedConversation, updated_at: 0 }) })
    return () => { cancelled = true }
  }, [conversationId, t.chatSchedulesDeletedConversation])

  useEffect(() => {
    let cancelled = false
    const timer = window.setTimeout(() => {
      void api.scheduledTaskPreview(schedule)
        .then(times => { if (!cancelled) setPreview({ schedule, times, error: '' }) })
        .catch(err => { if (!cancelled) setPreview({ schedule, times: [], error: String(err) }) })
    }, 300)
    return () => { cancelled = true; window.clearTimeout(timer) }
  }, [schedule])

  useEffect(() => {
    if (!showConversationSearch) return
    let cancelled = false
    setResults(null)
    setSearchError('')
    const timer = window.setTimeout(() => {
      const search = query.trim()
        ? chatApi.searchConversations(query, 20)
        : chatApi.getConversations(0, 20)
      void search.then(items => { if (!cancelled) setResults({ query, items }) })
        .catch(err => { if (!cancelled) setSearchError(String(err)) })
    }, 300)
    return () => { cancelled = true; window.clearTimeout(timer) }
  }, [query, showConversationSearch])

  const previewReady = preview?.schedule === schedule
  const canSave = !saving && previewReady && !preview?.error
  const nameError = submitted && !name.trim() ? t.chatSchedulesNameRequired : ''
  const promptError = submitted && !prompt.trim() ? t.chatSchedulesPromptRequired : ''
  const targetError = submitted && target.kind === 'conversation' && !target.conversationId ? t.chatSchedulesConversationRequired : ''
  const save = async (run = false) => {
    if (!canSave) return
    setSubmitted(true)
    if (!name.trim() || !prompt.trim() || (target.kind === 'conversation' && !target.conversationId)) { setTab('settings'); return }
    setSaving(true)
    setError('')
    try {
      const saved = await api.scheduledTaskSave({ id: savedId.current, name, prompt, schedule, target, enabled })
      savedId.current = saved.id
      const boundTarget = { kind: 'conversation' as const, conversationId: saved.conversationId }
      setExistingTarget(boundTarget)
      setTargetKind('conversation')
      setChoosingConversation(false)
      baseline.current = JSON.stringify({ name, prompt, schedule, target: boundTarget, enabled })
      if (run) await api.scheduledTaskRunNow(saved.id)
      onSaved()
    } catch (err) { setError(String(err)) }
    finally { setSaving(false) }
  }
  const weekdayNames = weekdayLabels(t)
  const scheduleKinds: { kind: ScheduledTaskSchedule['kind']; label: string }[] = [
    { kind: 'once', label: t.chatSchedulesOnce }, { kind: 'daily', label: t.chatSchedulesDaily },
    { kind: 'weekly', label: t.chatSchedulesWeekly }, { kind: 'monthly', label: t.chatSchedulesMonthly },
    { kind: 'yearly', label: t.chatSchedulesYearly }, { kind: 'interval', label: t.chatSchedulesInterval },
    { kind: 'cron', label: t.chatSchedulesCron },
  ]
  const conversationLocation = (conversation: ConversationChoice) => {
    const projectId = conversation.project_id ?? conversation.projectId
    return projects.find(project => project.id === projectId)?.name ?? conversation.folder ?? t.chatSchedulesNoProject
  }
  const selected = selectedConversation?.id === conversationId ? selectedConversation : null
  const searchReady = results?.query === query

  return <ScheduleDialog title={task ? t.chatSchedulesEdit : t.chatSchedulesNew} onClose={() => { void requestClose() }} width={640}>
    {task && <div className="kv-seg mx-5 mb-2 mt-3 w-fit shrink-0" role="tablist" aria-label={t.chatSchedulesEdit}>
      {(['settings', 'history'] as const).map(value => <button key={value} type="button" role="tab" id={`${fieldId}-${value}`} aria-selected={tab === value} aria-controls={`${fieldId}-panel`} className={tab === value ? 'active' : ''} disabled={saving} onClick={() => setTab(value)}>
        {value === 'settings' ? t.chatSchedulesSettings : t.chatSchedulesHistory}
      </button>)}
    </div>}
    {tab === 'history' && task ? <div id={`${fieldId}-panel`} role="tabpanel" aria-labelledby={`${fieldId}-history`} className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <TaskHistory taskId={task.id} onOpenConversation={id => { void openConversation(id) }} />
    </div> : <form id={`${fieldId}-panel`} role={task ? 'tabpanel' : undefined} aria-labelledby={task ? `${fieldId}-settings` : undefined} className="flex min-h-0 flex-1 flex-col overflow-hidden" onSubmit={event => { event.preventDefault(); void save() }}>
      <div className="custom-scrollbar min-h-0 flex-1 overflow-y-auto px-5 py-3">
        <fieldset disabled={saving} className="m-0 min-w-0 border-0 p-0">
        <FieldBlock label={t.chatSchedulesName} htmlFor={`${fieldId}-name`}><Input id={`${fieldId}-name`} autoFocus value={name} onChange={setName} aria-invalid={Boolean(nameError)} aria-describedby={nameError ? `${fieldId}-name-error` : undefined} />
          {nameError && <p id={`${fieldId}-name-error`} className="mt-2 text-xs text-[var(--danger)]" role="alert">{nameError}</p>}
        </FieldBlock>
        <FieldBlock label={t.chatSchedulesPrompt} htmlFor={`${fieldId}-prompt`}><TextArea id={`${fieldId}-prompt`} value={prompt} onChange={setPrompt} rows={5} placeholder={t.chatSchedulesPrompt}
          aria-invalid={Boolean(promptError)} aria-describedby={promptError ? `${fieldId}-prompt-error` : undefined}
          onKeyDown={event => { if ((event.metaKey || event.ctrlKey) && event.key === 'Enter' && !event.nativeEvent.isComposing) { event.preventDefault(); void save() } }} />
          {promptError && <p id={`${fieldId}-prompt-error`} className="mt-2 text-xs text-[var(--danger)]" role="alert">{promptError}</p>}
        </FieldBlock>
        <FieldBlock label={t.chatSchedulesSchedule}>
          <div className="kv-seg flex-wrap" role="group" aria-label={t.chatSchedulesSchedule}>
            {scheduleKinds.map(({ kind, label }) => <button key={kind} type="button" aria-pressed={schedule.kind === kind}
              onClick={() => switchSchedule(kind)}
              className={schedule.kind === kind ? 'active' : ''}>{label}</button>)}
          </div>
          {schedule.kind === 'once' && <FieldBlock label={t.chatSchedulesDateTime}>
            <Input type="datetime-local" value={localDateTime(schedule.at)} aria-label={t.chatSchedulesDateTime}
              onChange={value => setSchedule({ ...schedule, at: value ? Math.floor(new Date(value).getTime() / 1000) : 0 })} />
          </FieldBlock>}
          {schedule.kind === 'interval' && <FieldBlock label={t.chatSchedulesInterval}>
            <div className="grid grid-cols-2 gap-3">
              <Input type="number" min={intervalUnit === 'hours' ? 1 / 60 : 1} max={intervalUnit === 'hours' ? 8760 : 525600} step={intervalUnit === 'hours' ? 'any' : 1}
                value={String(intervalUnit === 'hours' ? schedule.minutes / 60 : schedule.minutes)}
                aria-label={intervalUnit === 'hours' ? t.chatSchedulesHours : t.chatSchedulesMinutes}
                onChange={value => setSchedule({ ...schedule, minutes: Number(value) * (intervalUnit === 'hours' ? 60 : 1) })} />
              <Select value={intervalUnit} onChange={setIntervalUnit} ariaLabel={t.chatSchedulesInterval}
                options={[{ value: 'minutes', label: t.chatSchedulesMinutes }, { value: 'hours', label: t.chatSchedulesHours }]} />
            </div>
          </FieldBlock>}
          {(schedule.kind === 'daily' || schedule.kind === 'weekly' || schedule.kind === 'monthly' || schedule.kind === 'yearly') && <FieldBlock label={t.chatSchedulesTime}>
            <div className="max-w-[180px]"><Input type="time" value={clockTime(schedule.hour, schedule.minute)} aria-label={t.chatSchedulesTime}
              onChange={value => { const [hour, minute] = value.split(':').map(Number); setSchedule({ ...schedule, hour, minute }) }} /></div>
          </FieldBlock>}
          {schedule.kind === 'weekly' && <FieldBlock label={t.chatSchedulesWeekdays}>
            <div className="flex flex-wrap gap-1.5" role="group" aria-label={t.chatSchedulesWeekdays}>
              {[1, 2, 3, 4, 5, 6, 0].map(day => <button key={day} type="button" aria-label={weekdayNames[day]} aria-pressed={schedule.weekdays.includes(day)}
                onClick={() => setSchedule({ ...schedule, weekdays: schedule.weekdays.includes(day)
                  ? schedule.weekdays.filter(value => value !== day) : [...schedule.weekdays, day] })}
                className={`rounded-full border px-3 py-1.5 text-xs transition-colors focus-visible:outline-2 focus-visible:outline-[var(--color-ring)] ${schedule.weekdays.includes(day)
                  ? 'border-[var(--color-primary)] bg-[var(--color-primary)] text-[var(--color-primary-foreground)]'
                  : 'border-[var(--color-border)] text-[var(--color-muted-foreground)] hover:bg-[var(--color-muted)]'}`}>{lang === 'zh' ? weekdayNames[day].replace('周', '') : weekdayNames[day].slice(0, 3)}</button>)}
            </div>
          </FieldBlock>}
          {schedule.kind === 'monthly' && <FieldBlock label={t.chatSchedulesMonthDays}>
            <div className="flex flex-wrap gap-1.5" role="group" aria-label={t.chatSchedulesMonthDays}>
              {Array.from({ length: 31 }, (_, index) => index + 1).map(day => <Button key={day} size="sm" variant={schedule.days.includes(day) ? 'primary' : 'default'} aria-pressed={schedule.days.includes(day)}
                onClick={() => setSchedule({ ...schedule, days: schedule.days.includes(day) ? schedule.days.filter(value => value !== day) : [...schedule.days, day] })}>{day}</Button>)}
            </div>
          </FieldBlock>}
          {schedule.kind === 'yearly' && <div className="grid grid-cols-2 gap-3">
            <FieldBlock label={t.chatSchedulesMonth}><Select value={String(schedule.month)} ariaLabel={t.chatSchedulesMonth} onChange={value => setSchedule({ ...schedule, month: Number(value) })}
              options={Array.from({ length: 12 }, (_, index) => ({ value: String(index + 1), label: t.chatSchedulesMonthOption.replace('{month}', String(index + 1)) }))} /></FieldBlock>
            <FieldBlock label={t.chatSchedulesDay}><Select value={String(schedule.day)} ariaLabel={t.chatSchedulesDay} onChange={value => setSchedule({ ...schedule, day: Number(value) })}
              options={Array.from({ length: 31 }, (_, index) => ({ value: String(index + 1), label: t.chatSchedulesDayOption.replace('{day}', String(index + 1)) }))} /></FieldBlock>
          </div>}
          {schedule.kind === 'cron' && <FieldBlock label={t.chatSchedulesCron}>
            <Input mono value={schedule.expr} onChange={expr => setSchedule({ ...schedule, expr })} aria-label={t.chatSchedulesCron} />
            <p className="mt-2 font-mono text-xs text-[var(--color-muted-foreground)]">{t.chatSchedulesCronHint}</p>
          </FieldBlock>}
          {schedule.kind !== 'interval' && <p className="mt-2 text-xs text-[var(--color-muted-foreground)]">{t.chatSchedulesLocalTimezone} {timezone}</p>}
          <div className="mt-2 flex items-start gap-2 text-xs text-[var(--color-muted-foreground)]" aria-live="polite" aria-busy={!previewReady}>
            <CalendarClock size={14} strokeWidth={1.75} className="mt-0.5 shrink-0" />
            {!previewReady ? <p>{t.chatSchedulesPreviewPending}</p>
              : preview.error ? <p role="alert" className="break-words whitespace-pre-wrap text-[var(--danger)]">{preview.error}</p>
              : preview.times.length === 0 ? <p>{t.chatSchedulesPreviewEmpty}</p>
              : <p>{t.chatSchedulesPreview}{lang === 'zh' ? '：' : ': '}{preview.times.slice(0, 3)
                .map(time => friendlyScheduleTime(time, t, lang)).join(lang === 'zh' ? '、' : ', ')}{preview.times.length > 3 && '…'}</p>}
          </div>
        </FieldBlock>
        <FieldBlock label={t.chatSchedulesTarget}>
          {!task && <div className="grid grid-cols-2 gap-3" role="group" aria-label={t.chatSchedulesTarget}>
            {[
              { kind: 'newConversation' as const, label: t.chatSchedulesNewConversation, hint: t.chatSchedulesNewConversationHint, Icon: Plus },
              { kind: 'conversation' as const, label: t.chatSchedulesExistingConversation, hint: t.chatSchedulesExistingConversationHint, Icon: MessageSquare },
            ].map(({ kind, label, hint, Icon }) => <button key={kind} type="button" aria-pressed={targetKind === kind} onClick={() => setTargetKind(kind)}
              className={`flex min-w-0 flex-col gap-2 rounded-lg border p-3 text-left transition-colors focus-visible:outline-2 focus-visible:outline-[var(--color-ring)] ${targetKind === kind
                ? 'border-[var(--color-primary)] bg-[var(--color-muted)]' : 'border-[var(--color-border)] hover:bg-[var(--color-muted)]'}`}>
              <span className="flex items-center gap-2 text-xs font-medium"><Icon size={15} strokeWidth={1.75} className="shrink-0" /><span className="flex-1">{label}</span>{targetKind === kind && <Check size={14} strokeWidth={1.75} className="shrink-0 text-[var(--color-primary)]" />}</span>
              <span className="text-xs leading-5 text-[var(--color-muted-foreground)]">{hint}</span>
            </button>)}
          </div>}
          <p className="mt-2 text-xs leading-5 text-[var(--color-muted-foreground)]">{t.chatSchedulesApprovalNote}</p>
        </FieldBlock>
        {targetKind === 'newConversation' ? <>
          <FieldBlock label={t.chatSchedulesModel}>
            <div className="flex flex-wrap items-center gap-2">
              <Button size="sm" variant={newTarget.model ? 'ghost' : 'primary'} aria-pressed={!newTarget.model}
                onClick={() => setNewTarget({ ...newTarget, providerId: null, model: null })}>{t.chatSchedulesDefaultModel}</Button>
              <ModelSelector currentProviderId={newTarget.providerId ?? ''} currentModel={newTarget.model ?? ''}
                onModelChange={(providerId, model) => setNewTarget({ ...newTarget, providerId, model })} />
            </div>
          </FieldBlock>
          <FieldBlock label={t.chatSchedulesThinkingLevel}><Select value={newTarget.thinkingLevel ?? ''} ariaLabel={t.chatSchedulesThinkingLevel}
            onChange={value => setNewTarget({ ...newTarget, thinkingLevel: value ? value as 'off' | 'low' | 'medium' | 'high' : null })}
            options={[{ value: '', label: t.chatSchedulesFollowGlobal }, { value: 'off', label: t.chatSchedulesThinkingOff }, { value: 'low', label: t.chatSchedulesThinkingLow }, { value: 'medium', label: t.chatSchedulesThinkingMedium }, { value: 'high', label: t.chatSchedulesThinkingHigh }]} /></FieldBlock>
          <FieldBlock label={t.chatSchedulesProject}>
            <Select value={newTarget.projectId ?? ''} ariaLabel={t.chatSchedulesProject}
              onChange={projectId => setNewTarget({ ...newTarget, projectId: projectId || null })}
              options={[{ value: '', label: t.chatSchedulesNoProject }, ...projects.map(project => ({ value: project.id, label: project.name }))]} />
          </FieldBlock>
        </> : <FieldBlock label={t.chatSchedulesChooseConversation}>
          {conversationId && <div className="mb-3 flex items-center gap-3 rounded-lg border border-[var(--color-border)] bg-[var(--color-muted)] p-3">
            <MessageSquare size={16} strokeWidth={1.75} className="shrink-0 text-[var(--color-muted-foreground)]" />
            <div className="min-w-0 flex-1"><p className="truncate text-sm font-medium">{selected?.title || conversationId}</p>
              {selected && <p className="truncate text-xs text-[var(--color-muted-foreground)]">{conversationLocation(selected)}</p>}
            </div>
            {onOpenConversation && <Button size="sm" variant="ghost" onClick={() => { void openConversation(conversationId) }}>{t.chatSchedulesOpenConversation}</Button>}
            <Button size="sm" variant="ghost" onClick={() => setChoosingConversation(true)}>{t.chatSchedulesChangeConversation}</Button>
          </div>}
          {showConversationSearch && <>
            <div className="flex items-center gap-2"><Search size={16} strokeWidth={1.75} className="shrink-0 text-[var(--color-muted-foreground)]" />
              <Input value={query} onChange={setQuery} placeholder={t.chatSchedulesSearchConversation} aria-label={t.chatSchedulesSearchConversation}
                aria-invalid={Boolean(targetError)} aria-describedby={targetError ? `${fieldId}-target-error` : undefined} />
            </div>
            <div className="custom-scrollbar mt-2 max-h-[220px] overflow-y-auto" aria-busy={!searchReady && !searchError}>
              {searchReady && results.items.map(conversation => <button key={conversation.id} type="button" aria-pressed={conversationId === conversation.id}
                className="flex w-full items-center gap-3 rounded-lg px-3 py-2.5 text-left hover:bg-[var(--color-muted)] focus-visible:outline-2 focus-visible:outline-[var(--color-ring)]"
                onClick={() => { setSelectedConversation(conversation); setExistingTarget({ kind: 'conversation', conversationId: conversation.id }); setChoosingConversation(false) }}>
                <MessageSquare size={15} strokeWidth={1.75} className="shrink-0 text-[var(--color-muted-foreground)]" />
                <span className="min-w-0 flex-1"><span className="block truncate text-sm">{conversation.title}</span>
                  <span className="block truncate text-xs text-[var(--color-muted-foreground)]">{conversationLocation(conversation)} · {friendlyScheduleTime(conversation.updated_at, t, lang)}</span>
                </span>
                {conversationId === conversation.id && <Check size={15} strokeWidth={1.75} className="shrink-0 text-[var(--color-primary)]" />}
              </button>)}
              {!searchError && <>{!searchReady ? <p className="py-3 text-xs text-[var(--color-muted-foreground)]">{t.chatSchedulesLoading}</p>
                : results.items.length === 0 && <p className="py-3 text-xs text-[var(--color-muted-foreground)]">{t.chatSchedulesNoConversations}</p>}</>}
            </div>
            {searchError && <p className="mt-2 break-words whitespace-pre-wrap text-xs text-[var(--danger)]" role="alert">{searchError}</p>}
          </>}
          {targetError && <p id={`${fieldId}-target-error`} className="mt-2 text-xs text-[var(--danger)]" role="alert">{targetError}</p>}
        </FieldBlock>}
        </fieldset>
      </div>
      <footer className="shrink-0 border-t border-[var(--color-border)] bg-[var(--color-background)] px-5 py-4">
        {error && <p className="mb-3 break-words whitespace-pre-wrap text-sm text-[var(--danger)]" role="alert">{error}</p>}
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex items-center gap-2 text-xs"><Toggle checked={enabled} onChange={setEnabled} disabled={saving} ariaLabel={t.chatSchedulesEnabled} /><span>{t.chatSchedulesEnabled}</span></div>
          <div className="flex items-center gap-2"><Button onClick={() => { void requestClose() }} disabled={saving}>{t.cancel}</Button>
            <Button disabled={!canSave} onClick={() => { void save(true) }}>{t.chatSchedulesSaveAndRun}</Button>
            <Button type="submit" variant="primary" disabled={!canSave}>{t.save}</Button></div>
        </div>
      </footer>
    </form>}
  </ScheduleDialog>
}
