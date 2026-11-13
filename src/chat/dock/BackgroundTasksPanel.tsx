// Background tasks 面板：内置 run_command 后台作业 + 外部 CLI（claude）自报的后台任务。
// Running（可停止）/ Finished（可清空）两段，2.5s 轮询；inactive 时停轮询（同兄弟面板惯例）。
import { Square } from 'lucide-react'
import { api, type BackgroundTaskInfo } from '../../api/tauri'
import { i18n, type Lang } from '../../components/i18n'
import { notifyError } from '../../components/appNoticeStore'
import { formatTaskElapsed, partitionTasks } from '../backgroundTasks'
import { stopBackgroundTask, updateBackgroundTasks, useBackgroundTasks } from '../useBackgroundTasks'
import { SubagentTaskList } from '../AgentHub'
import { useSubagentRoster } from '../useSubagentRoster'
import { BackgroundTaskIcon } from './BackgroundTaskIcon'

function TaskGlyph({ kind }: { kind: string }) {
  return <BackgroundTaskIcon kind={kind} size={14} strokeWidth={1.8} className="mt-0.5 shrink-0 text-neutral-400" />
}

type BackgroundTasksPanelProps = {
  active: boolean
  lang: Lang
  /** 面板按对话隔离：只展示当前对话自己的任务。null = 还没有对话（新建未发送）。 */
  conversationId: string | null
}

export function BackgroundTasksPanel({ active, lang, conversationId }: BackgroundTasksPanelProps) {
  const t = i18n[lang]
  const tasks = useBackgroundTasks(conversationId, active)
  const { roster } = useSubagentRoster(active ? conversationId : null)

  const { running, finished } = partitionTasks(tasks)

  const stop = async (task: BackgroundTaskInfo) => {
    if (!conversationId) return
    try {
      await stopBackgroundTask(conversationId, task)
    } catch (error) {
      notifyError(error instanceof Error ? error.message : String(error))
    }
  }

  const clearFinished = async () => {
    if (!conversationId) return
    try {
      await api.chatClearFinishedBackgroundTasks(conversationId)
      updateBackgroundTasks(conversationId, (prev) => prev.filter((item) => item.status === 'running'))
    } catch {
      // next poll reflects the real state
    }
  }

  const statusLabel = (status: BackgroundTaskInfo['status']) =>
    status === 'completed'
      ? t.chatBgStatusCompleted
      : status === 'failed'
        ? t.chatBgStatusFailed
        : t.chatBgStatusStopped

  if (tasks.length === 0 && !roster?.agents.length) {
    return (
      <div className="grid flex-1 place-items-center px-6 text-center text-[12px] text-neutral-400 dark:text-neutral-500">
        {t.chatBgEmpty}
      </div>
    )
  }

  return (
    <div className="shrink-0 px-2 py-2">
      {conversationId && roster && <SubagentTaskList key={conversationId} conversationId={conversationId} roster={roster} />}
      {running.length > 0 && (
        <div className="px-1 py-1.5 text-[12px] font-medium text-neutral-500">
          {t.chatBgRunning} · {running.length}
        </div>
      )}
      {running.map((task) => (
        <div
          key={task.id}
          className="flex items-start gap-2 rounded-xl px-2 py-2 hover:bg-neutral-100/70"
        >
          <TaskGlyph kind={task.kind} />
          <div className="min-w-0 flex-1">
            <div
              className="truncate font-mono text-[12px] text-neutral-800"
              title={task.title}
            >
              {task.title}
            </div>
            <div className="mt-0.5 truncate text-[11px] text-neutral-400" title={task.summary ?? undefined}>
              {task.pid != null ? `pid ${task.pid} · ` : ''}
              {formatTaskElapsed(task.elapsedSecs)}
              {/* 运行中的 summary 是进度尾巴（子代理的 task_progress：tokens · tools · 最近工具） */}
              {task.summary ? ` · ${task.summary}` : ''}
            </div>
          </div>
          <button
            type="button"
            onClick={() => void stop(task)}
            className="grid size-7 shrink-0 place-items-center rounded-md text-neutral-400 hover:bg-red-50 hover:text-red-600 dark:hover:bg-red-950/30 dark:hover:text-red-300"
            title={t.chatKill}
            aria-label={t.chatKillNamed.replace('{name}', task.title)}
          >
            <Square size={13} strokeWidth={2} fill="currentColor" />
          </button>
        </div>
      ))}
      {finished.length > 0 && (
        <div className="flex items-center justify-between px-1 py-1.5">
          <span className="text-[12px] font-medium text-neutral-500">
            {t.chatBgFinished} · {finished.length}
          </span>
          <button
            type="button"
            onClick={() => void clearFinished()}
            className="rounded-md px-1.5 py-0.5 text-[11px] text-neutral-400 hover:bg-neutral-100 hover:text-neutral-600"
          >
            {t.chatBgClear}
          </button>
        </div>
      )}
      {finished.map((task) => (
        <div key={task.id} className="rounded-xl px-2 py-2 opacity-75">
          <div className="flex items-start gap-2">
            <TaskGlyph kind={task.kind} />
            <div className="min-w-0 flex-1">
              <div
                className="truncate font-mono text-[12px] text-neutral-600"
                title={task.title}
              >
                {task.title}
              </div>
              <div
                className="mt-0.5 truncate text-[11px] text-neutral-400"
                title={task.summary ?? undefined}
              >
                {statusLabel(task.status)}
                {task.exitCode != null && task.exitCode !== 0 ? ` · exit ${task.exitCode}` : ''}
                {task.summary ? ` · ${task.summary}` : ''}
              </div>
            </div>
          </div>
        </div>
      ))}
    </div>
  )
}
