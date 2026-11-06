import { MessagesSquare } from 'lucide-react'
import { useLang } from '../components/i18n'
import type { SubagentIrcMessage } from '../generated/chatProtocol'

export function IrcMessageCard({ message }: { message: SubagentIrcMessage }) {
  const zh = useLang() === 'zh'
  const recipient = message.broadcast ? (zh ? '所有代理' : 'All agents') : message.toAgent
  const route = `${message.fromAgent} → ${recipient}`
  const title = `${zh ? '代理消息' : 'Agent message'} · ${route}`
  return <article className="not-prose flex min-w-0 items-start gap-2 py-2" aria-label={title}>
    <MessagesSquare size={14} className="mt-0.5 shrink-0 text-[var(--text-muted)]" aria-hidden="true" />
    <div className="min-w-0 text-[13px] leading-5 [overflow-wrap:anywhere]">
      <header className="text-[var(--text-muted)]">{route}</header>
      <p className="whitespace-pre-wrap">{message.content}</p>
    </div>
  </article>
}
