import { useState } from 'react'
import { Check, ChevronRight, Terminal } from 'lucide-react'
import type { SubAgentRecord } from '../api/tauri'
import { ChatMarkdown } from './ChatMarkdown'
import { SubAgentAvatar } from './SubAgentAvatar'
import { Button } from '../components/Button'

function object(value: unknown): Record<string, unknown> {
  return value != null && typeof value === 'object' ? value as Record<string, unknown> : {}
}
function text(value: unknown): string {
  if (typeof value === 'string') return value
  if (Array.isArray(value)) return value.map(item => object(item).text).filter(item => typeof item === 'string').join('\n')
  return ''
}

function Delegation({ content, lang }: { content: string; lang: 'zh' | 'en' }) {
  const [expanded, setExpanded] = useState(false)
  return <div className="ml-6 rounded-2xl rounded-tr-md bg-neutral-100 px-3.5 py-3 dark:bg-neutral-800">
    <div className="mb-1 text-[11px] text-neutral-400">{lang === 'zh' ? '派工 / 补充信息' : 'Assignment / information'}</div>
    <div className={`whitespace-pre-wrap break-words leading-6 ${expanded ? '' : 'line-clamp-4'}`}>{content}</div>
    {content.length > 160 && <Button size="sm" variant="ghost" onClick={() => setExpanded(!expanded)}>{expanded ? (lang === 'zh' ? '收起' : 'Less') : (lang === 'zh' ? '展开' : 'More')}</Button>}
  </div>
}

export function SubAgentConversation({ child, lang }: { child: SubAgentRecord; lang: 'zh' | 'en' }) {
  const history = child.history.map(object).filter(message => ['user', 'assistant', 'tool'].includes(String(message.role)))
  const run = child.runs.at(-1)
  const messages = history.length ? history : child.runs.flatMap(item => [
    { role: 'user', content: item.prompt }, ...(item.result ? [{ role: 'assistant', content: item.result }] : []),
  ])
  const resultInHistory = messages.some(message => message.role === 'assistant' && text(message.content).trim() === run?.result?.trim())
  const timeline: Record<string, unknown>[] = [...messages]
  if (run?.result && !resultInHistory) timeline.push({ role: 'assistant', content: run.result })
  const results = new Map(history.filter(message => message.role === 'tool').map(message => [message.tool_call_id, text(message.content)]))
  return <div aria-label={lang === 'zh' ? '子代理对话' : 'Sub-agent conversation'} className="space-y-6 px-4 py-5 text-[13px]">
    {timeline.map((message, index) => {
      const content = text(message.content)
      if (message.role === 'tool') return null
      if (message.role === 'user') return <Delegation key={index} content={content} lang={lang} />
      const calls = Array.isArray(message.tool_calls) ? message.tool_calls.map(object) : []
      if (!content && !calls.length) return null
      return <article key={index} className="min-w-0 space-y-2">
        <div className="flex items-center gap-2 text-xs text-neutral-500"><SubAgentAvatar id={child.id} size={22} /><span>{child.name}</span></div>
        {content && <div className="min-w-0 break-words leading-6"><ChatMarkdown content={content} /></div>}
        {calls.map((call, callIndex) => {
          const fn = object(call.function)
          const output = results.get(call.id)
          return <details key={String(call.id ?? callIndex)} className="group rounded-lg border border-neutral-200/70 dark:border-neutral-700">
            <summary className="flex cursor-pointer list-none items-center gap-2 px-3 py-2 text-xs text-neutral-500"><Terminal size={13} /><span className="min-w-0 flex-1 truncate">{String(fn.name ?? call.name ?? 'Tool')}</span>{output != null && <Check size={12} />}<ChevronRight size={12} className="transition-transform group-open:rotate-90" /></summary>
            <div className="max-h-60 overflow-auto whitespace-pre-wrap break-words border-t border-neutral-200/70 p-3 text-xs custom-scrollbar dark:border-neutral-700">{output || (lang === 'zh' ? '等待执行结果' : 'Waiting for result')}</div>
          </details>
        })}
      </article>
    })}
    {child.messages.filter(message => !message.consumedBy).map(message => <div key={message.id}>
      <Delegation content={message.text} lang={lang} />
      <p className="mt-1 text-right text-[11px] text-neutral-400">{lang === 'zh' ? '已受理 · 等待处理' : 'Accepted · Pending'}</p>
    </div>)}
    {run?.error && <p role="status" className="rounded-lg bg-red-50 p-3 text-red-700 dark:bg-red-950/30 dark:text-red-300">{run.error}</p>}
    {run && ['running', 'finishing', 'stopping'].includes(run.status) && <div role="status" className="flex items-center gap-2 text-xs text-neutral-400"><SubAgentAvatar id={child.id} status={run.status} size={22} />{lang === 'zh' ? '正在处理任务…' : 'Working…'}</div>}
  </div>
}
