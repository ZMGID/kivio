import { useEffect, useRef, useState } from 'react'
import { api, type SubAgentRecord as Child } from '../api/tauri'
import { Button } from '../components/Button'
import { TextArea } from '../settings/components'

const active = (status: string) => ['running', 'finishing', 'stopping'].includes(status)
const chineseLabels: Record<string, string> = { running: '运行中', finishing: '正在收尾', stopping: '正在停止', completed: '已完成', failed: '失败', interrupted: '已中断' }

export function SubAgentPanel({ conversationId, lang = 'zh' }: { conversationId: string; lang?: 'zh' | 'en' }) {
  const t = (zh: string, en: string) => lang === 'zh' ? zh : en
  const labels = lang === 'zh' ? chineseLabels : { running: 'Running', finishing: 'Finishing', stopping: 'Stopping', completed: 'Completed', failed: 'Failed', interrupted: 'Interrupted' } as Record<string, string>
  const [connection, setConnection] = useState(0)
  const [children, setChildren] = useState<Child[]>([])
  const [selected, setSelected] = useState<Child | null>(null)
  const [text, setText] = useState('')
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const selectedId = useRef<string | null>(null)
  const epoch = useRef(0)
  const submission = useRef<{ fingerprint: string; id: string } | null>(null)

  useEffect(() => {
    let disposed = false
    const generation = ++epoch.current
    selectedId.current = null
    setSelected(null)
    setChildren([])
    setError('')
    setText('')
    setBusy(false)
    submission.current = null
    const sync = async () => {
      try {
        let snapshot = await api.chatSubagentControl(conversationId, { operation: 'list' })
        while (!disposed) {
          setChildren(snapshot.agents)
          const id = selectedId.current
          if (id) {
            const detail = await api.chatSubagentControl(conversationId, { operation: 'get', id })
            if (!disposed && selectedId.current === id) setSelected(detail)
          }
          if (disposed) return
          snapshot = await api.chatSubagentControl(conversationId, { operation: 'wait', cursor: snapshot.sequence, timeout_ms: 30_000 })
        }
      } catch (err) {
        if (!disposed && generation === epoch.current) setError(String(err))
      }
    }
    void sync()
    return () => { disposed = true; epoch.current = generation + 1 }
  }, [conversationId, connection])

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
        setChildren(current => current.map(child => child.id === detail.id ? detail : child))
        if (operation !== 'stop') setText('')
        submission.current = null
      }
    } catch (err) { if (generation === epoch.current) setError(String(err)) }
    finally { if (generation === epoch.current) setBusy(false) }
  }

  if (!children.length && !error) return null
  const run = selected?.runs.at(-1)
  return (
    <section aria-label={t('子代理协作', 'Sub-agent collaboration')} className="mx-4 my-2 rounded-lg border border-neutral-200 p-3 text-sm dark:border-neutral-700">
      <div className="mb-2 font-medium">{t('子代理', 'Sub-agents')} · {children.length}</div>
      <div className="flex flex-wrap gap-2">
        {children.map(child => <button key={child.id} type="button" onClick={() => void select(child)} aria-pressed={selected?.id === child.id} className="rounded border border-neutral-300 px-2 py-1 dark:border-neutral-600">
          <span>{child.name}</span> · {labels[child.runs.at(-1)?.status ?? ''] ?? t('未知状态', 'Unknown')}
        </button>)}
      </div>
      {error && <div className="mt-2"><p role="alert" className="text-red-600">{error}</p><Button onClick={() => setConnection(value => value + 1)}>{t('重新连接', 'Reconnect')}</Button></div>}
      {selected && run && <div className="mt-3 space-y-2">
        <p>{selected.profile.agentType} · {selected.profile.model} · {labels[run.status]}</p>
        <p className="whitespace-pre-wrap">{run.prompt}</p>
        {active(run.status) && selected.preview && <p className="whitespace-pre-wrap text-neutral-500">{selected.preview}</p>}
        {selected.steps?.map(step => <p key={step} className="text-xs text-neutral-500">{step}</p>)}
        {run.result && <div className="max-h-64 custom-scrollbar overflow-auto whitespace-pre-wrap break-words">{run.result}</div>}
        {run.error && <p role="status" className="whitespace-pre-wrap text-red-600">{run.error}</p>}
        <label className="block">{t('补充信息或后续任务', 'Information or follow-up task')}
          <TextArea value={text} onChange={setText} />
        </label>
        <div className="flex gap-3">
          <Button type="button" disabled={busy || !text.trim()} onClick={() => void operate('message')}>{active(run.status) ? t('补充信息', 'Send information') : t('留言', 'Leave message')}</Button>
          {!active(run.status) && <Button type="button" disabled={busy || !text.trim()} onClick={() => void operate('continue')}>{t('继续', 'Continue')}</Button>}
          {active(run.status) && <Button type="button" disabled={busy || run.status === 'stopping'} onClick={() => void operate('stop')}>{run.status === 'stopping' ? t('正在停止', 'Stopping') : t('停止此子代理', 'Stop this agent')}</Button>}
        </div>
        {selected.messages.map(message => <div key={message.id} className="border-l-2 border-neutral-300 pl-2">
          <span className="text-xs text-neutral-500">{message.sender} · {message.consumedBy ? t('已消费', 'Consumed') : t('已受理 · 待消费', 'Accepted · Pending')}</span>
          <p className="whitespace-pre-wrap">{message.text}</p>
        </div>)}
        <details><summary>{t('完整历史与工具记录', 'Full history and tool records')}</summary><pre className="max-h-80 custom-scrollbar overflow-auto whitespace-pre-wrap break-words text-xs">{JSON.stringify({ history: selected.history, tools: selected.tools, runs: selected.runs }, null, 2)}</pre></details>
      </div>}
    </section>
  )
}
