import { useEffect, useId, useRef, useState, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { Activity, ArrowLeft, ChevronRight, Users, X } from 'lucide-react'
import { Button, IconButton } from '../components/Button'
import { useLang } from '../components/i18n'
import { Input, TextArea } from '../settings/public/controls'
import { subagentProgress, subagentsApi, type SubagentActivity, type SubagentRoster, type SubagentTranscriptEntry, type SubagentView } from '../api/subagents'
import { AgentProgress, AgentStatus } from './AgentProgress'
import { useSubagentRoster } from './useSubagentRoster'
import './AgentHub.css'

const errorText = (error: unknown) => error instanceof Error ? error.message : String(error)
type Inspection = { agentId: string; entry?: SubagentActivity } | 'activity'

export function SubagentTaskList({ conversationId, roster }: { conversationId: string; roster: SubagentRoster }) {
  const zh = useLang() === 'zh'
  const [inspection, setInspection] = useState<Inspection | null>(null)
  if (!roster.agents.length) return null
  const active = roster.agents.filter(agent => agent.status === 'running')
  const finished = roster.agents.filter(agent => agent.status !== 'running')
  return <section className="kv-agent-task-list" aria-label={zh ? '子代理任务' : 'Subagent tasks'}>
    <header className="flex items-center gap-2 px-2 py-1.5">
      <Users size={15} aria-hidden="true" />
      <h3 className="min-w-0 flex-1 text-xs font-semibold">{zh ? '子代理' : 'Subagents'}</h3>
      <IconButton variant="ghost" aria-haspopup="dialog" label={zh ? '查看子代理活动' : 'View subagent activity'} onClick={() => setInspection('activity')}><Activity size={15} /></IconButton>
    </header>
    {([
      [zh ? '进行中' : 'Active', active],
      [zh ? '历史' : 'History', finished],
    ] satisfies [string, SubagentView[]][]).map(([label, agents]) => agents.length > 0 && <section key={label} className="kv-agent-group">
      <h4>{label}</h4>
      {agents.map(agent =>
        <button key={agent.id} type="button" className="kv-agent-list-row" aria-haspopup="dialog" onClick={() => setInspection({ agentId: agent.id })}>
          <span className="min-w-0 flex-1 truncate font-medium">{agent.displayName || agent.id}</span>
          <AgentStatus state={displayStatus(agent)} zh={zh} />
          <ChevronRight size={14} className="shrink-0 text-[var(--text-muted)]" aria-hidden="true" />
        </button>
      )}
    </section>)}
    {inspection && <SubagentInspectionDialog conversationId={conversationId} initial={inspection} onClose={() => setInspection(null)} />}
  </section>
}

export function SubagentDialog({ title, onClose, onBack, children }: {
  title: string; onClose: () => void; onBack?: () => void; children: ReactNode
}) {
  const zh = useLang() === 'zh'
  const dialog = useRef<HTMLDialogElement>(null)
  const titleId = useId()
  const heading = useRef<HTMLHeadingElement>(null)
  useEffect(() => {
    const node = dialog.current
    if (!node) return
    const trigger = document.activeElement
    node.showModal()
    heading.current?.focus()
    return () => {
      node.close()
      if (trigger instanceof HTMLElement && trigger.isConnected) trigger.focus()
    }
  }, [])
  return createPortal(<dialog ref={dialog} className="kv-modal kv-subagent-dialog" aria-labelledby={titleId}
    onKeyDown={event => event.stopPropagation()}
    onCancel={event => { event.preventDefault(); onClose() }}>
    <div className="kv-agent-panel">
      <header className="kv-agent-panel-bar">
        {onBack && <IconButton variant="ghost" label={zh ? '返回活动' : 'Back to activity'} onClick={onBack}><ArrowLeft size={16} /></IconButton>}
        <h2 id={titleId} tabIndex={-1} ref={heading} className="min-w-0 flex-1 truncate text-sm font-semibold">{title}</h2>
        <IconButton variant="ghost" label={zh ? '关闭' : 'Close'} onClick={onClose}><X size={17} /></IconButton>
      </header>
      {children}
    </div>
  </dialog>, document.body)
}

export function SubagentInspectionDialog({ conversationId, initial, onClose }: { conversationId: string; initial: Inspection; onClose: () => void }) {
  const zh = useLang() === 'zh'
  const [inspection, setInspection] = useState<Inspection>(initial)
  const { roster, error, refresh } = useSubagentRoster(conversationId)
  const selected = inspection !== 'activity' && roster?.agents.find(agent => agent.id === inspection.agentId)
  const parent = selected && roster?.agents.find(agent => agent.id === selected.parentId)
  const title = inspection === 'activity' ? zh ? '子代理活动' : 'Subagent activity'
    : selected ? selected.displayName || selected.id : inspection.agentId
  return <SubagentDialog title={title} onClose={onClose}
    onBack={initial === 'activity' && inspection !== 'activity' ? () => setInspection('activity') : undefined}>
    {initial === 'activity' && <section className="kv-agent-activity-view" hidden={inspection !== 'activity'}>
      <SubagentActivityList conversationId={conversationId} roster={roster} onSelect={entry => setInspection({ agentId: entry.agentId, entry })} />
    </section>}
    {inspection !== 'activity' && (selected
        ? <AgentDetail key={`${selected.id}:${selected.generation}`} conversationId={conversationId} agent={selected}
            parentName={parent ? parent.displayName || parent.id : undefined} onChanged={refresh} anchor={inspection.entry} />
        : <div className="kv-agent-panel-scroll custom-scrollbar">
          {error ? <div role="alert"><p className="mb-2 break-words text-sm">{error}</p><Button size="sm" onClick={refresh}>{zh ? '重试' : 'Retry'}</Button></div>
            : <p role="status" className="text-sm text-[var(--text-muted)]">{roster
              ? zh ? '这个任务的代理会话不可查看。' : 'The agent session is unavailable.'
              : zh ? '正在加载子代理…' : 'Loading subagent…'}</p>}
        </div>)}
  </SubagentDialog>
}

function SubagentActivityList({ conversationId, roster, onSelect }: {
  conversationId: string
  roster: SubagentRoster | null
  onSelect: (entry: SubagentActivity) => void
}) {
  const zh = useLang() === 'zh'
  const [rows, setRows] = useState<SubagentActivity[] | null>(null)
  const [error, setError] = useState('')
  const [revision, setRevision] = useState(0)
  const [query, setQuery] = useState('')
  const [kind, setKind] = useState<SubagentActivity['kind'] | 'all'>('all')
  const labels = {
    response: zh ? '回复' : 'Responses', tool: zh ? '工具' : 'Tools',
    message: zh ? '通信' : 'Messages', lifecycle: zh ? '运行' : 'Runs',
  } satisfies Record<SubagentActivity['kind'], string>
  const kinds = ['all', 'response', 'tool', 'message', 'lifecycle'] as const
  useEffect(() => {
    let active = true
    subagentsApi.activity(conversationId).then(entries => {
      if (active) { setRows(entries); setError('') }
    }).catch(error => { if (active) setError(errorText(error)) })
    return () => { active = false }
  }, [conversationId, revision])
  const names = new Map(roster?.agents.map(agent => [agent.id, agent.displayName || agent.id]))
  const term = query.trim().toLowerCase()
  const visible = rows?.filter(row => (kind === 'all' || row.kind === kind)
    && `${names.get(row.agentId) || row.agentId} ${row.summary}`.toLowerCase().includes(term))
  const lifecycle: Record<string, string> = {
    turn_start: zh ? '开始执行' : 'Run started', turn_end: zh ? '运行结束' : 'Run ended',
    completed: zh ? '已完成' : 'Completed', failed: zh ? '失败' : 'Failed',
    aborted: zh ? '已停止' : 'Stopped', compaction: zh ? '压缩上下文' : 'Context compacted',
  }
  return <>
    <div className="kv-agent-activity-controls">
      <Input value={query} onChange={setQuery} placeholder={zh ? '搜索代理或内容' : 'Search agents or activity'} aria-label={zh ? '搜索活动' : 'Search activity'} />
      <Button size="sm" variant="ghost" onClick={() => setRevision(value => value + 1)}>{zh ? '刷新' : 'Refresh'}</Button>
    </div>
    <nav className="kv-agent-views" aria-label={zh ? '活动类型' : 'Activity type'}>
      {kinds.map(value => <button key={value} type="button" aria-pressed={kind === value} onClick={() => setKind(value)}>
        {value === 'all' ? zh ? '全部' : 'All' : labels[value]}
      </button>)}
    </nav>
    <div className="kv-agent-panel-scroll custom-scrollbar">
      <p className="mb-2 text-xs text-[var(--text-muted)]">{zh ? '最近记录，最多 200 条。点击可定位到原记录。' : 'Up to 200 recent events. Select an event to jump to its transcript.'}</p>
      {error && <p role="alert" className="my-2 break-words text-sm text-[var(--danger)]">{error}</p>}
      {!rows && !error && <p role="status">{zh ? '正在加载活动…' : 'Loading activity…'}</p>}
      {visible?.length === 0 && <p className="py-4 text-sm text-[var(--text-muted)]">{zh ? '没有匹配的活动' : 'No matching activity'}</p>}
      {visible?.map(row => <button type="button" key={`${row.agentId}:${row.entryId}`} className="kv-agent-list-row" onClick={() => onSelect(row)}>
        <span className="min-w-0 flex-1">
          <span className="block truncate font-medium">{names.get(row.agentId) || row.agentId} · {labels[row.kind]}</span>
          <span className="block truncate text-xs text-[var(--text-muted)]">{row.kind === 'lifecycle' ? lifecycle[row.summary] || row.summary : row.summary}</span>
        </span>
        {row.timestamp != null && <time className="shrink-0 text-xs text-[var(--text-muted)]">{new Date(row.timestamp).toLocaleTimeString()}</time>}
        <ChevronRight size={14} aria-hidden="true" />
      </button>)}
    </div>
  </>
}

function displayStatus(agent: SubagentView) {
  if (agent.status === 'running' || agent.status === 'aborted') return agent.status
  return subagentProgress(agent.progress)?.status || agent.status
}

function isConversationMessage(entry: SubagentTranscriptEntry) {
  return entry.kind === 'message' && ['user', 'assistant', 'irc'].includes(entry.role) && Boolean(entry.content.trim())
}

function AgentDetail({ conversationId, agent, parentName, onChanged, anchor }: {
  conversationId: string; agent: SubagentView; parentName?: string; onChanged: () => void; anchor?: SubagentActivity
}) {
  const zh = useLang() === 'zh'
  const t = (cn: string, en: string) => zh ? cn : en
  const [entries, setEntries] = useState<SubagentTranscriptEntry[]>([])
  const [loaded, setLoaded] = useState(false)
  const [view, setView] = useState<'result' | 'conversation' | 'diagnostics'>(
    anchor ? anchor.kind === 'response' || anchor.kind === 'message' ? 'conversation' : 'diagnostics' : 'result',
  )
  const [tailError, setTailError] = useState('')
  const [tailRetry, setTailRetry] = useState(0)
  const [draft, setDraft] = useState('')
  const [operation, setOperation] = useState<string | null>(null)
  const [issue, setIssue] = useState('')
  const [receipt, setReceipt] = useState('')
  const mounted = useRef(true)
  const busy = useRef(false)
  const selectedEntry = useRef<HTMLElement>(null)
  const target = { conversationId, agentId: agent.id, expectedGeneration: agent.generation }
  useEffect(() => { mounted.current = true; return () => { mounted.current = false } }, [])
  useEffect(() => {
    let cancelled = false
    let timer: ReturnType<typeof setTimeout> | undefined
    let cursor: Parameters<typeof subagentsApi.transcript>[1] = null
    async function tail() {
      try {
        const page = await subagentsApi.transcript({ conversationId, agentId: agent.id, expectedGeneration: agent.generation }, cursor)
        if (cancelled || page.id !== agent.id || page.generation !== agent.generation) return
        cursor = page.cursor
        setEntries(previous => {
          const known = new Set(page.reset ? [] : previous.map(entry => entry.id))
          return [...(page.reset ? [] : previous), ...page.entries.filter(entry => !known.has(entry.id))]
        })
        setLoaded(!page.hasMore)
        setTailError('')
        timer = setTimeout(() => void tail(), page.hasMore ? 0 : 500)
      } catch (error) { if (!cancelled) setTailError(errorText(error)) }
    }
    void tail()
    return () => { cancelled = true; clearTimeout(timer) }
  }, [conversationId, agent.id, agent.generation, tailRetry])
  async function act(kind: 'kill' | 'send') {
    if (busy.current) return
    busy.current = true; setOperation(kind); setIssue(''); setReceipt('')
    const sent = draft
    try {
      if (kind === 'send') {
        await subagentsApi.send(target, sent)
        if (mounted.current) { setDraft(current => current === sent ? '' : current); setReceipt(t('消息已受理', 'Message accepted')) }
      } else await subagentsApi.kill(target)
      if (mounted.current) onChanged()
    } catch (error) { if (mounted.current) setIssue(errorText(error)) }
    finally { busy.current = false; if (mounted.current) setOperation(null) }
  }
  const readonly = agent.agentType === 'advisor' || (!agent.revivable && agent.status !== 'running' && agent.status !== 'idle')
  const progress = subagentProgress(agent.progress)
  const conversation = entries.filter(isConversationMessage)
  const diagnostics = entries.filter(entry => !isConversationMessage(entry))
  const anchorAvailable = Boolean(anchor && entries.some(entry => entry.id === anchor.entryId))
  useEffect(() => {
    if (!anchorAvailable || !selectedEntry.current) return
    selectedEntry.current.scrollIntoView?.({ block: 'center' })
    selectedEntry.current.focus({ preventScroll: true })
  }, [anchorAvailable, view])
  const latestAnswer = conversation.filter(entry => entry.role === 'assistant').at(-1)
  const latestResult = latestAnswer?.content || (agent.status !== 'running' ? progress?.preview : null)
  const assignment = conversation.some(entry => entry.role === 'user') ? null
    : progress?.task || entries.find(entry => entry.kind === 'init')?.content.split('\n').slice(1).join('\n')
  const views = [
    { id: 'result', label: t('结果', 'Result') },
    { id: 'conversation', label: t('对话', 'Conversation') },
    { id: 'diagnostics', label: t('诊断', 'Diagnostics') },
  ] as const
  return <section className="kv-agent-detail" aria-label={t('代理详情', 'Agent details')}>
    <div className="kv-agent-detail-heading">
      <div className="min-w-0 space-y-1">
        <AgentStatus state={displayStatus(agent)} zh={zh} />
        <p className="truncate text-xs text-[var(--text-muted)]" title={agent.model ?? undefined}>{[agent.agentType, agent.model].filter(Boolean).join(' · ')}</p>
        {parentName && <p className="truncate text-xs text-[var(--text-muted)]">{t('派工者', 'Assigned by')} {parentName}</p>}
      </div>
      {agent.status === 'running' && !readonly && <Button size="sm" variant="danger" disabled={operation !== null} onClick={() => void act('kill')}>{t('停止', 'Stop')}</Button>}
    </div>
    {agent.reviveError && <p className="px-4 pb-2 break-words text-xs text-[var(--text-muted)]">{agent.reviveError}</p>}
    <nav className="kv-agent-views" aria-label={t('代理详情视图', 'Agent detail views')}>
      {views.map(item => <button key={item.id} type="button" aria-pressed={view === item.id} onClick={() => setView(item.id)}>{item.label}</button>)}
    </nav>
    <div key={view} className="kv-agent-panel-scroll custom-scrollbar" role="region" aria-label={views.find(item => item.id === view)?.label}>
      {view === 'result' && <>
        {(agent.activity || progress) && <div className="kv-agent-activity">
          {agent.activity && agent.activity !== progress?.stage && <p>{agent.activity}</p>}
          <AgentProgress value={agent.progress} expanded={false} zh={zh} />
        </div>}
        {progress?.error && <p className="mb-3 break-words text-xs text-[var(--danger)]">{progress.error}</p>}
        {latestResult ? <TranscriptText label={agent.status === 'running' ? t('最近回复', 'Latest reply') : t('最近结果', 'Latest result')} content={latestResult} />
          : loaded && !tailError && <p className="py-3 text-sm text-[var(--text-muted)]" role="status">
            {agent.status === 'running' ? t('等待子代理回复…', 'Waiting for a reply…') : t('暂无子代理回复', 'No subagent replies yet')}
          </p>}
      </>}
      {view === 'conversation' && <>
        {assignment && <TranscriptText label={t('任务指令', 'Assignment')} content={assignment} />}
        {conversation.map(entry => <TranscriptLine key={entry.id} entry={entry} zh={zh} anchorRef={entry.id === anchor?.entryId ? selectedEntry : undefined} />)}
        {loaded && !assignment && conversation.length === 0 && <p className="text-sm text-[var(--text-muted)]">{t('暂无对话记录', 'No conversation history')}</p>}
      </>}
      {view === 'diagnostics' && diagnostics.map(entry => <TranscriptLine key={entry.id} entry={entry} zh={zh} anchorRef={entry.id === anchor?.entryId ? selectedEntry : undefined} />)}
      {view === 'diagnostics' && loaded && diagnostics.length === 0 && <p className="text-sm text-[var(--text-muted)]">{t('暂无诊断记录', 'No diagnostics yet')}</p>}
      {anchor && loaded && !anchorAvailable && <p role="status" className="mt-2 text-xs text-[var(--text-muted)]">{t('这条活动的原记录已不可用。', 'The original entry is no longer available.')}</p>}
      {!loaded && !tailError && <p role="status" className="text-sm text-[var(--text-muted)]">{t('正在加载记录…', 'Loading activity…')}</p>}
      {tailError && <div role="alert" className="space-y-2 py-3 text-xs"><p>{t('读取历史失败', 'Could not read transcript')}</p><p className="break-words">{tailError}</p><Button size="sm" onClick={() => setTailRetry(value => value + 1)}>{t('重试读取', 'Retry transcript')}</Button></div>}
    </div>
    <footer className="kv-agent-composer">
      {issue && <p role="alert" className="break-words text-xs text-[var(--danger)]">{issue}</p>}
      {readonly ? <p className="text-xs text-[var(--text-muted)]">{t('此代理仅供查看历史', 'This agent is read-only')}</p> : <>
        <TextArea value={draft} onChange={setDraft} aria-label={t('发送给代理的消息', 'Message to agent')}
          placeholder={agent.status === 'running' ? t('在下一步引导子代理…', 'Steer this agent at its next step…') : t('向子代理发送后续指令…', 'Send a follow-up instruction…')} rows={2} maxRows={5} />
        <div className="flex items-center justify-between gap-2">
          <p role="status" className="text-xs text-[var(--text-muted)]">{operation ? t('处理中…', 'Working…') : receipt}</p>
          <Button size="sm" variant="primary" disabled={!draft.trim() || operation !== null} onClick={() => void act('send')}>{t('发送消息', 'Send message')}</Button>
        </div>
      </>}
    </footer>
  </section>
}

function TranscriptLine({ entry, zh, anchorRef }: { entry: SubagentTranscriptEntry; zh: boolean; anchorRef?: React.RefObject<HTMLElement> }) {
  const label = entry.role === 'assistant' ? zh ? '子代理' : 'Subagent'
    : entry.role === 'user' ? zh ? '指令' : 'Instruction'
      : entry.role === 'irc' ? zh ? '通信' : 'Message' : entry.role || entry.kind
  return <TranscriptText label={label} content={entry.content} anchorRef={anchorRef} />
}

function TranscriptText({ label, content, anchorRef }: { label: string; content: string; anchorRef?: React.RefObject<HTMLElement> }) {
  return <article ref={anchorRef} tabIndex={anchorRef ? -1 : undefined} className="kv-agent-answer" data-target={anchorRef ? 'true' : undefined}>
    <h4>{label}</h4>
    <p className="whitespace-pre-wrap [overflow-wrap:anywhere]">{content}</p>
  </article>
}
