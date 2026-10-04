import { useEffect, useRef, useState, type ReactNode } from 'react'
import { api, type SubAgentRecord as Child } from '../api/tauri'
import { ChevronLeft } from 'lucide-react'
import { useSubAgents, refreshSubAgents } from './useSubAgents'
import { Button, IconButton } from '../components/Button'
import { SubAgentAvatar } from './SubAgentAvatar'
import { SubAgentConversation } from './SubAgentConversation'
import { SubAgentElapsed } from './SubAgentElapsed'
import { requestDockSubAgent } from './dock/dockPreview'
import { subAgentStatusLabel } from './subAgentStatus'

const active = (status: string) => ['running', 'finishing', 'stopping'].includes(status)

export function SubAgentIndicator({ conversationId, onOpen, lang = 'zh' }: { conversationId: string; onOpen: () => void; lang?: 'zh' | 'en' }) {
  const { agents } = useSubAgents(conversationId)
  const running = agents.filter(child => active(child.runs.at(-1)?.status ?? ''))
  if (!running.length) return null
  return <div className="ml-auto flex h-5 min-w-0 items-center gap-1">
    <div className="isolate flex h-5 min-w-0 max-w-[min(12rem,55vw)] items-center overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
      {running.map(child => {
        const label = `${child.name} · ${subAgentStatusLabel(child.runs.at(-1), lang)}`
        return <button key={child.id} type="button" aria-label={label} title={label} onClick={() => { onOpen(); requestDockSubAgent({ conversationId, agentId: child.id }) }}
          className="relative flex size-5 shrink-0 items-center justify-center rounded-full first:ml-0 -ml-2.5 hover:z-20 focus-visible:z-20 focus-visible:outline focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-[var(--accent)]">
          <SubAgentAvatar id={child.id} status={child.runs.at(-1)?.status} size={18} />
        </button>
      })}
    </div>
    {running.length > 1 && <span aria-hidden="true" className="text-[10px] leading-none tabular-nums text-neutral-400">{running.length}</span>}
  </div>
}

export function SubAgentPanel({ conversationId, lang = 'zh', revealAgent, listFooter }: { conversationId: string; lang?: 'zh' | 'en'; revealAgent?: { agentId: string; nonce: number } | null; listFooter?: ReactNode }) {
  const t = (zh: string, en: string) => lang === 'zh' ? zh : en
  const { agents: children, error: connectionError } = useSubAgents(conversationId)
  const [selected, setSelected] = useState<Child | null>(null)
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [error, setError] = useState('')
  const epoch = useRef(0)
  useEffect(() => {
    epoch.current += 1
    setSelectedId(null)
    setSelected(null)
    setError('')
  }, [conversationId])
  useEffect(() => {
    if (revealAgent) { setSelected(null); setError(''); setSelectedId(revealAgent.agentId) }
  }, [conversationId, revealAgent])
  const revision = children.find(child => child.id === selectedId)?.sequence
  useEffect(() => {
    if (!selectedId) return
    let disposed = false
    const generation = epoch.current
    void api.chatSubagentControl(conversationId, { operation: 'get', id: selectedId }).then(detail => {
      if (!disposed && epoch.current === generation) { setSelected(detail); setError('') }
    }).catch(err => { if (!disposed) setError(String(err)) })
    return () => { disposed = true }
  }, [conversationId, selectedId, revision])

  const groups = [
    { label: t('正在运行', 'Running'), items: children.filter(child => active(child.runs.at(-1)?.status ?? '')), empty: t('没有正在运行的子代理', 'No running sub-agents') },
    { label: t('历史', 'History'), items: children.filter(child => !active(child.runs.at(-1)?.status ?? '')), empty: t('暂无子代理历史', 'No sub-agent history') },
  ]
  const failure = error || connectionError
  return <section aria-label={t('子代理协作', 'Sub-agent collaboration')} className="flex shrink-0 flex-col text-[13px]">
    {failure && <div className="p-4"><p role="alert" className="break-words text-red-600">{failure}</p><Button size="sm" onClick={() => { setError(''); refreshSubAgents(conversationId); if (selectedId) { setSelectedId(null); setSelected(null) } }}>{t('重新连接', 'Reconnect')}</Button></div>}
    {!selectedId ? <div className="px-3 py-5">
      {groups.map(group => (group.items.length > 0 || group.empty) && <div key={group.label} className="mb-7">
        <h3 className="mb-2 px-2 text-xs font-normal text-neutral-400">{group.label} · {group.items.length}</h3>
        {!group.items.length && <p className="px-2 py-1 text-xs text-neutral-400">{group.empty}</p>}
        {group.items.map(child => <button key={child.id} type="button" onClick={() => { setError(''); setSelected(null); setSelectedId(child.id) }} className="flex w-full items-center gap-3 rounded-lg px-2 py-3 text-left transition-colors hover:bg-neutral-500/5 focus-visible:outline focus-visible:outline-2 focus-visible:outline-[var(--accent)]">
          <SubAgentAvatar id={child.id} status={child.runs.at(-1)?.status} />
          <span className="min-w-0 flex-1 truncate" title={child.name}>{child.name}</span>
          <SubAgentElapsed run={child.runs.at(-1)} lang={lang} />
          <span className="shrink-0 text-[11px] text-neutral-400">{subAgentStatusLabel(child.runs.at(-1), lang)}</span>
        </button>)}
      </div>)}
    </div> : <div className="min-w-0">
      <div className="sticky top-0 z-10 flex items-center gap-2 border-b border-neutral-200 bg-[var(--theme-surface)]/95 px-2 py-3 backdrop-blur">
        <IconButton label={t('返回任务列表', 'Back to tasks')} size="sm" variant="ghost" onClick={() => { setSelectedId(null); setSelected(null) }}><ChevronLeft size={15} /></IconButton>
        <SubAgentAvatar id={selectedId} size={24} />
        <span className="min-w-0 flex-1 truncate font-medium">{selected?.name ?? children.find(child => child.id === selectedId)?.name}</span>
        <span className="max-w-[35%] truncate text-[11px] text-neutral-400" title={subAgentStatusLabel(selected?.runs.at(-1), lang)}>{selected?.profile.model}</span>
      </div>
      {selected ? <SubAgentConversation key={selected.id} child={selected} lang={lang} /> : <p role="status" className="p-5 text-xs text-neutral-400">{t('正在加载对话…', 'Loading conversation…')}</p>}
    </div>}
    {!selectedId && listFooter}
  </section>
}
