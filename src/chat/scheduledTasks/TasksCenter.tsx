import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react'
import { MessageSquarePlus, Plus, Upload } from 'lucide-react'
import { api } from '../../api/tauri'
import type { AutomationMeta } from '../../api/automationContracts'
import { Button } from '../../components/Button'
import { useT } from '../../components/i18n'
import { AutomationCenter } from '../automation/AutomationCenter'
import { automationApi } from '../automation/api'
import { setHash } from '../chatRoutes'
import { useTauriEvent } from '../hooks/useTauriEvent'
import { ScheduledTasksCenter } from './ScheduledTasksCenter'
import type { RegisterTaskLeaveGuard } from './ScheduledTaskEditor'
import { useScheduledTasks } from './useScheduledTasks'
import '../market/market.css'

export function TasksCenter({ tab, onOpenConversation, onCreateByChat, registerLeaveGuard }: {
  tab: 'schedules' | 'automations'
  onOpenConversation: (id: string) => void
  onCreateByChat: (prompt: string) => void
  registerLeaveGuard?: RegisterTaskLeaveGuard
}) {
  const t = useT()
  const { tasks } = useScheduledTasks()
  const [automations, setAutomations] = useState<AutomationMeta[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')
  const loadGeneration = useRef(0)
  const reloadAutomations = useCallback(async () => {
    const generation = ++loadGeneration.current
    try {
      const items = await automationApi.list()
      if (generation !== loadGeneration.current) return
      setAutomations(items)
      setError('')
    } catch (err) {
      if (generation === loadGeneration.current) setError(String(err))
    } finally {
      if (generation === loadGeneration.current) setLoading(false)
    }
  }, [])
  useEffect(() => {
    void reloadAutomations()
    return () => { loadGeneration.current += 1 }
  }, [reloadAutomations])
  useTauriEvent(api.onAutomationChanged, () => { void reloadAutomations() }, [])

  const shell = (body: ReactNode, actions: ReactNode) => <section className="kv assistant-center-root flex h-full min-h-0 flex-col">
    <header className="shrink-0 px-6 pb-5 pt-5">
      <div className="mx-auto w-full max-w-[832px]">
        <div className="flex items-center justify-between gap-4">
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-3" data-tauri-drag-region="false">
              <h1 className="m-0 text-[26px] font-bold text-[var(--text)]">{t.chatNavTasks}</h1>
              <div className="kv-plugin-segments" role="tablist" aria-label={t.chatNavTasks}>
                {(['schedules', 'automations'] as const).map(value => (
                  <button key={value} id={`tasks-tab-${value}`} type="button" role="tab"
                    className="kv-plugin-segment" aria-current={tab === value ? 'page' : undefined}
                    aria-selected={tab === value} aria-controls="tasks-panel"
                    tabIndex={tab === value ? 0 : -1} onClick={() => setHash(`#chat/${value}`)}
                    onKeyDown={event => {
                      if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return
                      event.preventDefault()
                      const next = event.key === 'Home' ? 'schedules' : event.key === 'End' ? 'automations' : tab === 'schedules' ? 'automations' : 'schedules'
                      setHash(`#chat/${next}`)
                      document.getElementById(`tasks-tab-${next}`)?.focus()
                    }}>
                    <span className="flex items-center gap-2">
                      {value === 'schedules' ? t.chatNavSchedules : t.chatNavAutomations}
                      <span className="text-[11px] opacity-60">· {value === 'schedules' ? tasks?.length ?? '—' : loading ? '—' : automations.length}</span>
                    </span>
                  </button>
                ))}
              </div>
            </div>
            <p className="mt-1 text-[13px] text-neutral-500 dark:text-neutral-400">{t.chatTasksSubtitle}</p>
          </div>
          <div className="flex shrink-0 items-center gap-2">{actions}</div>
        </div>
        <p className="mt-3 text-[12px] leading-relaxed text-neutral-500 dark:text-neutral-400">{tab === 'schedules' ? t.chatTasksSchedulesHint : t.chatTasksAutomationsHint}</p>
      </div>
    </header>
    <div id="tasks-panel" role="tabpanel" aria-labelledby={`tasks-tab-${tab}`} className="flex min-h-0 flex-1 flex-col">{body}</div>
  </section>

  const createByChatPrompt = () => onCreateByChat(tab === 'schedules' ? t.chatSchedulesChatPrompt : t.chatAutomationChatPrompt)
  const createByChat = <Button size="sm" variant="primary" onClick={createByChatPrompt}>
    <MessageSquarePlus size={14} strokeWidth={1.75} />{t.chatTasksCreateByChat}
  </Button>

  return tab === 'automations'
    ? <AutomationCenter items={automations} loading={loading} listError={error} onReload={reloadAutomations} onCreateByChat={createByChatPrompt}
      renderList={(body, actions) => shell(body, <>
        <Button size="sm" variant="ghost" onClick={actions.onImport}><Upload size={14} strokeWidth={1.75} />{t.chatAutomationImport}</Button>
        <Button size="sm" variant="ghost" onClick={actions.onCreate}><Plus size={14} strokeWidth={1.75} />{t.chatTasksCreateManually}</Button>
        {createByChat}
      </>)} />
    : <ScheduledTasksCenter onOpenConversation={onOpenConversation} onCreateByChat={createByChatPrompt} registerLeaveGuard={registerLeaveGuard}
      renderList={(body, onCreate) => shell(body, <>
        <Button size="sm" variant="ghost" onClick={onCreate}><Plus size={14} strokeWidth={1.75} />{t.chatTasksCreateManually}</Button>
        {createByChat}
      </>)} />
}
