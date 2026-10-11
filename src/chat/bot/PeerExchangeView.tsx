import { useEffect, useState } from 'react'
import { ArrowLeft, ArrowLeftRight, Check, Copy } from 'lucide-react'
import { Button, IconButton } from '../../components/Button'
import { useLang, useT } from '../../components/i18n'
import { copyToClipboard } from '../../utils/clipboard'
import { BotAvatar } from '../BotAvatar'
import { ChatMarkdown } from '../ChatMarkdown'
import { collaborationApi, decodeExchange, type Exchange } from './collaboration'
import { useBotCollaboration } from './collaborationStore'
import { deliveryStatusLabel } from './presence'

function formatTime(at: number, lang: 'zh' | 'en') {
  return new Date(at * 1000).toLocaleString(lang === 'zh' ? 'zh-CN' : 'en-US', {
    month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hour12: false,
  })
}

function laterExchange(current: Exchange, incoming: Exchange) {
  if (incoming.entries.length !== current.entries.length) return incoming.entries.length > current.entries.length
  const at = (exchange: Exchange) => exchange.entries.reduce((best, entry) => Math.max(best, entry.at), 0)
  return at(incoming) > at(current)
}

/** Store events win once they contain the reply. A late query cannot replace that. */
function visibleExchange(store: Exchange | null, query: Exchange | null): Exchange | null {
  if (!store) return query
  if (!query || query.id !== store.id) return store
  return laterExchange(store, query) ? query : store
}

export function PeerExchangeView({ exchangeId, onClose }: { exchangeId: string; onClose: () => void }) {
  const t = useT()
  const lang = useLang()
  const { snapshot } = useBotCollaboration()
  const live = snapshot.exchanges.find((item) => item.id === exchangeId) ?? null
  const [fetched, setFetched] = useState<{ id: string; exchange: Exchange | null; error: string; loading: boolean }>({
    id: '', exchange: null, error: '', loading: false,
  })
  const queried = fetched.id === exchangeId ? fetched.exchange : null
  const exchange = visibleExchange(live, queried)
  const [copiedId, setCopiedId] = useState('')
  useEffect(() => {
    if (!copiedId) return
    const timeout = window.setTimeout(() => setCopiedId(''), 2000)
    return () => window.clearTimeout(timeout)
  }, [copiedId])
  const participants = [...new Map(exchange?.entries.map((entry) => [
    entry.actor.kind === 'bot' ? entry.actor.id : 'user', entry.actor,
  ]) ?? []).values()]
  const error = exchange ? '' : fetched.id === exchangeId ? fetched.error : ''
  const loading = !exchange && !error && (fetched.id !== exchangeId || fetched.loading)
  useEffect(() => {
    let current = true
    const requested = exchangeId
    setFetched({ id: requested, exchange: null, error: '', loading: true })
    void collaborationApi.exchange(requested).then((raw) => {
      if (!current) return
      const decoded = decodeExchange(raw)
      if (!decoded || decoded.id !== requested) {
        setFetched({ id: requested, exchange: null, error: t.chatBotExchangeFailed, loading: false })
        return
      }
      setFetched({ id: requested, exchange: decoded, error: '', loading: false })
    }).catch((err) => {
      if (!current) return
      setFetched({ id: requested, exchange: null, error: err instanceof Error ? err.message : String(err), loading: false })
    })
    return () => { current = false }
  }, [exchangeId, t.chatBotExchangeFailed])
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onClose])
  return (
    <section className="absolute inset-0 z-30 flex min-h-0 flex-col bg-[var(--theme-surface)]" aria-label={t.chatBotOpenExchange}>
      <header className="flex min-h-14 shrink-0 items-center gap-3 px-4 py-2">
        <Button size="sm" onClick={onClose}><ArrowLeft size={14} />{t.chatBotExchangeClose}</Button>
        <div className="flex min-w-0 flex-1 flex-wrap items-center justify-center gap-x-2 gap-y-1 text-[13px] text-[var(--text-muted)]">
          {participants.map((actor, index) => (
            <span key={actor.kind === 'bot' ? actor.id : 'user'} className="flex min-w-0 items-center gap-2">
              {index > 0 && <ArrowLeftRight size={14} aria-hidden="true" />}
              {actor.kind === 'bot' && <BotAvatar size={24} />}
              <span className="break-words">{actor.kind === 'user' ? t.chatBotYou : actor.name}</span>
            </span>
          ))}
        </div>
      </header>
      <div className="custom-scrollbar min-h-0 flex-1 overflow-y-auto px-6 pb-8">
        <div className="chat-reading-content space-y-5 pt-4">
        {loading && <p className="text-[13px] text-neutral-500" aria-busy="true">{t.chatLoading}</p>}
        {error && <p className="text-[13px] text-red-600" role="alert">{error}</p>}
        {exchange && exchange.entries.length === 0 && <p className="text-[13px] text-neutral-500">{t.chatBotExchangeEmpty}</p>}
        {exchange?.entries.map((entry) => (
          <article key={entry.id} className={`flex min-w-0 items-end gap-2.5 ${entry.actor.kind === 'user' ? 'justify-end' : ''}`}>
            {entry.actor.kind === 'bot' && <BotAvatar size={28} />}
            <div className="min-w-0 max-w-[90%]">
              <p className="mb-1.5 px-1 text-[12px] font-medium text-[var(--text-muted)]">
                {entry.actor.kind === 'user' ? t.chatBotYou : entry.actor.name}
              </p>
              <div className="chat-user-bubble min-w-0 rounded-2xl px-4 py-3 [overflow-wrap:anywhere]">
                <ChatMarkdown content={entry.body} />
              </div>
              <div className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-1 px-1 text-[11px] text-[var(--text-muted)]">
                <time dateTime={new Date(entry.at * 1000).toISOString()}>{formatTime(entry.at, lang)}</time>
                {entry.status !== 'succeeded' && <span>{deliveryStatusLabel(entry.status, t)}</span>}
                {entry.body.trim() && (
                  <IconButton
                    size="xs" variant="ghost" label={copiedId === entry.id ? t.annotateCopied : t.editCopy}
                    onClick={() => { void copyToClipboard(entry.body).then((ok) => { if (ok) setCopiedId(entry.id) }) }}
                  >
                    {copiedId === entry.id ? <Check size={12} /> : <Copy size={12} />}
                  </IconButton>
                )}
              </div>
            </div>
          </article>
        ))}
        </div>
      </div>
    </section>
  )
}
