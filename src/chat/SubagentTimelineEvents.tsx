import { useState } from 'react'
import { Ellipsis, Flower2, Users } from 'lucide-react'
import { IconButton } from '../components/Button'
import { useLang } from '../components/i18n'
import { parsedRecord, record, text } from '../api/subagents'
import { AgentProgress, AgentOutput, AgentStatus } from './AgentProgress'
import { SubagentDialog } from './AgentHub'
import { requestDockSubagent } from './dock/dockPreview'
import { normalizeToolCallStatus } from './toolStatus'
import type { SubagentTimelineEntry, SubagentTimelineStatus } from '../generated/chatProtocol'
import type { ToolCallRecord } from './types'
import './AgentHub.css'

type AgentRow = { id: string; name: string; role: string; model: string; jobId: string; progress: Record<string, unknown>; result: Record<string, unknown>; index: number }
const arrayRecords = (value: unknown) => Array.isArray(value) ? value.map(record) : []
const agentColors = ['text-violet-400', 'text-emerald-400', 'text-rose-400', 'text-sky-400']

function agentColor(id: string): string {
  let hash = 0
  for (const char of id) hash = (hash * 31 + char.charCodeAt(0)) % agentColors.length
  return agentColors[hash]
}

const lifecycleLabels: Record<SubagentTimelineStatus, [string, string]> = {
  started: ['开始工作', 'started working'],
  completed: ['已完成', 'completed'],
  failed: ['失败', 'failed'],
  interrupted: ['已中断', 'was interrupted'],
}

export function SubagentLifecycleEvent({ event, conversationId, live = false }: {
  event: SubagentTimelineEntry
  conversationId?: string | null
  live?: boolean
}) {
  const zh = useLang() === 'zh'
  const label = `${event.name || event.agentId} ${lifecycleLabels[event.status][zh ? 0 : 1]}`
  const content = <>
    <Flower2 size={14} className={`shrink-0 ${agentColor(event.agentId)}`} aria-hidden="true" />
    <span>{label}</span>
  </>
  return <div className={`not-prose kv-agent-event${live ? ' chat-motion-bubble-in' : ''}`}>
    {conversationId ? <button type="button" className="kv-agent-event-link" onClick={() => requestDockSubagent({ conversationId, agentId: event.agentId })}>
      {content}
    </button> : <span className="kv-agent-event-link">{content}</span>}
  </div>
}

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
  return rows
}

export function SubagentTimelineEvents({ toolCall, conversationId }: { toolCall: ToolCallRecord; conversationId?: string | null }) {
  const zh = useLang() === 'zh'
  const [inspection, setInspection] = useState<'receipt' | null>(null)
  const t = (a: string, b: string) => zh ? a : b
  const args = parsedRecord(toolCall.arguments ?? toolCall.args ?? toolCall.input)
  const structured = record(toolCall.structured_content ?? toolCall.structuredContent)
  const tasks = arrayRecords(args.tasks)
  const rows = taskRows(structured)
  const status = normalizeToolCallStatus(toolCall.status)
  const pending = status === 'running' || status === 'pending'
  const label = t('派工回执', 'Dispatch receipt')
  const dispatchStatus = status === 'error' ? t('派工失败', 'dispatch failed')
    : status === 'cancelled' || status === 'skipped' ? t('派工已中断', 'dispatch cancelled')
      : pending ? t('正在派工', 'dispatching') : t('已派工', 'dispatched')
  const result = text(toolCall.result_preview ?? toolCall.resultPreview ?? toolCall.result ?? toolCall.output)
  const issue = toolCall.error || text(structured.error)
  const root = conversationId || toolCall.conversationId
  const close = () => setInspection(null)
  const timelineAgentIds = new Set(Array.isArray(structured.timelineAgentIds) ? structured.timelineAgentIds : [])
  const events = rows.filter(row => !timelineAgentIds.has(row.id)).map(row => ({
    id: row.id, key: row.id || row.index, name: row.name || t('子代理', 'Subagent'),
    state: text(row.result.status) || text(row.progress.status),
  }))
  if (events.length === 0 && timelineAgentIds.size > 0 && !issue && !inspection) return null
  if (events.length === 0 && (timelineAgentIds.size === 0 || issue)) events.push({
    id: '', key: 'dispatch', name: tasks.length === 1
      ? text(tasks[0].name) || text(tasks[0].agent) || t('子代理', 'Subagents')
      : t('子代理', 'Subagents'),
    state: '',
  })
  return <section className="not-prose kv-agent-events">
    {events.map(({ id, key, name, state }, index) => {
      const action = state === 'running' ? t('开始工作', 'started working')
        : state === 'completed' ? t('已完成', 'completed')
          : state === 'failed' || state === 'error' ? t('失败', 'failed')
            : state === 'aborted' || state === 'cancelled' ? t('已中断', 'was interrupted')
              : state === 'queued' || state === 'pending' ? t('等待运行', 'queued') : dispatchStatus
      const inspectAgent = root && id
      const caption = `${name} ${action}`
      return <div className="kv-agent-event" key={key}>
        <button type="button" className="kv-agent-event-link" aria-haspopup={inspectAgent ? undefined : 'dialog'}
          onClick={() => inspectAgent && root ? requestDockSubagent({ conversationId: root, agentId: id }) : setInspection('receipt')}>
          {id ? <Flower2 size={14} className={`shrink-0 ${agentColor(id)}`} aria-hidden="true" />
            : <Users size={14} className="shrink-0" aria-hidden="true" />}
          <span>{caption}</span>
        </button>
        {index === 0 && inspectAgent && <IconButton variant="ghost" className="kv-agent-event-receipt"
          label={t('查看派工回执', 'Inspect dispatch receipt')} aria-haspopup="dialog" onClick={() => setInspection('receipt')}>
          <Ellipsis size={14} aria-hidden="true" />
        </IconButton>}
      </div>
    })}
    {issue && !inspection && <p role="alert" className="break-words pl-6 text-xs text-[var(--danger)]">{issue}</p>}
    {inspection === 'receipt' && <SubagentDialog title={label} onClose={close}>
      <div className="kv-agent-panel-scroll space-y-4 text-[13px] leading-6 custom-scrollbar">
        {issue && <p role="alert" className="whitespace-pre-wrap break-words text-[var(--danger)]">{issue}</p>}
        {text(args.context) && <div><span className="text-xs text-[var(--text-muted)]">{t('上下文', 'Context')}</span><AgentOutput content={text(args.context)} /></div>}
        {tasks.map((task, index) => <div key={index}>
          <span className="text-xs font-medium text-[var(--text-muted)]">{text(task.name) || text(task.agent) || t(`任务 ${index + 1}`, `Task ${index + 1}`)}</span>
          {text(task.agent) && text(task.agent) !== text(task.name) && <span className="ml-2 text-xs text-[var(--text-muted)]">{text(task.agent)}</span>}
          {text(task.task) && <AgentOutput content={text(task.task)} />}
          {text(task.taskFile) && <p className="break-all font-mono text-xs">{text(task.taskFile)}</p>}
          {text(task.solutionSpace) && <p className="whitespace-pre-wrap break-words">{text(task.solutionSpace)}</p>}
        </div>)}
        {rows.map(row => {
          const output = text(row.result.output) || text(row.result.result)
          const name = row.name || t('子代理', 'Subagent')
          return <article className="kv-agent-answer" key={row.id || row.index}>
            <h4>{name}<AgentStatus state={text(row.result.status) || text(row.progress.status)} zh={zh} /></h4>
            {(row.model || text(row.progress.model) || row.role) && <p className="break-words text-xs text-[var(--text-muted)]">{[row.role, row.model || text(row.progress.model)].filter(Boolean).join(' · ')}</p>}
            <AgentProgress value={row.progress} expanded={!output} zh={zh} />
            {row.jobId && <p className="break-all text-xs text-[var(--text-muted)]">{t('任务', 'Job')}: {row.jobId}</p>}
            {output && <AgentOutput content={output} />}
            {['error', 'warning', 'mergeError', 'outputPath', 'transcriptPath'].map(key => text(row.result[key]) && <p key={key} className="whitespace-pre-wrap break-words text-xs">{text(row.result[key])}</p>)}
            {row.result.truncated === true && <p className="text-xs text-[var(--text-muted)]">{t('回执输出已截断，可查看完整历史。', 'Receipt output was truncated. Inspect the full transcript.')}</p>}
          </article>
        })}
        {text(structured.reason) && <p className="break-words">{text(structured.reason)}</p>}
        {result && rows.length === 0 && <AgentOutput content={result} />}
      </div>
    </SubagentDialog>}
  </section>
}
