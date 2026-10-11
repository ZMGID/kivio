import type { ReactNode } from 'react'
import { useLang, useT } from '../../components/i18n'
import { BotAvatar } from '../BotAvatar'
import { ChatMarkdown } from '../ChatMarkdown'
import type { Actor, DeliveryStatus } from './collaboration'
import { deliveryStatusLabel } from './presence'

/** Shared conversation presentation for peer exchanges and personal groups. */
export function BotMessage({ actor, body, at, status, children }: {
  actor: Actor
  body: string
  at: number
  status?: DeliveryStatus
  children?: ReactNode
}) {
  const t = useT()
  const lang = useLang()
  const time = new Date(at * 1000)
  return (
    <article className={`group flex min-w-0 items-end gap-2.5 ${actor.kind === 'user' ? 'justify-end' : ''}`}>
      {actor.kind === 'bot' && <BotAvatar icon={actor.icon ?? undefined} color={actor.color ?? undefined} size={32} animated={false} />}
      <div className="min-w-0 max-w-[90%]">
        <p className="mb-1.5 px-1 text-[12px] font-medium text-[var(--text-muted)]">
          {actor.kind === 'user' ? t.chatBotYou : actor.name}
        </p>
        <div className="chat-user-bubble min-w-0 rounded-2xl px-4 py-3 [overflow-wrap:anywhere]">
          <ChatMarkdown content={body} />
        </div>
        <div className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-1 px-1 text-[11px] text-[var(--text-muted)]">
          <time dateTime={time.toISOString()}>{time.toLocaleString(lang === 'zh' ? 'zh-CN' : 'en-US', {
            month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hour12: false,
          })}</time>
          {status && status !== 'succeeded' && <span>{deliveryStatusLabel(status, t)}</span>}
          {children}
        </div>
      </div>
    </article>
  )
}
