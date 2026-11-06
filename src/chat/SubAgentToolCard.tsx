import { useState } from 'react'
import { ChevronRight, Users } from 'lucide-react'
import { useLang } from '../components/i18n'
import { Button } from '../components/Button'
import { parsedRecord, record, subagentProgress, text } from '../api/subagents'
import { AgentProgress, AgentOutput } from './AgentProgress'
import { AgentHub } from './AgentHub'
import { canonicalToolName } from './segments'
import { normalizeToolCallStatus } from './toolStatus'
import type { ToolCallRecord } from './types'

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

type AgentRow = { id: string; name: string; role: string; model: string; jobId: string; progress: Record<string, unknown>; result: Record<string, unknown>; index: number }
const arrayRecords = (value: unknown) => Array.isArray(value) ? value.map(record) : []
const settled = (row: AgentRow) => ['completed', 'failed', 'error', 'aborted', 'cancelled'].includes(text(row.result.status) || text(row.progress.status))
const problem = (row: AgentRow) => ['failed', 'error', 'aborted', 'cancelled'].includes(text(row.result.status) || text(row.progress.status)) || Boolean(row.result.error || row.result.warning || row.result.mergeError)
const duration = (row: AgentRow) => subagentProgress(row.result)?.durationMs ?? subagentProgress(row.progress)?.durationMs ?? 0

function taskRows(structured: Record<string, unknown>): AgentRow[] {
  const agents = arrayRecords(structured.agents)
  const updates = { ...record(structured.progressByTask), ...record(structured.subagentProgressByTask) }
  for (const progress of arrayRecords(structured.progress)) {
    const id = text(progress.id) || text(progress.taskId)
    if (id) updates[id] = progress
  }
  const rows: AgentRow[] = agents.map((agent, index) => ({
    id: text(agent.agentId) || text(agent.id), name: text(agent.name) || text(agent.agentId) || text(agent.id) || text(agent.agent),
    role: text(agent.agent), model: text(agent.model), jobId: text(agent.jobId),
    progress: record(updates[text(agent.agentId) || text(agent.id)]), result: {}, index,
  }))
  for (const [id, value] of Object.entries(updates)) {
    if (!rows.some(row => row.id === id)) {
      const progress = record(value)
      rows.push({ id, name: text(progress.name) || id, role: text(progress.agent), model: text(progress.model), jobId: '', progress, result: {}, index: rows.length })
    }
  }
  for (const result of arrayRecords(structured.results)) {
    const id = text(result.agentId) || text(result.id)
    const existing = rows.find(row => row.id === id || row.jobId === text(result.id))
    if (existing) {
      existing.result = result
      existing.progress = { ...existing.progress, ...result, ...record(result.progress) }
    } else rows.push({ id, name: text(result.name) || text(result.agent) || id, role: text(result.agent), model: text(result.resolvedModel) || text(result.model), jobId: '', progress: { ...result, ...record(result.progress) }, result, index: rows.length })
  }
  return rows.sort((a, b) => Number(!settled(a)) - Number(!settled(b)) || (settled(a) ? duration(a) - duration(b) : 0) || a.index - b.index)
}

export function SubAgentToolCard({ toolCall, conversationId }: { toolCall: ToolCallRecord; conversationId?: string | null }) {
  const zh = useLang() === 'zh'
  const [expanded, setExpanded] = useState(false)
  const [inspected, setInspected] = useState<string | null>(null)
  const t = (a: string, b: string) => zh ? a : b
  const waiting = canonicalToolName(toolCall) === 'wait'
  const args = parsedRecord(toolCall.arguments ?? toolCall.args ?? toolCall.input)
  const structured = record(toolCall.structured_content ?? toolCall.structuredContent)
  const tasks = arrayRecords(args.tasks)
  const jobs = arrayRecords(structured.jobs)
  const messages = arrayRecords(structured.messages)
  const rows = taskRows(structured)
  const status = normalizeToolCallStatus(toolCall.status)
  const pending = status === 'running' || status === 'pending'
  const failed = status === 'error' || status === 'cancelled' || status === 'skipped'
  const running = rows.filter(row => row.progress.status === 'running' && !settled(row)).length
  const queued = rows.filter(row => !settled(row) && (row.progress.status === 'pending' || row.progress.status === 'queued')).length
  const finished = rows.filter(settled).length
  const label = waiting ? t('等待子代理', 'Wait for sub-agents')
    : [t('子代理', 'Sub-agents'), tasks.length === 1 ? text(tasks[0].name) || text(tasks[0].agent) : tasks.length > 1 ? t(`${tasks.length} 个任务`, `${tasks.length} tasks`) : ''].filter(Boolean).join(' · ')
  const interrupted = structured.interrupted === true || structured.status === 'interrupted'
  const runningJobs = jobs.filter(job => job.status === 'running').length
  const noWork = Array.isArray(structured.jobs) && jobs.length === 0 && messages.length === 0
  const delivery = text(record(structured.delivery).status) || text(structured.deliveryStatus)
  const summary = failed ? status === 'error' ? t('操作异常', 'Operation error') : t('已中断', 'Cancelled')
    : waiting ? pending ? jobs.length ? t(`正在等待 ${jobs.length} 项作业`, `Waiting for ${jobs.length} jobs`) : t('正在等待结果', 'Waiting for results') : interrupted ? t('等待已打断', 'Wait interrupted') : messages.length ? t(`收到 ${messages.length} 条消息`, `${messages.length} messages received`) : runningJobs ? t(`${runningJobs} 项仍在运行`, `${runningJobs} still running`) : delivery ? t('结果已送达', 'Results delivered') : noWork ? t('无需等待', 'Nothing to wait for') : t('已完成', 'Completed')
      : running ? t(`${running} 个运行中`, `${running} running`)
        : queued ? t(`${queued} 个等待运行`, `${queued} queued`)
          : finished ? t(`${finished} 个已结束`, `${finished} finished`)
            : pending ? t('正在启动…', 'Starting…') : t('已调度', 'Scheduled')
  const result = text(toolCall.result_preview ?? toolCall.resultPreview ?? toolCall.result ?? toolCall.output)
  const issue = toolCall.error || text(structured.error)
  const active = rows.filter(row => !settled(row))
  const collapsed = active.length ? [...rows.filter(settled).slice(-Math.max(0, 4 - active.length)), ...active].slice(-4)
    : [...rows.filter(problem), ...rows.filter(row => !problem(row))].slice(0, 4)
  const visible = expanded ? rows : collapsed
  const root = conversationId || toolCall.conversationId
  return <section className="not-prose my-1 min-w-0 text-[12.5px] leading-5">
    <button type="button" aria-expanded={expanded} onClick={() => setExpanded(value => !value)}
      className="flex w-full min-w-0 items-center gap-2 rounded-md px-3 py-2 text-left text-[var(--text-muted)] hover:bg-[var(--bg-hover)] focus-visible:outline focus-visible:outline-2 focus-visible:outline-[var(--color-ring)]">
      <Users size={16} className="shrink-0" aria-hidden="true" /><span className="min-w-0 flex-1 truncate" title={label}>{label}</span>
      <span aria-live="polite" className={`max-w-[50%] truncate text-xs ${failed ? 'text-[var(--danger)]' : ''}`} title={summary}>{summary}</span><ChevronRight size={12} aria-hidden="true" className={`shrink-0 transition-transform ${expanded ? 'rotate-90' : ''}`} />
    </button>
    <div className="ml-5 space-y-2 border-l border-[var(--color-border)] py-1 pl-3">
      {expanded && <>
        {issue && <p role="alert" className="whitespace-pre-wrap break-words text-[var(--danger)]">{issue}</p>}
        {text(args.context) && <div><span className="text-xs text-[var(--text-muted)]">{t('上下文', 'Context')}</span><p className="whitespace-pre-wrap break-words">{text(args.context)}</p></div>}
        {tasks.map((task, index) => <div key={index}>
          <span className="text-xs text-[var(--text-muted)]">{text(task.name) || text(task.agent) || t(`任务 ${index + 1}`, `Task ${index + 1}`)}</span>
          {text(task.agent) && text(task.agent) !== text(task.name) && <span className="ml-2 text-xs text-[var(--text-muted)]">{text(task.agent)}</span>}
          {text(task.task) && <AgentOutput content={text(task.task)} zh={zh} lines={20} />}
          {text(task.taskFile) && <p className="break-all font-mono text-xs">{text(task.taskFile)}</p>}
          {text(task.solutionSpace) && <p className="whitespace-pre-wrap break-words"><span className="text-[var(--text-muted)]">{t('解决空间', 'Solution space')}: </span>{text(task.solutionSpace)}</p>}
        </div>)}
      </>}
      {visible.map(row => {
        const output = text(row.result.output) || text(row.result.result)
        return <div key={row.id || row.index}>
          <div className="flex flex-wrap gap-x-2 text-[var(--text-muted)]"><span>{row.name || t('子代理', 'Sub-agent')}</span><span className={problem(row) ? 'text-[var(--danger)]' : undefined}>{statusLabel(text(row.result.status) || text(row.progress.status), zh)}</span></div>
          {(row.model || text(row.progress.model) || row.role) && <p className="break-words text-xs text-[var(--text-muted)]">{[row.role, row.model || text(row.progress.model)].filter(Boolean).join(' · ')}</p>}
          <AgentProgress value={row.progress} expanded={expanded && !output} zh={zh} />
          {expanded && <>
            {row.id && <p className="break-all text-xs text-[var(--text-muted)]">{t('代理', 'Agent')}: {row.id}</p>}
            {row.jobId && <p className="break-all text-xs text-[var(--text-muted)]">{t('任务', 'Job')}: {row.jobId}</p>}
            {output && <AgentOutput content={output} zh={zh} />}
            {['error', 'warning', 'mergeError', 'outputPath', 'transcriptPath'].map(key => text(row.result[key]) && <p key={key} className="whitespace-pre-wrap break-words text-xs">{text(row.result[key])}</p>)}
            {row.result.truncated === true && <p className="text-xs text-[var(--text-muted)]">{t('回执输出已截断，可查看完整历史。', 'Receipt output was truncated. Inspect the full transcript.')}</p>}
            {root && row.id && <Button size="sm" variant="ghost" onClick={() => setInspected(row.id)}>{t('查看代理历史', 'Inspect agent transcript')}</Button>}
          </>}
        </div>
      })}
      {!expanded && rows.length > visible.length && <p className="text-xs text-[var(--text-muted)]">{t(`另有 ${rows.length - visible.length} 项 · 展开查看全部`, `${rows.length - visible.length} more · Expand to see all`)}</p>}
      {expanded && <>
        {text(structured.reason) && <p className="break-words">{text(structured.reason)}</p>}
        {delivery && <p className="break-words text-xs text-[var(--text-muted)]">{t('投递', 'Delivery')}: {delivery}</p>}
        {messages.map((message, index) => <div key={text(message.id) || index} className="space-y-1"><p className="break-words text-xs text-[var(--text-muted)]">{text(message.fromAgent) || text(message.from)} → {text(message.toAgent) || text(message.to)}</p><AgentOutput content={text(message.content) || text(message.body)} zh={zh} /></div>)}
        {jobs.map((job, index) => <div key={text(job.id) || index}>
          <div className="flex flex-wrap gap-x-2 text-[var(--text-muted)]"><span>{text(job.label) || text(job.agentId) || text(job.id)}</span><span>{statusLabel(text(job.status), zh)}</span></div>
          {text(job.agentId) && <p className="break-all text-xs text-[var(--text-muted)]">{t('代理', 'Agent')}: {text(job.agentId)}</p>}
          {text(job.id) && <p className="break-all text-xs text-[var(--text-muted)]">{t('任务', 'Job')}: {text(job.id)}</p>}
          <AgentProgress value={job.progress} zh={zh} />
          {text(job.result) && !result.includes(text(job.result)) && <AgentOutput content={text(job.result)} zh={zh} />}
          {(job.consumed === true || job.delivered === true) && <p className="text-xs text-[var(--text-muted)]">{t('结果已送达', 'Result delivered')}</p>}
        </div>)}
        {arrayRecords(structured.runningAgents).map((agent, index) => <p key={text(agent.id) || index} className="break-words text-xs text-[var(--text-muted)]">{text(agent.displayName) || text(agent.id)} · {t('仍在运行（非本次等待的作业）', 'Still running (outside the jobs being awaited)')}</p>)}
        {result && (waiting || rows.length === 0) && <AgentOutput content={result} zh={zh} />}
      </>}
    </div>
    {inspected && root && <AgentHub key={root} conversationId={root} initialAgentId={inspected} onClose={() => setInspected(null)} />}
  </section>
}
