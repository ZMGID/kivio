import type { SubagentIrcMessage } from '../generated/chatProtocol'
import { useLang } from '../components/i18n'
import { AgentOutput } from './AgentProgress'
import { useState } from 'react'
import { api } from '../api/tauri'
import { useTauriEvent } from './hooks/useTauriEvent'

export function IrcObservations({ conversationId }: { conversationId?: string | null }) {
  return conversationId ? <ObservedTraffic key={conversationId} conversationId={conversationId} /> : null
}

function ObservedTraffic({ conversationId }: { conversationId: string }) {
  const [messages, setMessages] = useState<SubagentIrcMessage[]>([])
  useTauriEvent(api.onChatSubagentIrc, message => {
    if (message.conversationId !== conversationId || !message.observation) return
    // shortcut: keep the latest 50 display-only observations while this conversation is open; page if longer inspection becomes necessary.
    setMessages(previous => previous.some(item => item.id === message.id) ? previous : [...previous, message].slice(-50))
  }, [conversationId])
  return <div data-chat-agent-observations>{messages.map(message => <IrcMessageCard key={message.id} message={message} />)}</div>
}

export function IrcMessageCard({ message }: { message: SubagentIrcMessage }) {
  const zh = useLang() === 'zh'
  return <article className="my-2 min-w-0 space-y-1 rounded-lg border border-[var(--color-border)] px-3 py-2 text-[13px] leading-6" aria-label={zh ? '代理消息' : 'Agent message'}>
    <header className="flex flex-wrap items-center justify-between gap-x-3 gap-y-1 text-xs text-[var(--text-muted)]">
      <span className="break-all">{message.fromAgent} → {message.broadcast ? zh ? '所有代理' : 'All agents' : message.toAgent}</span>
      <span>{message.observation ? zh ? '旁观' : 'Observation' : message.replyTo ? zh ? '回复' : 'Reply' : zh ? '收到消息' : 'Incoming'}{message.progress ? zh ? ' · 进度' : ' · Progress' : ''}</span>
    </header>
    <AgentOutput content={message.content} zh={zh} />
  </article>
}
