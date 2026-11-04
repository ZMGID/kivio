import { useState } from 'react'
import { ChevronRight, Users } from 'lucide-react'
import { useLang } from '../components/i18n'
import { canonicalToolName } from './segments'
import { normalizeToolCallStatus } from './toolStatus'
import type { ToolCallRecord } from './types'

const object = (value: unknown): Record<string, unknown> => value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {}
const string = (value: unknown) => typeof value === 'string' ? value : ''
function parsed(value: unknown) {
  if (typeof value !== 'string') return object(value)
  try { return object(JSON.parse(value)) } catch { return {} }
}

function statusLabel(status: string, zh: boolean): string {
  switch (status) {
    case 'pending':
    case 'queued': return zh ? '等待运行' : 'Queued'
    case 'running': return zh ? '运行中' : 'Running'
    case 'completed': return zh ? '已完成' : 'Completed'
    case 'failed':
    case 'error': return zh ? '失败' : 'Failed'
    case 'aborted':
    case 'cancelled': return zh ? '已中断' : 'Cancelled'
    default: return zh ? '已调度' : 'Scheduled'
  }
}

function Progress({ value }: { value: Record<string, unknown> }) {
  const steps = Array.isArray(value.steps) ? value.steps.filter((step): step is string => typeof step === 'string' && Boolean(step)) : []
  const preview = string(value.preview) || (Array.isArray(value.recentOutput) ? value.recentOutput.filter((line): line is string => typeof line === 'string').join('\n') : '')
  const stage = string(value.stage)
  const currentTool = string(value.currentToolIntent) || string(value.currentTool)
  return <>
    {steps.map((step, index) => <p key={index} className="break-words font-mono text-xs text-neutral-500">{step}</p>)}
    {stage && <p className="whitespace-pre-wrap break-words">{stage}</p>}
    {currentTool && <p className="break-words font-mono text-xs text-neutral-500">{currentTool}</p>}
    {preview && <p className="whitespace-pre-wrap break-words">{preview}</p>}
  </>
}

export function SubAgentToolCard({ toolCall }: { toolCall: ToolCallRecord }) {
  const zh = useLang() === 'zh'
  const [expanded, setExpanded] = useState(false)
  const t = (a: string, b: string) => zh ? a : b
  const waiting = canonicalToolName(toolCall) === 'wait'
  const args = parsed(toolCall.arguments ?? toolCall.args ?? toolCall.input)
  const structured = object(toolCall.structured_content ?? toolCall.structuredContent)
  const tasks = Array.isArray(args.tasks) ? args.tasks.map(object) : []
  const agents = Array.isArray(structured.agents) ? structured.agents.map(object) : []
  const jobs = Array.isArray(structured.jobs) ? structured.jobs.map(object) : []
  const updates = object(structured.subagentProgressByTask)
  const rows = agents.map(agent => {
    const id = string(agent.agentId)
    return {
      id,
      name: string(agent.name) || id || string(agent.agent),
      jobId: string(agent.jobId),
      progress: object(updates[id]),
    }
  })
  for (const [id, update] of Object.entries(updates)) {
    if (!rows.some(row => row.id === id)) {
      const progress = object(update)
      rows.push({ id, name: string(progress.name) || id, jobId: '', progress })
    }
  }
  const status = normalizeToolCallStatus(toolCall.status)
  const pending = status === 'running' || status === 'pending'
  const failed = status === 'error' || status === 'cancelled' || status === 'skipped'
  const running = rows.filter(row => row.progress.status === 'running').length
  const queued = rows.filter(row => row.progress.status === 'pending' || row.progress.status === 'queued').length
  const finished = rows.filter(row => ['completed', 'failed', 'aborted', 'cancelled'].includes(string(row.progress.status))).length
  const label = waiting ? t('等待子代理', 'Wait for sub-agents')
    : [t('子代理', 'Sub-agents'), tasks.length === 1 ? string(tasks[0].name) || string(tasks[0].agent) : tasks.length > 1 ? t(`${tasks.length} 个任务`, `${tasks.length} tasks`) : ''].filter(Boolean).join(' · ')
  const summary = failed ? status === 'error' ? t('操作异常', 'Operation error') : t('已中断', 'Cancelled')
    : waiting ? pending ? t('正在等待结果', 'Waiting for results') : t('已完成', 'Completed')
      : running ? t(`${running} 个运行中`, `${running} running`)
        : queued ? t(`${queued} 个等待运行`, `${queued} queued`)
          : finished ? t(`${finished} 个已结束`, `${finished} finished`)
            : pending ? t('正在启动…', 'Starting…') : t('已调度', 'Scheduled')
  const result = string(toolCall.result_preview ?? toolCall.resultPreview ?? toolCall.result ?? toolCall.output)
  const issue = toolCall.error || string(structured.error)

  return <section className="not-prose my-1 min-w-0 text-[12.5px] leading-5">
    <button type="button" aria-expanded={expanded} onClick={() => setExpanded(value => !value)}
      className="flex w-full min-w-0 items-center gap-2 rounded-md px-3 py-2 text-left text-neutral-500 hover:bg-neutral-500/5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-neutral-400">
      <Users size={16} className="shrink-0" aria-hidden="true" />
      <span className="min-w-0 flex-1 truncate" title={label}>{label}</span>
      <span className={`max-w-[50%] truncate text-xs ${failed ? 'text-red-600' : ''}`} title={summary}>{summary}</span>
      <ChevronRight size={12} aria-hidden="true" className={`shrink-0 transition-transform ${expanded ? 'rotate-90' : ''}`} />
    </button>
    {expanded && <div className="ml-5 space-y-2 border-l border-neutral-200 py-2 pl-3">
      {issue && <p role="alert" className="whitespace-pre-wrap break-words text-red-600">{issue}</p>}
      {string(args.context) && <div><span className="text-xs text-neutral-500">{t('上下文', 'Context')}</span><p className="whitespace-pre-wrap break-words">{string(args.context)}</p></div>}
      {tasks.map((task, index) => <div key={index}>
        <span className="text-xs text-neutral-500">{string(task.name) || string(task.agent) || t(`任务 ${index + 1}`, `Task ${index + 1}`)}</span>
        {string(task.agent) && string(task.agent) !== string(task.name) && <span className="ml-2 text-xs text-neutral-500">{string(task.agent)}</span>}
        {string(task.task) && <p className="whitespace-pre-wrap break-words">{string(task.task)}</p>}
        {string(task.solutionSpace) && <p className="whitespace-pre-wrap break-words"><span className="text-neutral-500">{t('解决空间', 'Solution space')}: </span>{string(task.solutionSpace)}</p>}
      </div>)}
      {rows.map((row, index) => <div key={row.id || index}>
        <div className="flex flex-wrap gap-x-2 text-neutral-500"><span>{row.name || t('子代理', 'Sub-agent')}</span><span>{statusLabel(string(row.progress.status), zh)}</span></div>
        {row.id && <p className="break-all text-xs text-neutral-500">{t('代理', 'Agent')}: {row.id}</p>}
        {row.jobId && <p className="break-all text-xs text-neutral-500">{t('任务', 'Job')}: {row.jobId}</p>}
        <Progress value={row.progress} />
      </div>)}
      {jobs.map((job, index) => <div key={string(job.id) || index}>
        <div className="flex flex-wrap gap-x-2 text-neutral-500"><span>{string(job.label) || string(job.agentId) || string(job.id)}</span><span>{statusLabel(string(job.status), zh)}</span></div>
        {string(job.agentId) && <p className="break-all text-xs text-neutral-500">{t('代理', 'Agent')}: {string(job.agentId)}</p>}
        {string(job.id) && <p className="break-all text-xs text-neutral-500">{t('任务', 'Job')}: {string(job.id)}</p>}
        <Progress value={object(job.progress)} />
        {string(job.result) && !result.includes(string(job.result)) && <pre className="whitespace-pre-wrap break-words font-sans">{string(job.result)}</pre>}
      </div>)}
      {result && (waiting || !agents.length) && <pre className="whitespace-pre-wrap break-words font-sans">{result}</pre>}
    </div>}
  </section>
}
