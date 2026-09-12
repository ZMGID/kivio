import type { SubAgentRecord } from '../api/tauri'
import type { ChatMessage } from './types'

const obj = (value: unknown): Record<string, unknown> => value != null && typeof value === 'object' ? value as Record<string, unknown> : {}
const text = (value: unknown): string => typeof value === 'string' ? value : Array.isArray(value) ? value.map(part => obj(part).text).filter(part => typeof part === 'string').join('\n') : ''

/** Adapt provider history to the same messages/segments used by ordinary chat. */
export function subAgentMessages(child: SubAgentRecord): ChatMessage[] {
  const run = child.runs.at(-1)
  const history = child.history.map(obj).filter(message => ['user', 'assistant', 'tool'].includes(String(message.role)))
  if (!history.length) child.runs.forEach(item => {
    history.push({ role: 'user', content: item.prompt })
    if (item.result) history.push({ role: 'assistant', content: item.result })
  })
  const result = run?.result ?? (run?.status === 'failed' && run.error?.startsWith('recovered: ') ? run.error.slice(11) : null)
  if (result && !history.some(message => message.role === 'assistant' && text(message.content).trim() === result.trim())) history.push({ role: 'assistant', content: result })
  const outputs = new Map(history.filter(message => message.role === 'tool').map(message => [message.tool_call_id, text(message.content)]))
  const ledger = new Map(child.tools.map(obj).map(tool => [tool.id, tool]))
  const messages: ChatMessage[] = []
  let answer: ChatMessage | null = null
  history.forEach((message, index) => {
    if (message.role === 'tool') return
    const content = text(message.content)
    if (message.role === 'user') {
      answer = null
      messages.push({ id: `${child.id}-user-${index}`, role: 'user', content, timestamp: 0 })
      return
    }
    if (!answer) {
      answer = { id: `${child.id}-answer-${index}`, role: 'assistant', content: '', timestamp: 0, segments: [], tool_calls: [] }
      messages.push(answer)
    }
    const calls = Array.isArray(message.tool_calls) ? message.tool_calls.map(obj) : []
    const phase = calls.length ? 'tool_loop' : 'synthesis'
    if (content) {
      answer.content = content
      answer.segments!.push({ id: `${child.id}-text-${index}`, kind: 'text', phase, order: answer.segments!.length, text: content })
    }
    calls.forEach((call, callIndex) => {
      const id = String(call.id ?? `${index}-${callIndex}`)
      const fn = obj(call.function)
      const saved = ledger.get(id)
      const output = outputs.get(id)
      answer!.tool_calls!.push({ id, name: String(fn.name ?? call.name ?? saved?.name ?? 'Tool'), source: 'native', arguments: fn.arguments ?? saved?.arguments, status: output != null ? 'success' : saved?.status === 'unknown' ? 'error' : 'running', result_preview: output })
      answer!.segments!.push({ id: `${child.id}-tool-${id}`, kind: 'tool', phase: 'tool_loop', order: answer!.segments!.length, tool_call_id: id })
    })
  })
  child.messages.filter(message => !message.consumedBy).forEach(message => messages.push({ id: message.id, role: 'user', content: message.text, timestamp: 0 }))
  return messages
}
