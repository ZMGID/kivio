import { useEffect, useId, useLayoutEffect, useRef, useState, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { Activity, ArrowLeft, ChevronRight, Users, Wrench, X } from 'lucide-react'
import { Button, IconButton } from '../components/Button'
import { useLang } from '../components/i18n'
import { Input } from '../settings/public/controls'
import { subagentProgress, subagentsApi, type SubagentActivity, type SubagentRoster, type SubagentTranscriptEntry, type SubagentView } from '../api/subagents'
import { AgentStatus } from './AgentProgress'
import { useSubagentRoster } from './useSubagentRoster'
import { ChatMarkdown } from './ChatMarkdown'
import { requestDockSubagent, type DockSubagent } from './dock/dockPreview'
import './AgentHub.css'

const errorText = (error: unknown) => error instanceof Error ? error.message : String(error)

export function SubagentTaskList({ conversationId, roster }: { conversationId: string; roster: SubagentRoster }) {
  const zh = useLang() === 'zh'
  const [showActivity, setShowActivity] = useState(false)
  if (!roster.agents.length) return null
  const active = roster.agents.filter(agent => agent.status === 'running')
  const finished = roster.agents.filter(agent => agent.status !== 'running')
  return <section className="kv-agent-task-list" aria-label={zh ? '子代理任务' : 'Subagent tasks'}>
    <header className="flex items-center gap-2 px-2 py-1.5">
      <Users size={15} aria-hidden="true" />
      <h3 className="min-w-0 flex-1 text-xs font-semibold">{zh ? '子代理' : 'Subagents'}</h3>
      <IconButton variant="ghost" aria-haspopup="dialog" label={zh ? '查看子代理活动' : 'View subagent activity'} onClick={() => setShowActivity(true)}><Activity size={15} /></IconButton>
    </header>
    {([
      [zh ? '进行中' : 'Active', active],
      [zh ? '历史' : 'History', finished],
    ] satisfies [string, SubagentView[]][]).map(([label, agents]) => agents.length > 0 && <section key={label} className="kv-agent-group">
      <h4>{label}</h4>
      {agents.map(agent =>
        <button key={agent.id} type="button" className="kv-agent-list-row" onClick={() => requestDockSubagent({ conversationId, agentId: agent.id })}>
          <span className="min-w-0 flex-1 truncate font-medium">{agent.displayName || agent.id}</span>
          <AgentStatus state={displayStatus(agent)} zh={zh} />
          <ChevronRight size={14} className="shrink-0 text-[var(--text-muted)]" aria-hidden="true" />
        </button>
      )}
    </section>)}
    {showActivity && <SubagentDialog title={zh ? '子代理活动' : 'Subagent activity'} onClose={() => setShowActivity(false)}>
      <SubagentActivityList conversationId={conversationId} roster={roster} onSelect={entry => {
        setShowActivity(false)
        requestDockSubagent({ conversationId, agentId: entry.agentId, entryId: entry.entryId })
      }} />
    </SubagentDialog>}
  </section>
}

export function SubagentDialog({ title, onClose, children }: {
  title: string; onClose: () => void; children: ReactNode
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
        <h2 id={titleId} tabIndex={-1} ref={heading} className="min-w-0 flex-1 truncate text-sm font-semibold">{title}</h2>
        <IconButton variant="ghost" label={zh ? '关闭' : 'Close'} onClick={onClose}><X size={17} /></IconButton>
      </header>
      {children}
    </div>
  </dialog>, document.body)
}

export function SubagentSidebar({ target, onBack }: { target: DockSubagent; onBack?: () => void }) {
  const zh = useLang() === 'zh'
  const { conversationId, agentId, entryId } = target
  const { roster, error, refresh } = useSubagentRoster(conversationId)
  const selected = roster?.agents.find(agent => agent.id === agentId)
  const titleId = useId()
  const title = useRef<HTMLHeadingElement>(null)
  useLayoutEffect(() => {
    const trigger = document.activeElement
    title.current?.focus({ preventScroll: true })
    return () => {
      if (trigger instanceof HTMLElement && trigger.isConnected) trigger.focus({ preventScroll: true })
    }
  }, [])
  return <section className="kv-agent-panel" aria-labelledby={titleId}>
    <header className="kv-agent-panel-bar">
      {onBack && <IconButton variant="ghost" label={zh ? '返回' : 'Back'} onClick={onBack}><ArrowLeft size={16} /></IconButton>}
      <h2 id={titleId} ref={title} tabIndex={-1} className="min-w-0 flex-1 truncate text-sm font-semibold">{selected?.displayName || agentId}</h2>
      {selected && <AgentStatus state={displayStatus(selected)} zh={zh} />}
    </header>
    {selected
      ? <AgentTranscript key={selected.generation} conversationId={conversationId} agent={selected} entryId={entryId} />
      : <div className="kv-agent-panel-scroll custom-scrollbar text-sm">
        {error ? <div role="alert"><p className="mb-2 break-words">{error}</p><Button size="sm" onClick={refresh}>{zh ? '重试' : 'Retry'}</Button></div>
          : <p role="status" className="text-[var(--text-muted)]">{roster
            ? zh ? '没有可查看的对话' : 'No conversation available'
            : zh ? '正在加载…' : 'Loading…'}</p>}
      </div>}
  </section>
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
  if (entry.kind !== 'message' || !['user', 'assistant', 'irc'].includes(entry.role)) return false
  const content = entry.content.trim()
  return Boolean(content) && (entry.role !== 'user' || !/^(<kivio_runtime_context>|<system-reminder>)/.test(content))
}

function AgentTranscript({ conversationId, agent, entryId }: {
  conversationId: string; agent: SubagentView; entryId?: string
}) {
  const zh = useLang() === 'zh'
  const [entries, setEntries] = useState<SubagentTranscriptEntry[]>([])
  const [loaded, setLoaded] = useState(false)
  const [error, setError] = useState('')
  const [retry, setRetry] = useState(0)
  const scroll = useRef<HTMLDivElement>(null)
  const follow = useRef(true)
  const anchor = useRef<HTMLElement>(null)
  const anchorAvailable = entries.some(entry => entry.id === entryId && (isConversationMessage(entry) || transcriptTool(entry)))
  const { id, generation } = agent
  useEffect(() => {
    let cancelled = false
    let timer: ReturnType<typeof setTimeout> | undefined
    let cursor: Parameters<typeof subagentsApi.transcript>[1] = null
    async function tail() {
      try {
        const page = await subagentsApi.transcript({ conversationId, agentId: id, expectedGeneration: generation }, cursor)
        if (cancelled || page.id !== id || page.generation !== generation) return
        cursor = page.cursor
        if (page.reset || page.entries.length) {
          setEntries(previous => {
            const retained = page.reset ? [] : previous
            const known = new Set(retained.map(entry => entry.id))
            return [...retained, ...page.entries.filter(entry => !known.has(entry.id))]
          })
        }
        setLoaded(!page.hasMore)
        setError('')
        timer = setTimeout(() => void tail(), page.hasMore ? 0 : 500)
      } catch (error) {
        if (!cancelled) setError(errorText(error))
      }
    }
    void tail()
    return () => { cancelled = true; clearTimeout(timer) }
  }, [conversationId, id, generation, retry])
  useEffect(() => {
    if (!anchorAvailable || !anchor.current) return
    follow.current = false
    anchor.current.scrollIntoView?.({ block: 'center' })
    anchor.current.focus({ preventScroll: true })
  }, [anchorAvailable, entryId])
  useLayoutEffect(() => {
    const node = scroll.current
    if (node && follow.current && !anchorAvailable) node.scrollTop = node.scrollHeight
  }, [entries, anchorAvailable])

  const conversation = entries.filter(entry => isConversationMessage(entry) || transcriptTool(entry))
  const assignment = conversation.some(entry => entry.role === 'user') ? null
    : entries.find(entry => entry.kind === 'init')?.content.split('\n').slice(1).join('\n')
  return <div ref={scroll} className="kv-agent-panel-scroll custom-scrollbar space-y-4" onScroll={event => {
    const node = event.currentTarget
    follow.current = node.scrollHeight - node.scrollTop - node.clientHeight < 48
  }}>
    {assignment?.trim() && <div className="kv-agent-chat-user"><ChatMarkdown content={assignment} /></div>}
    {conversation.map(entry => {
      const tool = transcriptTool(entry)
      return <article key={entry.id} ref={entry.id === entryId ? anchor : undefined}
        tabIndex={entry.id === entryId ? -1 : undefined}
        className={tool ? 'flex items-center gap-2 break-all text-xs text-[var(--text-muted)]'
          : entry.role === 'user' ? 'kv-agent-chat-user' : 'min-w-0'}>
        {tool ? <><Wrench size={13} className="shrink-0" aria-hidden="true" />{tool}</>
          : <ChatMarkdown content={entry.role === 'user' ? entry.content.replace(/^Complete assignment thoroughly:\s+/, '') : entry.content} />}
      </article>
    })}
    {!error && (!loaded || (conversation.length === 0 && !assignment?.trim())) && <p role="status" className="text-sm text-[var(--text-muted)]">
      {!loaded ? zh ? '正在加载…' : 'Loading…'
        : agent.status === 'running' ? zh ? '等待子代理回复…' : 'Waiting for a reply…'
          : zh ? '暂无对话记录' : 'No conversation history'}
    </p>}
    {error && <div role="alert" className="space-y-2 py-3 text-xs"><p className="break-words">{error}</p>
      <Button size="sm" onClick={() => setRetry(value => value + 1)}>{zh ? '重试读取' : 'Retry transcript'}</Button>
    </div>}
  </div>
}

function transcriptTool(entry: SubagentTranscriptEntry): string {
  if (entry.role !== 'tool' || !/-tool-\d+$/.test(entry.id)) return ''
  const name = entry.content.split('\n', 1)[0]
  return /^[\w:.-]+$/.test(name) ? name.replace(/^native__/, '') : ''
}
