import { useEffect, useRef, useState, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { AlertCircle, ArrowUpRight, CalendarCheck, Clock, History, MessageSquarePlus, MoreHorizontal, Play, Plus, SquarePen, Trash2 } from 'lucide-react'
import { api } from '../../api/tauri'
import type { ScheduleRule, ScheduledTask } from '../../api/scheduledTaskContracts'
import { Button, IconButton } from '../../components/Button'
import { confirmDialog } from '../../components/dialogQueue'
import { useLang, useT } from '../../components/i18n'
import { Toggle } from '../../settings/public/controls'
import { chatApi } from '../api'
import { usePopoverMenu } from '../usePopoverMenu'
import { useClampedMenuPosition } from '../useClampedMenuPosition'
import { ScheduledTaskEditor, type RegisterTaskLeaveGuard } from './ScheduledTaskEditor'
import { formatScheduleTime, friendlyScheduleTime, scheduleSummary } from './scheduleSummary'
import { useScheduledTasks } from './useScheduledTasks'


function TaskMenu({ anchor, busy, onAction, onClose }: {
  anchor: { left: number; top: number }
  busy: boolean
  onAction: (action: 'run' | 'edit' | 'history' | 'conversation' | 'delete') => void
  onClose: () => void
}) {
  const t = useT()
  const ref = useRef<HTMLDivElement>(null)
  usePopoverMenu(true, onClose, ref)
  const position = useClampedMenuPosition(ref, anchor)
  useEffect(() => {
    const closeOutside = (event: PointerEvent) => { if (!ref.current?.contains(event.target as Node)) onClose() }
    window.addEventListener('pointerdown', closeOutside)
    return () => window.removeEventListener('pointerdown', closeOutside)
  }, [onClose])
  const items = [
    { action: 'run' as const, label: t.chatSchedulesRunNow, Icon: Play },
    { action: 'edit' as const, label: t.chatSchedulesEdit, Icon: SquarePen },
    { action: 'history' as const, label: t.chatSchedulesHistory, Icon: History },
    { action: 'conversation' as const, label: t.chatSchedulesOpenConversation, Icon: ArrowUpRight },
    { action: 'delete' as const, label: t.dialogDelete, Icon: Trash2 },
  ]
  return createPortal(<div ref={ref} role="menu" className="kv-menu chat-motion-popover fixed z-[200] min-w-[176px]" style={position}>
    {items.map(({ action, label, Icon }) => <button key={action} type="button" role="menuitem" disabled={busy && (action === 'run' || action === 'delete')}
      className={`kv-menu-item ${action === 'delete' ? 'kv-menu-item--danger border-t border-[var(--theme-surface-border)]' : ''}`}
      onClick={() => { onClose(); onAction(action) }}><Icon strokeWidth={1.75} />{label}</button>)}
  </div>, document.body)
}

export function ScheduledTasksCenter({ onOpenConversation, onCreateByChat, renderList, registerLeaveGuard }: {
  onOpenConversation: (id: string) => void
  onCreateByChat: () => void
  renderList: (body: ReactNode, onCreate: () => void) => ReactNode
  registerLeaveGuard?: RegisterTaskLeaveGuard
}) {
  const t = useT()
  const lang = useLang()
  const { tasks, liveRuns, error: loadError } = useScheduledTasks()
  const [editor, setEditor] = useState<{ task: ScheduledTask | null; template?: { name: string; prompt: string; schedule: ScheduleRule }; initialTab?: 'settings' | 'history' } | null>(null)
  const [menu, setMenu] = useState<{ task: ScheduledTask; left: number; top: number } | null>(null)
  const [titles, setTitles] = useState<Record<string, string | null>>({})
  const [busy, setBusy] = useState<string | null>(null)
  const [error, setError] = useState('')
  useEffect(() => {
    if (!tasks) return
    let cancelled = false
    const ids = [...new Set(tasks.map(task => task.conversationId))]
    void Promise.all(ids.map(async id => {
      try { return [id, (await chatApi.getConversation(id)).title] as const }
      catch { return [id, null] as const }
    })).then(entries => { if (!cancelled) setTitles(Object.fromEntries(entries)) })
    return () => { cancelled = true }
  }, [tasks])

  const mutate = async (id: string, action: () => Promise<unknown>) => {
    setBusy(id)
    setError('')
    try { await action() }
    catch (err) { setError(String(err)) }
    finally { setBusy(null) }
  }
  const remove = async (task: ScheduledTask) => {
    if (!await confirmDialog({ message: t.chatSchedulesDeleteConfirm.replace('{name}', task.name), confirmLabel: t.dialogDelete, danger: true })) return
    await mutate(task.id, () => api.scheduledTaskDelete(task.id))
  }
  const templates: { name: string; prompt: string; schedule: ScheduleRule }[] = [
    { name: t.chatSchedulesTemplateNewsName, prompt: t.chatSchedulesTemplateNews, schedule: { kind: 'daily', hour: 9, minute: 0 } },
    { name: t.chatSchedulesTemplateWeekName, prompt: t.chatSchedulesTemplateWeek, schedule: { kind: 'weekly', weekdays: [1], hour: 9, minute: 0 } },
    { name: t.chatSchedulesTemplateReportName, prompt: t.chatSchedulesTemplateReport, schedule: { kind: 'weekly', weekdays: [1, 2, 3, 4, 5], hour: 18, minute: 0 } },
  ]
  const body = <>
    <div className="custom-scrollbar mx-auto flex min-h-0 w-full max-w-[880px] flex-1 flex-col overflow-y-auto px-6 pb-6">
      {(error || loadError) && <p className="mb-3 text-[13px] text-red-600 dark:text-red-400" role="alert">{error || loadError}</p>}
      {!tasks ? <p className="text-[13px] text-neutral-500">{t.chatSchedulesLoading}</p> : tasks.length === 0 ? <div className="flex flex-1 flex-col items-center justify-center rounded-xl border border-dashed border-[var(--theme-surface-border)] px-6 py-14 text-center dark:border-white/[0.1]">
        <span className="mb-4 flex h-12 w-12 items-center justify-center rounded-2xl bg-[var(--theme-surface-muted)] text-neutral-400"><Clock size={23} strokeWidth={1.5} /></span>
        <h2 className="text-[15px] font-medium text-neutral-800">{t.chatSchedulesEmpty}</h2>
        <p className="mt-2 max-w-[24rem] text-[13px] leading-relaxed text-neutral-500 dark:text-neutral-400">{t.chatSchedulesEmptyHint}</p>
        <div className="mt-5 flex items-center gap-2">
          <Button size="sm" variant="primary" onClick={onCreateByChat}><MessageSquarePlus size={14} strokeWidth={1.75} />{t.chatTasksCreateByChat}</Button>
          <Button size="sm" onClick={() => setEditor({ task: null })}><Plus size={14} strokeWidth={1.75} />{t.chatTasksCreateManually}</Button>
        </div>
        <div className="mt-8 flex max-w-[34rem] flex-wrap justify-center gap-2">{templates.map(template => <Button key={template.name} size="sm" onClick={() => setEditor({ task: null, template })}>{template.prompt}</Button>)}</div>
      </div> : <ul className="flex flex-col gap-2">{tasks.map(task => {
        const Icon = task.status === 'completed' ? CalendarCheck : Clock
        const live = liveRuns.get(task.id)
        const running = live?.status === 'running' || live?.status === 'queued' ? live.status : null
        const target = t.chatSchedulesSendTo.replace('{title}', titles[task.conversationId] === null ? t.chatSchedulesDeletedConversation : titles[task.conversationId] ?? t.chatSchedulesLoading)
        return <li key={task.id} className="flex items-center gap-3 rounded-xl border border-[var(--theme-surface-border)] bg-[var(--theme-surface)] px-4 py-3 dark:border-white/[0.08]">
          <button type="button" className="flex min-w-0 flex-1 items-center gap-3 text-left" onClick={() => setEditor({ task })}>
            <span className={`flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-[var(--theme-surface-muted)] dark:bg-white/[0.06] ${!task.enabled ? 'text-neutral-400 dark:text-neutral-500' : 'text-neutral-600'}`}><Icon size={16} strokeWidth={1.75} /></span>
            <span className="min-w-0"><span className={`block truncate text-[14px] font-medium ${!task.enabled ? 'text-neutral-500 dark:text-neutral-400' : 'text-neutral-900'}`}>{task.name}</span>
              <span className="mt-0.5 block truncate text-[12px] text-neutral-500 dark:text-neutral-400" title={`${scheduleSummary(task.schedule, t, lang)} · ${target}`}>{scheduleSummary(task.schedule, t, lang)} · {target}</span>
            </span>
          </button>
          <div className="flex shrink-0 flex-col items-end gap-1">
            <span className="text-[12px] tabular-nums text-neutral-500 dark:text-neutral-400" title={formatScheduleTime(task.nextRunAt, lang)}>{task.status === 'completed' ? t.chatSchedulesCompleted : !task.enabled ? t.chatSchedulesDisabled : friendlyScheduleTime(task.nextRunAt, t, lang)}</span>
            {(running || task.lastError) && <span className="flex items-center gap-1.5">
              {running && <span className="rounded-md bg-blue-500/10 px-1.5 py-0.5 text-[10px] text-blue-600 dark:text-blue-400">{running === 'queued' ? t.chatSchedulesQueued : t.chatSchedulesRunning}</span>}
              {task.lastError && <span title={task.lastError} className="flex items-center gap-1 rounded-md bg-red-500/10 px-1.5 py-0.5 text-[10px] text-red-600 dark:text-red-400"><AlertCircle size={11} />{t.chatSchedulesFailed}</span>}
            </span>}
          </div>
          <Toggle checked={task.enabled} disabled={busy !== null} ariaLabel={`${t.chatSchedulesEnabled} · ${task.name}`} onChange={enabled => { void mutate(task.id, () => api.scheduledTaskSetEnabled(task.id, enabled)) }} />
          <IconButton size="sm" variant="ghost" label={`${t.chatTasksMore} · ${task.name}`} onClick={event => { const rect = event.currentTarget.getBoundingClientRect(); setMenu({ task, left: rect.right - 176, top: rect.bottom + 4 }) }}><MoreHorizontal size={16} strokeWidth={1.75} /></IconButton>
        </li>
      })}</ul>}
    </div>
    {editor && <ScheduledTaskEditor task={editor.task} template={editor.template} initialTab={editor.initialTab} registerLeaveGuard={registerLeaveGuard}
      onOpenConversation={onOpenConversation} onClose={() => setEditor(null)} onSaved={() => setEditor(null)} />}
    {menu && <TaskMenu anchor={menu} busy={busy !== null} onClose={() => setMenu(null)} onAction={action => {
      const task = menu.task
      if (action === 'run') void mutate(task.id, () => api.scheduledTaskRunNow(task.id))
      else if (action === 'edit') setEditor({ task })
      else if (action === 'history') setEditor({ task, initialTab: 'history' })
      else if (action === 'delete') void remove(task)
      else onOpenConversation(task.conversationId)
    }} />}
  </>
  return renderList(body, () => setEditor({ task: null }))
}
