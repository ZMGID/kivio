import { useState } from 'react'
import { ChevronRight, MessageCircle, Users } from 'lucide-react'
import { useLang } from '../components/i18n'
import type { ToolCallRecord } from './types'
import { useSubAgents } from './useSubAgents'
import { SubAgentAvatar } from './SubAgentAvatar'
import { requestDockSubAgent } from './dock/dockPreview'
import { ChatMarkdown } from './ChatMarkdown'
import { subAgentStatusLabel } from './subAgentStatus'

const object = (value: unknown): Record<string, unknown> => value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {}
const string = (value: unknown) => typeof value === 'string' ? value : ''
function parsed(value: unknown) {
  if (typeof value !== 'string') return object(value)
  try { return object(JSON.parse(value)) } catch { return {} }
}

export function SubAgentToolCard({ toolCall }: { toolCall: ToolCallRecord }) {
  const zh = useLang() === 'zh'
  const [expanded, setExpanded] = useState(false)
  const t = (a: string, b: string) => zh ? a : b
  const args = parsed(toolCall.arguments ?? toolCall.args ?? toolCall.input)
  const structured = object(toolCall.structured_content ?? toolCall.structuredContent)
  const receipt = Object.keys(structured).length ? structured : parsed(toolCall.result_preview ?? toolCall.resultPreview ?? toolCall.result ?? toolCall.output)
  const launch = receipt.type === 'subagent_started'
  // Route by the native operation even before admission succeeds. Legacy saved
  // native results also use this view; only a durable receipt enables controls.
  const starting = !launch && !args.operation && (toolCall.source === 'native' || receipt.type === 'subagent')
  const historical = receipt.type === 'subagent'
  const conversationId = string(receipt.conversation_id) || toolCall.conversationId || ''
  const { agents, error } = useSubAgents(launch && conversationId ? conversationId : null)
  const child = agents.find(item => item.id === receipt.id)
  // A later continuation must never rewrite the status of the original launch.
  const run = child?.runs.find(item => item.id === receipt.execution_id)
  const operations: Record<string, string> = { list: t('查看子代理', 'List sub-agents'), get: t('读取子代理结果', 'Read sub-agent result'), wait: t('等待子代理', 'Wait for sub-agents'), message: t('补充消息', 'Send information'), continue: t('继续子代理', 'Continue sub-agent'), stop: t('停止子代理', 'Stop sub-agent') }
  const reasons: Record<string, string> = { timeout: t('等待超时，子代理继续运行', 'Wait timed out; children keep running'), result_ready: t('收到子代理结果', 'Results received'), all_finished: t('子代理均已返回', 'All children returned'), user_input: t('收到用户输入', 'User input received') }
  const operation = string(args.operation)
  const rows = launch ? [{ ...run, id: receipt.id, name: child?.name || receipt.name, status: run?.status || 'accepted' }] : Array.isArray(receipt.agents) ? receipt.agents.map(object) : receipt.id ? [receipt] : []
  const failed = ['error', 'failed', 'cancelled'].includes(toolCall.status ?? '')
  const pending = ['running', 'pending', 'calling', 'in_progress'].includes(toolCall.status ?? '')
  const prompt = string(args.message) || (starting ? string(args.prompt) : '')
  const legacyRecovery = string(receipt.error).startsWith('recovered: ')
  const result = string(receipt.result) || (legacyRecovery ? string(receipt.error).slice(11) : '')
  if (launch) {
    const row = rows[0]
    const name = string(row.name) || string(row.id)
    return <button type="button"
      disabled={!conversationId || !row.id}
      onClick={() => requestDockSubAgent({ conversationId, agentId: string(row.id) })}
      title={t('打开子代理对话', 'Open sub-agent conversation')}
      className="not-prose my-1 flex w-full items-center gap-2 rounded-md px-3 py-2 text-left text-[12.5px] leading-5 text-neutral-500 enabled:hover:bg-neutral-500/5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-neutral-400 disabled:cursor-default">
      <SubAgentAvatar id={string(row.id)} size={20} status={string(row.status)} />
      <span className="min-w-0 flex-1 truncate" title={name}>{name}</span>
      <span className={`shrink-0 text-xs ${error ? 'text-amber-600' : 'text-neutral-500'}`}>
        {error ? t('状态暂时不可用', 'Status temporarily unavailable') : subAgentStatusLabel(row, zh ? 'zh' : 'en')}
      </span>
    </button>
  }
  const row = rows.length === 1 ? rows[0] : undefined
  const name = row ? string(row.name) || t('子代理', 'Sub-agent') : ''
  const operationError = failed || (Boolean(receipt.error) && !receipt.id && !legacyRecovery)
  const waiting = operation === 'wait'
  const waitReason = string(receipt.reason)
  // A completed polling window is not progress or an execution failure.
  // Keep the receipt in history, but omit its transient UI after it ends.
  if (waiting && !operationError) {
    if (pending) return <div role="status" className="not-prose my-1 flex items-center gap-2 px-3 py-2 text-[12.5px] leading-5 text-neutral-500">
      <Users size={16} className="shrink-0" aria-hidden="true" />
      {t('正在等待结果', 'Waiting for results')}
    </div>
    if (waitReason === 'timeout' || waitReason === 'user_input') return null
  }
  const label = starting ? [t('子代理', 'Sub-agent'), string(args.name) || string(receipt.name) || string(args.subagent_type)].filter(Boolean).join(' · ') : operation === 'message'
    ? pending ? t(`正在向「${name || '子代理'}」发送消息`, `Sending message to ${name || 'sub-agent'}`)
      : operationError ? t(`向「${name || '子代理'}」发送消息`, `Send message to ${name || 'sub-agent'}`)
        : t(`已向「${name || '子代理'}」发送消息`, `Message sent to ${name || 'sub-agent'}`)
    : [waiting && !operationError && ['result_ready', 'all_finished'].includes(waitReason)
      ? t('子代理结果', 'Sub-agent results')
      : operations[operation] || t('子代理操作', 'Sub-agent operation'), name].filter(Boolean).join(' · ')
  const status = starting ? (historical ? t('历史记录', 'Saved history') : pending ? t('正在启动…', 'Starting…') : failed ? t('未启动', 'Not started') : t('历史记录', 'Saved history')) : operationError ? t('操作异常', 'Operation error')
    : pending ? t('处理中…', 'Working…')
      : (waiting && row ? subAgentStatusLabel(row, zh ? 'zh' : 'en') : reasons[waitReason]) || (operation === 'message' ? t('已受理', 'Accepted')
        : row ? subAgentStatusLabel(row, zh ? 'zh' : 'en')
          : t(`${rows.length} 个子代理`, `${rows.length} sub-agents`))
  const issue = toolCall.error || string(receipt.error) || (operationError ? t('操作异常', 'Operation error') : '')
  return <section className="not-prose my-1 min-w-0 text-[12.5px] leading-5">
    <button type="button" aria-expanded={expanded} onClick={() => setExpanded(value => !value)}
      className="flex w-full min-w-0 items-center gap-2 rounded-md px-3 py-2 text-left text-neutral-500 hover:bg-neutral-500/5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-neutral-400">
      {operation === 'message' ? <MessageCircle size={16} className="shrink-0" aria-hidden="true" />
        : row ? <SubAgentAvatar id={string(row.id)} size={20} status={string(row.status)} />
          : <Users size={16} className="shrink-0" aria-hidden="true" />}
      <span className="min-w-0 flex-1 truncate" title={label}>{label}</span>
      <span className={`max-w-[50%] truncate text-xs ${operationError ? 'text-red-600' : ''}`} title={status}>{status}</span>
      <ChevronRight size={12} aria-hidden="true" className={`shrink-0 transition-transform ${expanded ? 'rotate-90' : ''}`} />
    </button>
    {expanded && <div className="ml-5 space-y-2 border-l border-neutral-200 py-2 pl-3 dark:border-neutral-700">
      {rows.map((item, index) => <button key={string(item.id) || index} type="button" disabled={!conversationId || !item.id}
        onClick={() => requestDockSubAgent({ conversationId, agentId: string(item.id) })}
        className="flex w-full items-center gap-2 rounded-md px-2 py-1 text-left enabled:hover:bg-neutral-500/5 disabled:cursor-default">
        <SubAgentAvatar id={string(item.id)} size={20} status={string(item.status)} />
        <span className="min-w-0 flex-1 truncate">{string(item.name) || t('子代理', 'Sub-agent')}</span>
        <span className="shrink-0 text-xs text-neutral-500">{subAgentStatusLabel(item, zh ? 'zh' : 'en')}</span>
      </button>)}
      {typeof receipt.waited_ms === 'number' && <p className="text-xs text-neutral-500">{t('实际等待', 'Waited')} {(receipt.waited_ms / 1000).toFixed(1)}s</p>}
      {issue && !legacyRecovery && <p role="alert" className="whitespace-pre-wrap break-words text-red-600">{issue}</p>}
      {prompt && <p className="whitespace-pre-wrap break-words">{prompt}</p>}
      {result && <ChatMarkdown content={result} />}
      {!rows.length && !pending && !operationError && operation === 'list' && <p>{t('暂无子代理', 'No sub-agents')}</p>}
    </div>}
  </section>
}
