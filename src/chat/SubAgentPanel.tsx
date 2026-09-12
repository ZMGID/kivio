import { useEffect, useRef, useState } from 'react'
import { api, type SubAgentRecord as Child } from '../api/tauri'
import { Bot, ChevronLeft, ChevronRight, Users } from 'lucide-react'
import { useSubAgents, refreshSubAgents, updateSubAgent } from './useSubAgents'
import { ChatMarkdown } from './ChatMarkdown'
import { Button, IconButton } from '../components/Button'
import { TextArea } from '../settings/components'

const active = (status: string) => ['running', 'finishing', 'stopping'].includes(status)
const chineseLabels: Record<string, string> = { running: '运行中', finishing: '正在收尾', stopping: '正在停止', completed: '已完成', failed: '失败', interrupted: '已中断' }

export type MainAgentTask = { model: string; running: boolean; onStop: () => void }

export function SubAgentIndicator({ conversationId, onOpen, lang = 'zh' }: { conversationId: string; onOpen: () => void; lang?: 'zh' | 'en' }) {
  const { agents, error } = useSubAgents(conversationId)
  if (!agents.length && !error) return null
  const label = lang === 'zh' ? `子代理 ${agents.length} · 打开任务` : `Sub-agents ${agents.length} · Open tasks`
  return <Button variant="ghost" size="sm" aria-label={label} title={label} onClick={onOpen}><Users size={14} /><span className="tabular-nums">{error ? '!' : agents.length}</span></Button>
}

export function SubAgentPanel({ conversationId, lang = 'zh', mainAgent }: { conversationId: string; lang?: 'zh' | 'en'; mainAgent?: MainAgentTask }) {
  const t = (zh: string, en: string) => lang === 'zh' ? zh : en
  const labels = lang === 'zh' ? chineseLabels : { running: 'Running', finishing: 'Finishing', stopping: 'Stopping', completed: 'Completed', failed: 'Failed', interrupted: 'Interrupted' } as Record<string, string>
  const { agents: children, error: connectionError } = useSubAgents(conversationId)
  const [selected, setSelected] = useState<Child | null>(null)
  const [text, setText] = useState('')
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const selectedId = useRef<string | null>(null)
  const epoch = useRef(0)
  const submission = useRef<{ fingerprint: string; id: string } | null>(null)

  useEffect(() => {
    const generation = ++epoch.current
    selectedId.current = null
    setSelected(null)
    setError('')
    setText('')
    setBusy(false)
    submission.current = null
    return () => { epoch.current = generation + 1 }
  }, [conversationId])

  const revision = children.find(child => child.id === selectedId.current)?.sequence
  useEffect(() => {
    const id = selectedId.current
    if (!id) return
    let disposed = false
    void api.chatSubagentControl(conversationId, { operation: 'get', id }).then(detail => {
      if (!disposed && selectedId.current === id) setSelected(detail)
    }).catch(err => { if (!disposed) setError(String(err)) })
    return () => { disposed = true }
  }, [conversationId, revision])

  const select = async (child: Child) => {
    selectedId.current = child.id
    const generation = epoch.current
    try {
      const detail = await api.chatSubagentControl(conversationId, { operation: 'get', id: child.id })
      if (generation === epoch.current && selectedId.current === child.id) setSelected(detail)
    } catch (err) { if (generation === epoch.current) setError(String(err)) }
  }

  const operate = async (operation: 'message' | 'continue' | 'stop') => {
    if (!selected) return
    const generation = epoch.current
    setBusy(true)
    setError('')
    try {
      const fingerprint = JSON.stringify([selected.id, operation, text, selected.runs.at(-1)?.id])
      if (submission.current?.fingerprint !== fingerprint) submission.current = { fingerprint, id: crypto.randomUUID() }
      const detail = await api.chatSubagentControl(conversationId, {
        operation, id: selected.id, execution_id: selected.runs.at(-1)?.id,
        message_id: submission.current.id, message: text,
      })
      if (generation === epoch.current && selectedId.current === detail.id) {
        setSelected(detail)
        updateSubAgent(conversationId, detail)
        if (operation !== 'stop') setText('')
        submission.current = null
      }
    } catch (err) { if (generation === epoch.current) setError(String(err)) }
    finally { if (generation === epoch.current) setBusy(false) }
  }

  const run = selected?.runs.at(-1)
  const failure = error || connectionError
  return (
    <section aria-label={t('子代理协作', 'Sub-agent collaboration')} className="flex shrink-0 flex-col text-[13px]">
      {failure && <div className="p-3"><p role="alert" className="break-words text-red-600">{failure}</p><Button size="sm" onClick={() => { setError(''); refreshSubAgents(conversationId) }}>{t('重新连接', 'Reconnect')}</Button></div>}
      {!selected && <div className="space-y-1 p-2">
        {mainAgent && <div className="mb-3 rounded-lg border border-neutral-200 p-3 dark:border-neutral-700">
          <div className="flex items-center gap-2"><Bot size={15} /><span className="font-medium">{t('主代理', 'Main agent')}</span><span className="ml-auto text-xs text-neutral-500">{mainAgent.running ? t('运行中', 'Running') : t('空闲', 'Idle')}</span></div>
          <div className="mt-1 truncate text-xs text-neutral-500">{mainAgent.model}</div>
          {mainAgent.running && <Button size="sm" variant="ghost" className="mt-2" onClick={mainAgent.onStop}>{t('停止整个协作', 'Stop collaboration')}</Button>}
        </div>}
        <div className="px-2 py-1 text-xs text-neutral-500">{t('子代理', 'Sub-agents')} · {children.length}</div>
        {children.map(child => <button key={child.id} type="button" onClick={() => void select(child)} className="flex w-full items-center gap-2 rounded-lg px-3 py-2.5 text-left hover:bg-neutral-500/5">
          <Bot size={15} className="shrink-0 text-neutral-400" />
          <span className="min-w-0 flex-1"><span className="block truncate font-medium">{child.name}</span><span className="block truncate text-xs text-neutral-500">{child.profile.model}</span></span>
          <span className="shrink-0 text-xs text-neutral-500">{labels[child.runs.at(-1)?.status ?? ''] ?? t('未知状态', 'Unknown')}</span><ChevronRight size={13} className="shrink-0 text-neutral-400" />
        </button>)}
        {!children.length && <p className="px-2 py-3 text-xs text-neutral-400">{t('派出的子代理会显示在这里', 'Delegated agents will appear here')}</p>}
      </div>}
      {selected && run && <div className="min-w-0">
        <div className="sticky top-0 z-10 flex items-center gap-2 border-b border-neutral-200 bg-white p-2 dark:border-neutral-700 dark:bg-neutral-900">
          <IconButton label={t('返回任务列表', 'Back to tasks')} size="sm" variant="ghost" onClick={() => { selectedId.current = null; setSelected(null); setText('') }}><ChevronLeft size={15} /></IconButton>
          <span className="min-w-0 flex-1 truncate font-medium">{selected.name}</span><span className="shrink-0 text-xs text-neutral-500">{labels[run.status]}</span>
        </div>
        <div className="space-y-4 p-3">
          <p className="truncate text-xs text-neutral-500">{selected.profile.agentType} · {selected.profile.model}</p>
          <details><summary className="cursor-pointer text-xs text-neutral-500">{t('任务说明', 'Task instructions')}</summary><p className="mt-2 max-h-48 overflow-auto whitespace-pre-wrap break-words custom-scrollbar">{run.prompt}</p></details>
          {active(run.status) && <div className="space-y-1 text-xs text-neutral-500">{selected.steps?.map(step => <p key={step}>{step}</p>)}<p>{t('子代理正在处理任务，完成后结果会显示在这里。', 'The agent is working. Its result will appear here when ready.')}</p></div>}
          {run.result && <div className="min-w-0 break-words"><ChatMarkdown content={run.result} /></div>}
          {run.error && <p role="status" className="whitespace-pre-wrap break-words text-red-600">{run.error}</p>}
          <label className="block space-y-2"><span>{t('补充信息或后续任务', 'Information or follow-up task')}</span><TextArea value={text} onChange={setText} /></label>
          <div className="flex flex-wrap gap-2">
            <Button size="sm" disabled={busy || !text.trim()} onClick={() => void operate('message')}>{active(run.status) ? t('补充信息', 'Send information') : t('留言', 'Leave message')}</Button>
            {!active(run.status) && <Button size="sm" disabled={busy || !text.trim()} onClick={() => void operate('continue')}>{t('继续', 'Continue')}</Button>}
            {active(run.status) && <Button size="sm" variant="ghost" disabled={busy || run.status === 'stopping'} onClick={() => void operate('stop')}>{run.status === 'stopping' ? t('正在停止', 'Stopping') : t('停止此子代理', 'Stop this agent')}</Button>}
          </div>
          {selected.messages.map(message => <div key={message.id} className="border-l-2 border-neutral-200 pl-2 dark:border-neutral-700"><span className="text-xs text-neutral-500">{message.sender} · {message.consumedBy ? t('已消费', 'Consumed') : t('已受理 · 待消费', 'Accepted · Pending')}</span><p className="whitespace-pre-wrap break-words">{message.text}</p></div>)}
          <details><summary className="cursor-pointer text-xs text-neutral-500">{t('完整历史与工具记录', 'Full history and tool records')}</summary><pre className="mt-2 max-h-80 overflow-auto whitespace-pre-wrap break-words text-xs custom-scrollbar">{JSON.stringify({ history: selected.history, tools: selected.tools, runs: selected.runs }, null, 2)}</pre></details>
        </div>
      </div>}
    </section>
  )
}
