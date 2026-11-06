import { Fragment, useState } from 'react'
import { Ellipsis, Users } from 'lucide-react'
import { IconButton } from '../components/Button'
import { useLang } from '../components/i18n'
import { parsedRecord, record, text } from '../api/subagents'
import { AgentProgress, AgentOutput, AgentStatus } from './AgentProgress'
import { SubagentDialog } from './AgentHub'
import { requestDockSubagent } from './dock/dockPreview'
import { normalizeToolCallStatus } from './toolStatus'
import { AgentAvatar } from './AgentAvatar'
import type { SubagentTimelineEntry, SubagentTimelineStatus } from '../generated/chatProtocol'
import type { ToolCallRecord } from './types'
import './AgentHub.css'

type AgentRow = { id: string; name: string; role: string; model: string; jobId: string; progress: Record<string, unknown>; result: Record<string, unknown>; index: number }
const arrayRecords = (value: unknown) => Array.isArray(value) ? value.map(record) : []

const lifecycleLabels: Record<SubagentTimelineStatus, [string, string]> = {
  started: ['开始工作', 'started working'],
  completed: ['已完成', 'completed'],
  failed: ['失败', 'failed'],
  interrupted: ['已中断', 'was interrupted'],
}

export function SubagentLifecycleEvent({ event, parallel, conversationId, live = false }: {
  event: SubagentTimelineEntry
  parallel?: SubagentTimelineEntry[]
  conversationId?: string | null
  live?: boolean
}) {
  const zh = useLang() === 'zh'
  const action = lifecycleLabels[event.status][zh ? 0 : 1]
  const label = `${event.name || event.agentId} ${action}`
  const className = `not-prose kv-agent-event${live ? ' chat-motion-bubble-in' : ''}`
  if (parallel?.length) {
    const agents = [event, ...parallel]
    return <div className={className}>
      <span className="flex max-w-[40%] shrink-0 flex-wrap items-center gap-1">
        {agents.map(agent => <AgentAvatar key={agent.agentId} agentId={agent.agentId} />)}
      </span>
      <span className="min-w-0 text-[13px] leading-5 text-[var(--text-muted)] [overflow-wrap:anywhere]">
        {agents.map((agent, index) => {
          const name = agent.name || agent.agentId
          return <Fragment key={agent.agentId}>
            {index > 0 && (index === agents.length - 1 ? (zh ? '和' : ' and ') : (zh ? '、' : ', '))}
            {conversationId ? <button type="button" className="kv-agent-event-link max-w-full align-baseline"
              aria-label={`${name} ${action}`}
              onClick={() => requestDockSubagent({ conversationId, agentId: agent.agentId })}>{name}</button> : name}
          </Fragment>
        })}
        {' '}{action}
      </span>
    </div>
  }
  const content = <>
    <AgentAvatar agentId={event.agentId} />
    <span>{label}</span>
  </>
  return <div className={className}>
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
  const label = t('派工回执', 'Dispatch receipt')
  const dispatchStatus = status === 'error' ? t('派工失败', 'dispatch failed')
    : status === 'cancelled' || status === 'skipped' ? t('派工已中断', 'dispatch cancelled')
      : ''
  const result = text(toolCall.result_preview ?? toolCall.resultPreview ?? toolCall.result ?? toolCall.output)
  const issue = toolCall.error || text(structured.error)
  const root = conversationId || toolCall.conversationId
  const close = () => setInspection(null)
  const timelineAgentIds = new Set(Array.isArray(structured.timelineAgentIds) ? structured.timelineAgentIds : [])
  const events = rows.filter(row => !timelineAgentIds.has(row.id)).flatMap(row => {
    const state = text(row.result.status) || text(row.progress.status)
    const action = state === 'running' ? t('开始工作', 'started working')
      : state === 'completed' ? t('已完成', 'completed')
        : state === 'failed' || state === 'error' ? t('失败', 'failed')
          : state === 'aborted' || state === 'cancelled' ? t('已中断', 'was interrupted') : ''
    return action ? [{ id: row.id, key: row.id || row.index, name: row.name || t('子代理', 'Subagent'), action }] : []
  })
  if (rows.length === 0 && timelineAgentIds.size === 0 && (dispatchStatus || issue)) events.push({
    id: '', key: 'dispatch', name: tasks.length === 1
      ? text(tasks[0].name) || text(tasks[0].agent) || t('子代理', 'Subagents')
      : t('子代理', 'Subagents'),
    action: dispatchStatus || t('派工失败', 'dispatch failed'),
  })
  if (events.length === 0 && !inspection) return null
  return <section className="not-prose kv-agent-events">
    {events.map(({ id, key, name, action }, index) => {
      const inspectAgent = root && id
      const caption = `${name} ${action}`
      return <div className="kv-agent-event" key={key}>
        <button type="button" className="kv-agent-event-link" aria-haspopup={inspectAgent ? undefined : 'dialog'}
          onClick={() => inspectAgent && root ? requestDockSubagent({ conversationId: root, agentId: id }) : setInspection('receipt')}>
          {id ? <AgentAvatar agentId={id} />
            : <Users size={14} className="shrink-0" aria-hidden="true" />}
          <span>{caption}</span>
        </button>
        {index === 0 && inspectAgent && <IconButton variant="ghost" className="kv-agent-event-receipt"
          label={t('查看派工回执', 'Inspect dispatch receipt')} aria-haspopup="dialog" onClick={() => setInspection('receipt')}>
          <Ellipsis size={14} aria-hidden="true" />
        </IconButton>}
      </div>
    })}
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
