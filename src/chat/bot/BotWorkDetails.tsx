import { useEffect, useState } from 'react'
import { api, type ArtifactLibraryItem } from '../../api/tauri'
import type { ScheduledTaskRun } from '../../api/scheduledTaskContracts'
import { Button } from '../../components/Button'
import { confirmDialog } from '../../components/dialogQueue'
import { useLang, useT, type I18n } from '../../components/i18n'
import { scheduleSummary } from '../scheduledTasks/scheduleSummary'
import { useScheduledTasks } from '../scheduledTasks/useScheduledTasks'
import { artifactsForConversation } from './artifactScope'

function runLabel(status: ScheduledTaskRun['status'], t: I18n) {
  switch (status) {
    case 'queued': return t.chatSchedulesQueued
    case 'running': return t.chatSchedulesRunning
    case 'succeeded': return t.chatSchedulesSucceeded
    case 'failed': return t.chatSchedulesFailed
    case 'skipped': return t.chatSchedulesSkipped
    case 'interrupted': return t.chatSchedulesInterrupted
  }
}

export function BotWorkDetails({ conversationId, onOpenArtifacts, section = 'tasks' }: {
  conversationId: string
  section?: 'tasks' | 'artifacts'
  onOpenArtifacts: (conversationId: string) => void
}) {
  const t = useT()
  const lang = useLang()
  const { tasks, error: tasksError, liveRuns } = useScheduledTasks()
  const [artifacts, setArtifacts] = useState<ArtifactLibraryItem[] | null>(null)
  const [artifactError, setArtifactError] = useState('')
  const [actionError, setActionError] = useState('')
  const [busyId, setBusyId] = useState<string | null>(null)
  const [runsFor, setRunsFor] = useState<string | null>(null)
  const [runs, setRuns] = useState<ScheduledTaskRun[]>([])
  const mine = (tasks ?? []).filter((task) => task.conversationId === conversationId)
  useEffect(() => {
    if (section !== 'artifacts') return
    setArtifacts(null)
    let current = true
    setArtifactError('')
    const apply = (page: { items: ArtifactLibraryItem[] }) => {
      if (current) setArtifacts(artifactsForConversation(page.items, conversationId).slice(0, 5))
    }
    void api.chatArtifactsList(false).then((page) => {
      apply(page)
      return current ? api.chatArtifactsList(true) : undefined
    }).then((page) => {
      if (page) apply(page)
    }).catch((err) => {
      if (current) setArtifactError(err instanceof Error ? err.message : String(err))
    })
    return () => { current = false }
  }, [conversationId, section])
  const act = async (id: string, action: () => Promise<unknown>) => {
    if (busyId) return
    setBusyId(id)
    setActionError('')
    try {
      await action()
    } catch (err) {
      setActionError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusyId(null)
    }
  }
  const visibleError = section === 'tasks' ? actionError || tasksError : artifactError
  return (
    <div className="flex w-full flex-col gap-6 text-left">
      {visibleError && <p className="text-[12px] text-red-600" role="alert">{visibleError}</p>}
      {section === 'tasks' && <section aria-label={t.chatBotSchedules}>
        <h3 className="text-[13px] font-semibold text-neutral-900">{t.chatBotSchedules}</h3>
        {tasks === null && <p className="mt-2 text-[12px] text-neutral-500" aria-busy="true">{t.chatLoading}</p>}
        {tasks !== null && mine.length === 0 && <p className="mt-2 text-[12px] leading-5 text-neutral-500">{t.chatBotNoSchedules}</p>}
        <ul className="mt-2 flex flex-col divide-y divide-neutral-200/60">
          {mine.map((task) => {
            const live = liveRuns.get(task.id)
            return (
              <li key={task.id} className="py-3">
                <details>
                  <summary className="cursor-pointer text-[13px] text-neutral-900">
                    <span className="font-medium">{task.name}</span>
                    <span className="ml-2 text-[12px] text-neutral-500">{task.enabled ? (live ? runLabel(live.status, t) : t.chatSchedulesActive) : t.chatSchedulesDisabled}</span>
                    <span className="mt-1 block text-[12px] text-neutral-500">{scheduleSummary(task.schedule, t, lang)}</span>
                  </summary>
                  <div className="mt-2 flex flex-wrap gap-2">
                    <Button size="sm" disabled={busyId !== null} onClick={() => void act(task.id, () => api.scheduledTaskSetEnabled(task.id, !task.enabled))}>
                      {task.enabled ? t.chatBotSchedulePause : t.chatSchedulesEnabled}
                    </Button>
                    <Button size="sm" disabled={busyId !== null} onClick={() => {
                      void confirmDialog({ message: t.chatBotRunNowConfirm, confirmLabel: t.chatSchedulesRunNow }).then((yes) => {
                        if (yes) void act(task.id, () => api.scheduledTaskRunNow(task.id))
                      })
                    }}>{t.chatSchedulesRunNow}</Button>
                    <Button size="sm" disabled={busyId !== null} onClick={() => {
                      if (runsFor === task.id) { setRunsFor(null); return }
                      void act(task.id, async () => {
                        setRuns(await api.scheduledTaskRuns(task.id))
                        setRunsFor(task.id)
                      })
                    }}>{t.chatSchedulesHistory}</Button>
                  </div>
                  {runsFor === task.id && (
                    <ul className="mt-2 flex flex-col gap-1">
                      {runs.length === 0 && <li className="text-[12px] text-neutral-500">{t.chatSchedulesHistoryEmpty}</li>}
                      {runs.map((run) => (
                        <li key={run.id} className="text-[12px] text-neutral-700">
                          {runLabel(run.status, t)}
                          {run.trigger === 'manual' ? ` · ${t.chatSchedulesRunNow}` : ''}
                          {run.error ? ` · ${run.error}` : ''}
                        </li>
                      ))}
                    </ul>
                  )}
                </details>
                {(task.lastError || live?.error) && <p className="mt-1 text-[12px] text-red-600" role="alert">{task.lastError || live?.error}</p>}
              </li>
            )
          })}
        </ul>
      </section>}
      {section === 'artifacts' && <section aria-label={t.chatBotArtifacts}>
        <div className="flex items-center justify-between gap-2">
          <h3 className="text-[13px] font-semibold text-neutral-900">{t.chatBotArtifacts}</h3>
          {!!artifacts?.length && <Button size="sm" variant="ghost" onClick={() => onOpenArtifacts(conversationId)}>{t.chatBotOpenArtifacts}</Button>}
        </div>
        {artifacts === null && !artifactError && <p className="mt-2 text-[12px] text-neutral-500" aria-busy="true">{t.chatLoading}</p>}
        {artifacts && artifacts.length === 0 && <p className="mt-2 text-[12px] leading-5 text-neutral-500">{t.chatBotNoArtifacts}</p>}
        <ul className="mt-2 flex flex-col gap-1">
          {artifacts?.map((item) => <li key={item.id} className="truncate text-[13px] text-neutral-800">{item.title || item.artifact.name}</li>)}
        </ul>
      </section>}
    </div>
  )
}
