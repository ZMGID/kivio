import { Fragment, useState, type ReactNode } from 'react'
import { Bot, Ellipsis, Users } from 'lucide-react'
import { IconButton } from '../components/Button'
import { useLang } from '../components/i18n'
import { ProviderIcon } from '../components/ModelIcon'
import { formatTokensK } from '../utils/tokens'
import { parsedRecord, record, text } from '../api/subagents'
import { AgentProgress, AgentOutput, AgentStatus } from './AgentProgress'
import { SubagentDialog } from './AgentHub'
import { requestDockSubagent } from './dock/dockPreview'
import { AgentAvatar } from './AgentAvatar'
import { toolRecordId, externalSubagentInput, lifecycleSubagentProvider, resumedSubagentName, type LifecycleSubagentProvider } from './segments'
import { normalizeToolCallStatus } from './toolStatus'
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

export function SubagentLifecycleEvent({ event, parallel, conversationId, live = false, avatar, onOpen }: {
  event: SubagentTimelineEntry
  parallel?: SubagentTimelineEntry[]
  conversationId?: string | null
  live?: boolean
  /** 外部 CLI 子代理换成供应商图标；默认是内置子代理的形状头像。 */
  avatar?: (agentId: string) => ReactNode
  /** 外部 CLI 子代理不在内置注册表里，点击由调用方打开自己的详情。 */
  onOpen?: (agentId: string) => void
}) {
  const zh = useLang() === 'zh'
  const action = lifecycleLabels[event.status][zh ? 0 : 1]
  const label = `${event.name || event.agentId} ${action}`
  const className = `not-prose kv-agent-event${live ? ' chat-motion-bubble-in' : ''}`
  const renderAvatar = avatar ?? ((agentId: string) => <AgentAvatar agentId={agentId} />)
  const open = onOpen ?? (conversationId
    ? (agentId: string) => requestDockSubagent({ conversationId, agentId })
    : null)
  if (parallel?.length) {
    const agents = [event, ...parallel]
    return <div className={className}>
      <span className="flex max-w-[40%] shrink-0 flex-wrap items-center gap-1">
        {agents.map(agent => <Fragment key={agent.agentId}>{renderAvatar(agent.agentId)}</Fragment>)}
      </span>
      <span className="min-w-0 text-[13px] leading-5 text-[var(--text-muted)] [overflow-wrap:anywhere]">
        {agents.map((agent, index) => {
          const name = agent.name || agent.agentId
          return <Fragment key={agent.agentId}>
            {index > 0 && (index === agents.length - 1 ? (zh ? '和' : ' and ') : (zh ? '、' : ', '))}
            {open ? <button type="button" className="kv-agent-event-link max-w-full align-baseline"
              aria-label={`${name} ${action}`}
              onClick={() => open(agent.agentId)}>{name}</button> : name}
          </Fragment>
        })}
        {' '}{action}
      </span>
    </div>
  }
  const content = <>
    {renderAvatar(event.agentId)}
    <span>{label}</span>
  </>
  return <div className={className}>
    {open ? <button type="button" className="kv-agent-event-link" onClick={() => open(event.agentId)}>
      {content}
    </button> : <span className="kv-agent-event-link">{content}</span>}
  </div>
}

type ExternalSubagentView = {
  id: string
  provider: LifecycleSubagentProvider
  name: string
  prompt: string
  model: string
  usage: string
  state: 'running' | 'completed' | 'failed' | 'cancelled'
  finishedAt: number
  progress: Record<string, unknown>
  result: string
  error: string
  /** 旧版把后台派出回执当成了结果并标为完成：子代理早已结束，但结果和成败都没有保存。 */
  unrecorded: boolean
}

const externalFinish: Record<Exclude<ExternalSubagentView['state'], 'running'>, SubagentTimelineStatus> = {
  completed: 'completed',
  failed: 'failed',
  cancelled: 'interrupted',
}

const providerLabels: Record<LifecycleSubagentProvider, { icon: string; zh: string; en: string }> = {
  codex: { icon: 'openai', zh: 'Codex 子代理', en: 'Codex subagent' },
  claude: { icon: 'claude', zh: 'Claude 子代理', en: 'Claude subagent' },
  opencode: { icon: 'opencode', zh: 'OpenCode 子代理', en: 'OpenCode subagent' },
  external: { icon: '', zh: '子代理', en: 'Subagent' },
}

/** 派出回执只表示子代理开始跑：Codex 的 `started subagent …`；Claude 后台子代理的
 *  `Async agent launched successfully.`（旧记录里可能还包在 JSON 文本块里）。 */
function isLaunchReceipt(resultPreview: string): boolean {
  const trimmed = resultPreview.trim()
  return trimmed.startsWith('started subagent ') || trimmed.includes('Async agent launched successfully.')
    || Boolean(text(parsedRecord(trimmed).resumedAgentId))
    || trimmed.startsWith('started background subagent job ')
    || trimmed.startsWith('started background subagent task ')
}

function externalSubagentView(toolCall: ToolCallRecord, provider: LifecycleSubagentProvider, zh: boolean): ExternalSubagentView {
  const input = externalSubagentInput(toolCall)
  const args = provider === 'opencode' ? record(input.rawInput) : input
  const structured = record(toolCall.structured_content ?? toolCall.structuredContent)
  const progress = record(structured.subagentProgress)
  const resultPreview = text(structured.result) || text(toolCall.result_preview ?? toolCall.resultPreview ?? toolCall.result ?? toolCall.output)
  // shortcut: OpenCode ACP 尚未订阅跨回合子会话终态，接入该通道前，后台派出回执保持运行中。
  const receipt = provider === 'opencode'
    ? record(record(structured.rawOutput ?? input.rawOutput).metadata).status === 'running'
    : isLaunchReceipt(resultPreview)
  const status = normalizeToolCallStatus(toolCall.status)
  const state = status === 'error' ? 'failed' : status === 'cancelled' ? 'cancelled'
    : receipt || status === 'running' || status === 'pending' ? 'running' : 'completed'
  const usage = record(structured.usage)
  const label = providerLabels[provider]
  return {
    id: toolRecordId(toolCall),
    provider,
    name: text(args.description) || (structured.type === 'subagent' ? text(structured.name) : '')
      || text(args.name) || resumedSubagentName(toolCall) || text(progress.name)
      || text(args.subagent_type) || text(args.agent) || (zh ? label.zh : label.en),
    // `SendMessage` 的任务是发给子代理的那句话。
    prompt: text(args.prompt) || text(args.message),
    model: text(structured.model) || text(progress.model) || text(args.model),
    usage: [['↑', usage.inputTokens], ['↓', usage.outputTokens]].flatMap(([label, value]) =>
      typeof value === 'number' && Number.isFinite(value) && value >= 0 ? [`${label}${formatTokensK(value)}`] : []).join(' '),
    state,
    finishedAt: toolCall.completed_at ?? toolCall.completedAt ?? 0,
    progress: { ...progress, preview: receipt || !resultPreview ? progress.preview : undefined,
      tokens: progress.tokens ?? usage.totalTokens,
      durationMs: progress.durationMs ?? toolCall.duration_ms ?? toolCall.durationMs },
    result: receipt ? '' : provider === 'opencode'
      ? (/^<subagent\b[^>]*>\s*([\s\S]*?)\s*<\/subagent>$/.exec(resultPreview.trim())?.[1]
        ?? resultPreview) : resultPreview,
    error: toolCall.error || text(structured.error),
    unrecorded: (provider === 'codex' || provider === 'claude') && receipt && status === 'completed',
  }
}

/** 外部 CLI 子代理复用内置子代理时间线行，头像换成各 CLI 的图标。
 *  连续派出的合并成一行「A 和 B 开始工作」，结束的按完成顺序各一行；点名字看任务与回答。 */
export function ExternalSubagentEvents({ toolCalls, live = false }: { toolCalls: ToolCallRecord[]; live?: boolean }) {
  const zh = useLang() === 'zh'
  const [openId, setOpenId] = useState<string | null>(null)
  const agents = toolCalls.flatMap((toolCall) => {
    const provider = lifecycleSubagentProvider(toolCall)
    return provider ? [externalSubagentView(toolCall, provider, zh)] : []
  }).filter(agent => agent.id)
  if (agents.length === 0) return null
  const providers = new Map(agents.map(agent => [agent.id, agent.provider]))
  const avatar = (agentId: string) => {
    const icon = providerLabels[providers.get(agentId) ?? 'external'].icon
    return icon ? <ProviderIcon name={icon} size={18} className="shrink-0" />
      : <Bot size={18} className="shrink-0" aria-hidden="true" />
  }
  const entry = (agent: ExternalSubagentView, status: SubagentTimelineStatus): SubagentTimelineEntry => ({
    agentId: agent.id, name: agent.name, status, parentToolCallId: '',
    afterSegmentId: null, afterOrder: null, textChars: null,
  })
  const [first, ...rest] = agents.map(agent => entry(agent, 'started'))
  const finished = agents.filter(agent => agent.state !== 'running')
    .sort((left, right) => left.finishedAt - right.finishedAt)
  const opened = agents.find(agent => agent.id === openId)
  const output = opened?.result
  return <>
    <SubagentLifecycleEvent event={first} parallel={rest} live={live} avatar={avatar} onOpen={setOpenId} />
    {finished.map(agent => agent.state !== 'running' && <SubagentLifecycleEvent key={agent.id}
      event={entry(agent, externalFinish[agent.state])} live={live} avatar={avatar} onOpen={setOpenId} />)}
    {opened && <SubagentDialog title={opened.name} onClose={() => setOpenId(null)}>
      <div className="kv-agent-panel-scroll space-y-4 text-[13px] leading-6 custom-scrollbar">
        <article className="kv-agent-answer">
          <h4>{opened.name}{!opened.unrecorded && <AgentStatus state={opened.state} zh={zh} />}</h4>
          {opened.model && <p className="break-words text-xs text-[var(--text-muted)]">{opened.model}</p>}
          {opened.prompt && <div><span className="text-xs text-[var(--text-muted)]">{zh ? '任务' : 'Task'}</span><AgentOutput content={opened.prompt} /></div>}
          <AgentProgress value={opened.progress} zh={zh} />
          {opened.usage && <p className="text-xs text-[var(--text-muted)]">{opened.usage} tokens</p>}
          {output && <div><span className="text-xs text-[var(--text-muted)]">{zh ? '回答' : 'Reply'}</span><AgentOutput content={output} /></div>}
          {opened.unrecorded && <p className="text-xs text-[var(--text-muted)]">
            {zh ? '这条旧记录没有保存后台子代理的结果，无法确认完成状态。' : 'This older record did not save the background subagent\'s result, so its final status is unknown.'}
          </p>}
          {opened.error && <p role="alert" className="whitespace-pre-wrap break-words text-[var(--danger)]">{opened.error}</p>}
        </article>
      </div>
    </SubagentDialog>}
  </>
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
  const label = t('派工回执', 'Dispatch receipt')
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
  // A batch rejected before any subagent started is the model's to retry, not a timeline event;
  // the error still returns to the model. Only started subagents appear here.
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
