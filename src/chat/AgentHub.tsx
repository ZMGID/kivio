import { useEffect, useId, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { RefreshCw, Users, X } from 'lucide-react'
import { Button, IconButton } from '../components/Button'
import { useLang } from '../components/i18n'
import { Input, Select, TextArea } from '../settings/public/controls'
import { subagentsApi, type SubagentRoster, type SubagentTranscriptEntry, type SubagentView } from '../api/subagents'
import { AgentProgress } from './AgentProgress'
import { IrcMessageCard } from './IrcMessageCard'
import { api } from '../api/tauri'
import { useTauriEvent } from './hooks/useTauriEvent'
import type { SubagentIrcMessage } from '../generated/chatProtocol'
import './AgentHub.css'

const EMPTY_ROSTER: SubagentRoster = { agents: [], runningCount: 0 }
const agentKey = (agent: SubagentView) => `${agent.id}:${agent.generation}`
const errorText = (error: unknown) => error instanceof Error ? error.message : String(error)

function useRoster(conversationId: string, enabled = true) {
  const [roster, setRoster] = useState<SubagentRoster>(EMPTY_ROSTER)
  const [error, setError] = useState('')
  const [retry, setRetry] = useState(0)
  useEffect(() => {
    if (!enabled) return
    let cancelled = false
    let timer: ReturnType<typeof setTimeout> | undefined
    async function refresh() {
      try {
        const value = await subagentsApi.snapshot(conversationId)
        if (!cancelled) { setRoster(value); setError('') }
      } catch (error) {
        if (!cancelled) setError(errorText(error))
      }
      if (!cancelled) timer = setTimeout(() => void refresh(), 500)
    }
    void refresh()
    return () => { cancelled = true; clearTimeout(timer) }
  }, [conversationId, enabled, retry])
  return { roster, error, refresh: () => setRetry(value => value + 1) }
}

export function AgentHubEntry({ conversationId }: { conversationId: string }) {
  const zh = useLang() === 'zh'
  const [open, setOpen] = useState(false)
  const { roster } = useRoster(conversationId, !open)
  const label = zh ? `子代理 · ${roster.runningCount} 个运行中 / ${roster.agents.length} 个可查看` : `Sub-agents · ${roster.runningCount} running / ${roster.agents.length} inspectable`
  return <>
    <Button size="sm" variant="ghost" onClick={() => setOpen(true)} aria-haspopup="dialog" title={label} aria-label={label}>
      <Users size={16} aria-hidden="true" />
      <span>{zh ? '子代理' : 'Agents'}{roster.agents.length > 0 ? ` ${roster.runningCount}/${roster.agents.length}` : ''}</span>
    </Button>
    {open && <AgentHub key={conversationId} conversationId={conversationId} onClose={() => setOpen(false)} />}
  </>
}

export function AgentHub(props: { conversationId: string; initialAgentId?: string; onClose: () => void }) {
  return <AgentHubBody key={props.conversationId} {...props} />
}

function AgentHubBody({ conversationId, initialAgentId, onClose }: { conversationId: string; initialAgentId?: string; onClose: () => void }) {
  const zh = useLang() === 'zh'
  const t = (cn: string, en: string) => zh ? cn : en
  const { roster, error, refresh } = useRoster(conversationId)
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [section, setSection] = useState('agents')
  const [traffic, setTraffic] = useState<SubagentIrcMessage[]>([])
  const [query, setQuery] = useState('')
  const [status, setStatus] = useState('all')
  const [layout, setLayout] = useState('tree')
  const [restoreError, setRestoreError] = useState('')
  const [restoring, setRestoring] = useState(false)
  const mounted = useRef(true)
  const ranks = useRef(new Map<string, number>())
  const dialog = useRef<HTMLDialogElement>(null)
  const pressedBackdrop = useRef(false)
  const titleId = useId()
  useTauriEvent(api.onChatSubagentIrc, message => {
    if (message.conversationId !== conversationId) return
    // shortcut: retain 200 observations per open Hub; add paging if session traffic needs a longer view.
    setTraffic(previous => previous.some(item => item.id === message.id) ? previous : [...previous, message].slice(-200))
  }, [conversationId])
  useEffect(() => {
    mounted.current = true
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null
    dialog.current?.showModal()
    return () => { mounted.current = false; if (previous?.isConnected) previous.focus() }
  }, [])
  const candidates = [...roster.agents].sort((a, b) => Number(b.status === 'running') - Number(a.status === 'running') || b.lastActivity - a.lastActivity || a.id.localeCompare(b.id))
  for (const agent of candidates) if (!ranks.current.has(agentKey(agent))) ranks.current.set(agentKey(agent), ranks.current.size)
  const ordered = [...roster.agents].sort((a, b) => (ranks.current.get(agentKey(a)) ?? 0) - (ranks.current.get(agentKey(b)) ?? 0))
  const selected = ordered.find(agent => agent.id === selectedId)
    ?? ordered.find(agent => agent.id === initialAgentId)
    ?? ordered[0]
  const visible = ordered.filter(agent => (status === 'all' || agent.status === status) && `${agent.displayName} ${agent.id} ${agent.agentType} ${agent.activity ?? ''}`.toLowerCase().includes(query.toLowerCase()))
  const sorted = layout === 'tree' ? treeOrder(visible) : visible
  async function restore() {
    setRestoring(true); setRestoreError('')
    try { await subagentsApi.restore(conversationId); if (mounted.current) refresh() }
    catch (error) { if (mounted.current) setRestoreError(errorText(error)) }
    finally { if (mounted.current) setRestoring(false) }
  }
  return createPortal(<dialog ref={dialog} className="kv kv-modal kv-agent-hub" aria-labelledby={titleId}
    onCancel={event => { event.preventDefault(); onClose() }}
    onMouseDown={event => { pressedBackdrop.current = event.target === event.currentTarget }}
    onClick={event => { if (pressedBackdrop.current && event.target === event.currentTarget) onClose(); pressedBackdrop.current = false }}>
    <header className="flex items-center justify-between gap-3 border-b border-[var(--color-border)] px-5 py-4">
      <div><h2 id={titleId} className="text-[15px] font-semibold">{t('子代理', 'Agent Hub')}</h2><p className="text-xs text-[var(--text-muted)]">{t(`${roster.runningCount} 个运行中 · ${roster.agents.length} 个可查看`, `${roster.runningCount} running · ${roster.agents.length} inspectable`)}</p></div>
      <div className="flex items-center gap-2"><Select value={section} onChange={setSection} ariaLabel={t('子代理视图', 'Agent Hub view')} options={[{ value: 'agents', label: t('代理', 'Agents') }, { value: 'activity', label: t('通信', 'Activity') }]} /><IconButton label={t('刷新代理', 'Refresh agents')} variant="ghost" onClick={refresh}><RefreshCw size={16} /></IconButton><IconButton label={t('关闭子代理', 'Close Agent Hub')} variant="ghost" onClick={onClose}><X size={18} /></IconButton></div>
    </header>
    <div className="kv-agent-hub-body custom-scrollbar">
      <aside className="kv-agent-hub-roster">
        <div className="space-y-2 border-b border-[var(--color-border)] p-3">
          <Input value={query} onChange={setQuery} placeholder={t('查找代理', 'Find an agent')} aria-label={t('查找代理', 'Find an agent')} />
          <div className="flex gap-2"><Select value={status} onChange={setStatus} ariaLabel={t('代理状态筛选', 'Filter agent status')} options={[
            { value: 'all', label: t('全部状态', 'All statuses') }, ...(['running', 'idle', 'parked', 'aborted']).map(value => ({ value, label: identityLabel(value, zh) })),
          ]} /><Select value={layout} onChange={setLayout} ariaLabel={t('代理排列', 'Agent layout')} options={[{ value: 'tree', label: t('父子树', 'Parent tree') }, { value: 'flat', label: t('平面列表', 'Flat list') }]} /></div>
        </div>
        {error && <div role="alert" className="px-3 py-2 text-xs">{t('读取代理失败', 'Could not load agents')}<p className="break-words text-[var(--text-muted)]">{error}</p><Button size="sm" onClick={refresh}>{t('重试', 'Retry')}</Button></div>}
        <nav className="min-h-0 flex-1 overflow-auto p-2 custom-scrollbar" aria-label={t('代理列表', 'Agent list')}>
          {sorted.map(agent => <button type="button" key={agentKey(agent)} aria-current={selected && agentKey(selected) === agentKey(agent) ? 'true' : undefined}
            className="kv-agent-hub-row" onClick={() => { setSelectedId(agent.id); setSection('agents') }}>
              <div style={{ paddingInlineStart: layout === 'tree' ? Math.max(0, Math.min(agent.depth - 1, 6)) * 12 : 0 }}>
              <div className="flex items-center justify-between gap-2"><span className="truncate font-medium">{agent.displayName || agent.id}</span><span className="shrink-0 text-[11px] text-[var(--text-muted)]">{identityLabel(agent.status, zh)}</span></div>
              <p className="truncate text-xs text-[var(--text-muted)]">{agent.agentType} · {agent.id} · g{agent.generation}</p>
              {agent.activity && <p className="truncate text-xs text-[var(--text-muted)]">{agent.activity}</p>}
            </div>
          </button>)}
          {sorted.length === 0 && <p className="p-3 text-xs text-[var(--text-muted)]">{t('没有匹配的子代理', 'No matching sub-agents')}</p>}
        </nav>
        <footer className="space-y-2 border-t border-[var(--color-border)] p-3">
          <Button size="sm" onClick={() => void restore()} disabled={restoring}>{restoring ? t('恢复中…', 'Restoring…') : t('恢复中断运行', 'Restore interrupted runs')}</Button>
          {restoreError && <p role="alert" className="break-words text-xs">{restoreError}</p>}
        </footer>
      </aside>
      {section === 'activity' ? <section className="min-w-0 flex-1 overflow-auto p-4 custom-scrollbar" aria-label={t('代理通信', 'Agent activity')}><p className="mb-3 text-xs text-[var(--text-muted)]">{t('显示此面板打开期间最近 200 条通信；旁观记录不会加入主代理上下文。', 'Latest 200 messages observed while this panel is open. Observations do not enter Main context.')}</p>{traffic.map(message => <IrcMessageCard key={message.id} message={message} />)}{traffic.length === 0 && <p className="text-xs text-[var(--text-muted)]">{t('暂无通信', 'No messages observed yet')}</p>}</section> : selected ? <AgentDetail key={`${conversationId}:${agentKey(selected)}`} conversationId={conversationId} agent={selected} onChanged={refresh} />
        : <div className="flex flex-1 items-center justify-center p-8 text-sm text-[var(--text-muted)]">{t('派工后可在此查看子代理及历史', 'Delegated agents and their history appear here')}</div>}
    </div>
  </dialog>, document.body)
}

function treeOrder(agents: SubagentView[]): SubagentView[] {
  const seen = new Set<string>()
  const result: SubagentView[] = []
  function visit(agent: SubagentView) {
    if (seen.has(agentKey(agent))) return
    seen.add(agentKey(agent)); result.push(agent)
    for (const child of agents) if (child.parentId === agent.id) visit(child)
  }
  for (const agent of agents) if (!agents.some(parent => parent.id === agent.parentId)) visit(agent)
  for (const agent of agents) visit(agent)
  return result
}

function identityLabel(status: string, zh: boolean) {
  switch (status) {
    case 'running': return zh ? '运行中' : 'Running'
    case 'idle': return zh ? '空闲' : 'Idle'
    case 'parked': return zh ? '已释放' : 'Parked'
    case 'aborted': return zh ? '已终止' : 'Killed'
    default: return status
  }
}

function AgentDetail({ conversationId, agent, onChanged }: { conversationId: string; agent: SubagentView; onChanged: () => void }) {
  const zh = useLang() === 'zh'
  const t = (cn: string, en: string) => zh ? cn : en
  const [entries, setEntries] = useState<SubagentTranscriptEntry[]>([])
  const [tailError, setTailError] = useState('')
  const [tailRetry, setTailRetry] = useState(0)
  const [draft, setDraft] = useState('')
  const [operation, setOperation] = useState<string | null>(null)
  const [issue, setIssue] = useState('')
  const [receipt, setReceipt] = useState('')
  const mounted = useRef(true)
  const busy = useRef(false)
  const transcript = useRef<HTMLDivElement>(null)
  const followsTail = useRef(true)
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
        setTailError('')
        timer = setTimeout(() => void tail(), page.hasMore ? 0 : 500)
      } catch (error) { if (!cancelled) setTailError(errorText(error)) }
    }
    void tail()
    return () => { cancelled = true; clearTimeout(timer) }
  }, [conversationId, agent.id, agent.generation, tailRetry])
  useEffect(() => { if (followsTail.current && transcript.current) transcript.current.scrollTop = transcript.current.scrollHeight }, [entries])
  async function act(kind: 'open' | 'park' | 'kill' | 'send') {
    if (busy.current) return
    busy.current = true; setOperation(kind); setIssue(''); setReceipt('')
    const sent = draft
    try {
      if (kind === 'send') {
        const result = await subagentsApi.send(target, sent)
        if (mounted.current) { setDraft(current => current === sent ? '' : current); setReceipt(`${t('消息已受理', 'Message accepted')} · ${result.status}`) }
      } else await subagentsApi[kind](target)
      if (mounted.current) onChanged()
    } catch (error) { if (mounted.current) setIssue(errorText(error)) }
    finally { busy.current = false; if (mounted.current) setOperation(null) }
  }
  const readonly = agent.agentType === 'advisor' || (!agent.revivable && agent.status !== 'running' && agent.status !== 'idle')
  return <section className="kv-agent-hub-detail" aria-label={t('代理详情', 'Agent details')}>
    <header className="space-y-2 border-b border-[var(--color-border)] px-4 py-3">
      <div className="flex flex-wrap items-start justify-between gap-2"><div className="min-w-0"><h3 className="break-words text-sm font-semibold">{agent.displayName || agent.id}</h3><p className="break-all text-xs text-[var(--text-muted)]">{agent.id} · g{agent.generation} · {identityLabel(agent.status, zh)} · {agent.agentType}</p></div>
        <div className="flex flex-wrap gap-2">
          <Button size="sm" disabled={operation !== null || readonly} onClick={() => void act('open')}>{agent.status === 'parked' || agent.status === 'aborted' ? t('复活', 'Revive') : t('打开会话', 'Open session')}</Button>
          <Button size="sm" disabled={operation !== null || agent.status !== 'idle'} onClick={() => void act('park')}>{t('释放会话', 'Park')}</Button>
          <Button size="sm" variant="danger" disabled={operation !== null || agent.status === 'aborted'} onClick={() => void act('kill')}>{t('终止', 'Kill')}</Button>
        </div>
      </div>
      {agent.model && <p className="break-words font-mono text-xs text-[var(--text-muted)]">{agent.model}</p>}
      {agent.activity && <p className="break-words text-xs">{agent.activity}</p>}
      <AgentProgress value={agent.progress} expanded={false} zh={zh} />
      {agent.reviveError && <p className="break-words text-xs">{agent.reviveError}</p>}
      {issue && <div role="alert" className="break-words text-xs">{issue}<p>{t('请重试操作；未发送的文字已保留。', 'Retry the operation. Unsent text is preserved.')}</p></div>}
      {operation && <p role="status" className="text-xs text-[var(--text-muted)]">{t('操作中…', 'Working…')}</p>}
    </header>
    <div ref={transcript} className="kv-agent-hub-transcript custom-scrollbar" aria-label={t('代理历史', 'Agent transcript')} onScroll={event => { const node = event.currentTarget; followsTail.current = node.scrollHeight - node.scrollTop - node.clientHeight < 60 }}>
      {entries.map(entry => <article key={entry.id} className="space-y-1 border-b border-[var(--color-border)] py-3">
        <div className="flex items-center justify-between gap-2 text-xs text-[var(--text-muted)]"><span>{entry.kind === 'compaction' ? t('上下文压缩', 'Context compacted') : entry.role || entry.kind}</span>{entry.timestamp != null && <time>{new Date(entry.timestamp).toLocaleTimeString()}</time>}</div>
        <pre className="whitespace-pre-wrap break-words font-sans text-[13px] leading-6">{entry.content}</pre>
      </article>)}
      {entries.length === 0 && !tailError && <p className="py-6 text-xs text-[var(--text-muted)]">{t('暂无历史记录', 'No transcript entries yet')}</p>}
      {tailError && <div role="alert" className="space-y-2 py-3 text-xs"><p>{t('读取历史失败', 'Could not read transcript')}</p><p className="break-words">{tailError}</p><Button size="sm" onClick={() => setTailRetry(value => value + 1)}>{t('重试读取', 'Retry transcript')}</Button></div>}
    </div>
    <footer className="space-y-2 border-t border-[var(--color-border)] p-3">
      {readonly ? <p className="text-xs text-[var(--text-muted)]">{t('此代理仅供查看历史', 'This agent is read-only')}</p> : <>
        <TextArea value={draft} onChange={setDraft} aria-label={t('发送给代理的消息', 'Message to agent')} placeholder={t('消息会唤醒空闲代理；运行中则在步骤边界接收。', 'Messages wake idle agents; running agents receive them at a step boundary.')} rows={2} maxRows={5} />
        <div className="flex items-center justify-between gap-2"><p role="status" className="text-xs text-[var(--text-muted)]">{receipt}</p><Button size="sm" variant="primary" disabled={!draft.trim() || operation !== null} onClick={() => void act('send')}>{t('发送消息', 'Send message')}</Button></div>
      </>}
    </footer>
  </section>
}
