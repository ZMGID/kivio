import { useEffect, useRef, useState } from 'react'
import { api, type SubAgentRecord as Child } from '../api/tauri'
import { ChevronLeft } from 'lucide-react'
import { useSubAgents, refreshSubAgents } from './useSubAgents'
import { Button, IconButton } from '../components/Button'
import { SubAgentAvatar } from './SubAgentAvatar'
import { SubAgentConversation } from './SubAgentConversation'

const active = (status: string) => ['running', 'finishing', 'stopping'].includes(status)
const chineseLabels: Record<string, string> = { running: '运行中', finishing: '正在收尾', stopping: '正在停止', completed: '已完成', failed: '失败', interrupted: '已中断' }
export type MainAgentTask = { model: string; running: boolean; onStop: () => void }

export function SubAgentIndicator({ conversationId, onOpen, lang = 'zh' }: { conversationId: string; onOpen: () => void; lang?: 'zh' | 'en' }) {
  const { agents, error } = useSubAgents(conversationId)
  if (!agents.length && !error) return null
  const label = lang === 'zh' ? `子代理 ${agents.length} · 打开任务` : `Sub-agents ${agents.length} · Open tasks`
  return <Button variant="ghost" size="sm" aria-label={label} title={label} onClick={onOpen}><SubAgentAvatar id="main" size={18} /><span className="tabular-nums">{error ? '!' : agents.length}</span></Button>
}

export function SubAgentPanel({ conversationId, lang = 'zh', mainAgent }: { conversationId: string; lang?: 'zh' | 'en'; mainAgent?: MainAgentTask }) {
  const t = (zh: string, en: string) => lang === 'zh' ? zh : en
  const labels = lang === 'zh' ? chineseLabels : { running: 'Running', finishing: 'Finishing', stopping: 'Stopping', completed: 'Completed', failed: 'Failed', interrupted: 'Interrupted' } as Record<string, string>
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
    { label: t('已开启', 'Active'), items: children.filter(child => active(child.runs.at(-1)?.status ?? '')), empty: t('没有已开启的子代理', 'No active sub-agents') },
    { label: t('完成', 'Completed'), items: children.filter(child => child.runs.at(-1)?.status === 'completed') },
    { label: t('已中断 / 失败', 'Interrupted / failed'), items: children.filter(child => !active(child.runs.at(-1)?.status ?? '') && child.runs.at(-1)?.status !== 'completed') },
  ]
  const failure = error || connectionError
  return <section aria-label={t('子代理协作', 'Sub-agent collaboration')} className="flex shrink-0 flex-col text-[13px]">
    {failure && <div className="p-4"><p role="alert" className="break-words text-red-600">{failure}</p><Button size="sm" onClick={() => { setError(''); refreshSubAgents(conversationId); if (selectedId) { setSelectedId(null); setSelected(null) } }}>{t('重新连接', 'Reconnect')}</Button></div>}
    {!selectedId ? <div className="px-3 py-5">
      {mainAgent && <div className="mb-6 flex items-center gap-2 px-2 text-xs text-neutral-500"><SubAgentAvatar id="main" size={22} /><span>{t('主代理', 'Main agent')}</span><span className="min-w-0 flex-1 truncate">{mainAgent.model}</span><span>{mainAgent.running ? t('运行中', 'Running') : t('空闲', 'Idle')}</span></div>}
      {groups.map(group => (group.items.length > 0 || group.empty) && <div key={group.label} className="mb-7">
        <h3 className="mb-2 px-2 text-xs font-normal text-neutral-400">{group.label} · {group.items.length}</h3>
        {!group.items.length && <p className="px-2 py-1 text-xs text-neutral-400">{group.empty}</p>}
        {group.items.map(child => <button key={child.id} type="button" onClick={() => { setError(''); setSelected(null); setSelectedId(child.id) }} className="flex w-full items-center gap-3 rounded-lg px-2 py-3 text-left transition-colors hover:bg-neutral-500/5 focus-visible:outline focus-visible:outline-2 focus-visible:outline-blue-500">
          <SubAgentAvatar id={child.id} status={child.runs.at(-1)?.status} />
          <span className="min-w-0 flex-1 truncate" title={child.name}>{child.name}</span>
          <span className="shrink-0 text-[11px] text-neutral-400">{labels[child.runs.at(-1)?.status ?? ''] ?? t('未知', 'Unknown')}</span>
        </button>)}
      </div>)}
    </div> : <div className="min-w-0">
      <div className="sticky top-0 z-10 flex items-center gap-2 border-b border-neutral-200 bg-white/95 px-2 py-3 backdrop-blur dark:border-neutral-800 dark:bg-neutral-900/95">
        <IconButton label={t('返回任务列表', 'Back to tasks')} size="sm" variant="ghost" onClick={() => { setSelectedId(null); setSelected(null) }}><ChevronLeft size={15} /></IconButton>
        <SubAgentAvatar id={selectedId} size={24} />
        <span className="min-w-0 flex-1 truncate font-medium">{selected?.name ?? children.find(child => child.id === selectedId)?.name}</span>
        <span className="max-w-[35%] truncate text-[11px] text-neutral-400" title={labels[selected?.runs.at(-1)?.status ?? '']}>{selected?.profile.model}</span>
      </div>
      {selected ? <SubAgentConversation key={selected.id} child={selected} lang={lang} /> : <p role="status" className="p-5 text-xs text-neutral-400">{t('正在加载对话…', 'Loading conversation…')}</p>}
    </div>}
  </section>
}
