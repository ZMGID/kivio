import { Button } from '../../components/Button'
import { useT } from '../../components/i18n'
import { BotAvatar } from '../BotAvatar'
import { requestPeerExchange } from './exchangeRequest'

/** Colleague activity stays quiet in the private chat; its transcript opens on demand. */
export function BotExchangeNotice({ exchangeId, label, name, status, error }: {
  exchangeId?: string
  label: string
  name?: string
  status?: string
  error?: string
}) {
  const t = useT()
  const summary = (
    <span className="flex min-w-0 flex-wrap items-center justify-center gap-x-1.5 gap-y-1 text-[13px] text-[var(--text-muted)]">
      <span>{label}</span>
      {name && <><BotAvatar size={20} /><span className="break-words">{name}</span></>}
      {status && <><span aria-hidden="true">·</span><span>{status}</span></>}
    </span>
  )
  return (
    <div className="my-3 flex min-w-0 flex-col items-center gap-1">
      {exchangeId ? (
        <Button
          variant="ghost"
          size="sm"
          aria-label={`${t.chatBotOpenExchange} · ${label}${name ? ` ${name}` : ''}`}
          onClick={() => requestPeerExchange(exchangeId)}
        >
          {summary}
        </Button>
      ) : summary}
      {error && <p className="max-w-full break-words text-center text-[12px] text-[var(--danger)]" role="alert">{error}</p>}
    </div>
  )
}
