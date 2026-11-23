import { useEffect, useId, useMemo, useRef, useState, useSyncExternalStore, type ReactNode } from 'react'
import { CircleAlert, Clock, RefreshCw, Ticket, X } from 'lucide-react'
import { cliCreditAmount, cliRemaining, cliResetTime, cliWindowLabel } from '../api/cliAccountFormat'
import {
  externalCliSettingsApi,
  onExternalAgentsUpdated,
  type DetectedExternalAgent,
  type ExternalCliAccount,
} from '../api/externalCliSettings'
import { refreshCliAccount, useCliAccounts } from '../api/useCliAccount'
import { AgentIcon } from '../components/AgentIcon'
import { Button, IconButton } from '../components/Button'
import { notifyError } from '../components/appNoticeStore'
import { confirmDialog } from '../components/dialogQueue'
import { i18n, type Lang } from '../components/i18n'
import './UsageLimitsPanel.css'

type QuotaWindow = ExternalCliAccount['windows'][number]
type ResetCredits = NonNullable<ExternalCliAccount['resetCredits']>
type AgentRoster = { agents: DetectedExternalAgent[] | null; failed: boolean }

// Follow the desktop availability cache's recheck interval.
const ROSTER_RECHECK_MS = 10 * 60_000
const rosterListeners = new Set<() => void>()
let roster: AgentRoster = { agents: null, failed: false }
let rosterCheckedAt = 0
let rosterRequest: Promise<void> | undefined
let rosterEvents: Promise<unknown> | undefined

function publishRoster(next: AgentRoster) {
  roster = next
  rosterListeners.forEach(listener => listener())
}

function receiveAgents(agents: DetectedExternalAgent[]) {
  rosterCheckedAt = Date.now()
  publishRoster({ agents, failed: false })
}

function detectAgents(force = false): Promise<void> {
  if (rosterRequest) return rosterRequest
  if (!force && roster.agents && Date.now() - rosterCheckedAt < ROSTER_RECHECK_MS) return Promise.resolve()
  const previous = roster
  rosterRequest = externalCliSettingsApi.detectExternalAgents(force).then(agents => {
    if (roster === previous) receiveAgents(agents)
  }).catch(error => {
    if (roster === previous) publishRoster({ ...previous, failed: true })
    throw error
  }).finally(() => { rosterRequest = undefined })
  return rosterRequest
}

function subscribeRoster(listener: () => void) {
  rosterListeners.add(listener)
  rosterEvents ??= onExternalAgentsUpdated(receiveAgents).catch(() => {})
  void rosterEvents.then(() => {
    if (rosterListeners.has(listener)) void detectAgents().catch(() => {})
  })
  return () => { rosterListeners.delete(listener) }
}

function getRoster() {
  return roster
}

function countdown(milliseconds: number, zh: boolean) {
  const total = Math.max(1, Math.ceil(milliseconds / 60_000))
  const days = Math.floor(total / 1440)
  const hours = Math.floor(total % 1440 / 60)
  const minutes = total % 60
  return [
    days > 0 && `${days}${zh ? ' 天' : 'd'}`,
    hours > 0 && `${hours}${zh ? ' 小时' : 'h'}`,
    days === 0 && minutes > 0 && `${minutes}${zh ? ' 分钟' : 'm'}`,
  ].filter(Boolean).join(' ')
}

function LimitRow({ window, now, lang, stale, children }: {
  window: QuotaWindow
  now: number
  lang: Lang
  stale: boolean
  children?: ReactNode
}) {
  const t = i18n[lang]
  const zh = lang === 'zh'
  const name = cliWindowLabel(window, zh)
  const remaining = cliRemaining(window.usedPercent)
  const resetTime = cliResetTime(window.resetsAt)
  const resetDetail = resetTime === null ? null : t.cliLimitsResetAt.replace('{time}', new Date(resetTime).toLocaleString())
  const resetLabel = resetTime === null ? t.cliLimitsResetUnknown : resetTime <= now ? t.cliLimitsResetDue
    : t.cliLimitsResetIn.replace('{time}', countdown(resetTime - now, zh))
  const renews = !stale && resetTime !== null && resetTime > now
  const duration = window.windowDurationMins
  const timeLeft = renews && duration
    ? Math.min(100, (resetTime - now) / (duration * 60_000) * 100)
    : null
  const timeLabel = timeLeft === null ? null : t.cliLimitsTimeLeft.replace('{n}', String(Math.round(timeLeft)))
  const level = remaining <= 10 ? 'critical' : remaining <= 30 ? 'warning' : 'normal'
  return (
    <div className="cli-limit-row" data-level={level}>
      <div className="cli-limit-summary">
        <h3>{name}</h3>
        <div className="cli-limit-amount">
          <strong>{Math.round(remaining)}%</strong>
          <span>{t.cliLimitsPercentLeft}</span>
        </div>
      </div>
      <div className="cli-limit-reading">
        <div className="cli-limit-track" data-renews={renews} role="progressbar" aria-label={name} aria-valuemin={0} aria-valuemax={100}
          aria-valuenow={Math.round(remaining)} aria-valuetext={t.workspaceUsageLeft.replace('{n}', String(Math.round(remaining)))}>
          {remaining > 0 && <div className="cli-limit-fill" style={{ width: `${remaining}%` }} />}
          {timeLeft !== null && <span className="cli-limit-pace-mark" style={{ insetInlineStart: `clamp(2px, ${timeLeft}%, calc(100% - 2px))` }} aria-hidden="true" />}
        </div>
        <div className="cli-limit-meta">
          <span>
            <Clock size={13} aria-hidden="true" />
            {resetTime === null ? resetLabel : (
              <time dateTime={new Date(resetTime).toISOString()} title={resetDetail ?? undefined}>{resetLabel}</time>
            )}
          </span>
          {children}
          {timeLabel && <span className="cli-limit-pace">{timeLabel}</span>}
        </div>
      </div>
    </div>
  )
}

function ResetCardsDialog({ credits, lang, now, stale, redeeming, result, onClose, onRedeem }: {
  credits: ResetCredits
  lang: Lang
  now: number
  stale: boolean
  redeeming: boolean
  result: string | null
  onClose: () => void
  onRedeem: (creditId: string | null) => void
}) {
  const t = i18n[lang]
  const dialogRef = useRef<HTMLDialogElement>(null)
  const titleId = useId()
  useEffect(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null
    const dialog = dialogRef.current
    dialog?.showModal()
    return () => { dialog?.close(); if (previous?.isConnected) previous.focus() }
  }, [])
  const statuses = {
    available: t.cliLimitsResetCardReady, redeeming: t.cliLimitsResetCardRedeeming,
    redeemed: t.cliLimitsResetCardRedeemed, unknown: t.cliLimitsResetCardUnknown,
  }
  const cards = credits.cards ?? []
  const availableDetails = cards.filter(card => card.status === 'available').length
  const canUse = Boolean(credits.accountKey) && credits.availableCount > 0 && !stale && !redeeming
  const fullTime = (timestamp: number | null) => {
    const milliseconds = cliResetTime(timestamp)
    if (milliseconds === null) return '—'
    const date = new Date(milliseconds)
    return <time dateTime={date.toISOString()}>{date.toLocaleString(lang)}</time>
  }
  return (
    <dialog ref={dialogRef} className="kv kv-modal cli-reset-dialog" aria-labelledby={titleId}
      onKeyDown={event => event.stopPropagation()}
      onCancel={event => { event.preventDefault(); if (!redeeming) onClose() }}>
      <header className="cli-reset-dialog-header">
        <div>
          <h2 id={titleId}>{t.cliLimitsResetCards}</h2>
          <p className="cli-limits-message">{t.cliLimitsResetCardsAvailable.replace('{n}', credits.availableCount.toLocaleString(lang))}</p>
        </div>
        <IconButton label={lang === 'zh' ? '关闭' : 'Close'} variant="ghost" disabled={redeeming} onClick={onClose}><X size={16} aria-hidden="true" /></IconButton>
      </header>
      <div className="cli-reset-dialog-body custom-scrollbar" aria-busy={redeeming || undefined}>
        {result && <p className="cli-limits-message" role="status">{result}</p>}
        {stale && <p className="cli-limits-stale" role="status">{t.cliLimitsStale}</p>}
        {cards.map(card => {
          const expiresAt = cliResetTime(card.expiresAt)
          const expired = card.status === 'available' && expiresAt !== null && expiresAt <= now
          const title = card.title || (card.resetType === 'codexRateLimits' ? t.cliLimitsResetCardWindow : t.cliLimitsResetCards)
          return (
            <section key={card.id} className="cli-reset-card" aria-label={title}>
              <div className="cli-reset-card-heading">
                <div>
                  <h3>{title}</h3>
                  <span className="cli-limits-message">{expired ? t.cliLimitsResetCardExpired : statuses[card.status]}</span>
                </div>
                {card.status === 'available' && !expired && (
                  <Button size="sm" disabled={!canUse} onClick={() => onRedeem(card.id)}>{t.cliLimitsUseResetCard}</Button>
                )}
              </div>
              {card.description && <p>{card.description}</p>}
              <dl>
                <dt>{t.cliLimitsResetCardGranted}</dt><dd>{fullTime(card.grantedAt)}</dd>
                <dt>{t.cliLimitsResetCardExpiry}</dt><dd>{fullTime(card.expiresAt)}</dd>
              </dl>
            </section>
          )
        })}
        {credits.availableCount > availableDetails && <p className="cli-limits-message">{t.cliLimitsResetCardDetailsPartial}</p>}
        {credits.availableCount > 0 && availableDetails === 0 && (
          <Button size="sm" disabled={!canUse} onClick={() => onRedeem(null)}>{t.cliLimitsUseNextResetCard}</Button>
        )}
        {credits.availableCount === 0 && cards.length === 0 && (
          <p className="cli-limits-message">{t.cliLimitsResetCardNoneLeft}</p>
        )}
      </div>
    </dialog>
  )
}

function AgentLimits({ agent, account, error, lang, now }: {
  agent: DetectedExternalAgent
  account: ExternalCliAccount
  error: string | null
  lang: Lang
  now: number
}) {
  const t = i18n[lang]
  const identity = cliAccountIdentity(account)
  const updatedAt = new Date(account.checkedAt * 1000)
  const resetCredits = account.resetCredits
  const nextExpiry = cliResetTime(resetCredits?.nextExpiresAt ?? null)
  const expiresAt = nextExpiry === null ? null : new Date(nextExpiry)
  const [showResetCards, setShowResetCards] = useState(false)
  const [redeeming, setRedeeming] = useState(false)
  const [redemptionResult, setRedemptionResult] = useState<string | null>(null)
  const redeem = async (creditId: string | null) => {
    if (!resetCredits?.accountKey || error || redeeming) return
    setRedeeming(true)
    try {
      const confirmed = await confirmDialog({
        title: t.cliLimitsConfirmResetCard,
        message: t.cliLimitsResetCardEffect,
        confirmLabel: t.cliLimitsUseResetCard,
        danger: true,
      })
      if (!confirmed) return
      setRedemptionResult(null)
      const { outcome, refreshed } = await externalCliSettingsApi.redeemCodexResetCredit(
        { accountKey: resetCredits.accountKey, creditId }, resetCredits.availableCount,
      )
      const applied = outcome === 'reset' || outcome === 'alreadyRedeemed'
      const messages = {
        reset: t.cliLimitsResetCardUsed,
        nothingToReset: t.cliLimitsResetCardNothingToReset,
        noCredit: t.cliLimitsResetCardNoneLeft,
        alreadyRedeemed: t.cliLimitsResetCardAlreadyUsed,
      }
      setRedemptionResult(applied && !refreshed ? t.cliLimitsResetCardRefreshFailed : messages[outcome])
    } catch (cause) {
      setRedemptionResult(`${t.cliLimitsResetCardFailed} ${cause instanceof Error ? cause.message : String(cause)}`)
    } finally {
      setRedeeming(false)
    }
  }
  const availableLabel = resetCredits
    ? t.cliLimitsResetCardsAvailable.replace('{n}', resetCredits.availableCount.toLocaleString(lang))
    : ''
  const resetSummary = resetCredits && (resetCredits.availableCount > 0 || !!resetCredits.cards?.length) && (
    <>
      <Button size="sm" variant="ghost" aria-haspopup="dialog" aria-expanded={showResetCards}
        title={`${t.cliLimitsResetCards} · ${availableLabel}`}
        aria-label={`${t.cliLimitsResetCards} · ${availableLabel}`}
        onClick={() => setShowResetCards(true)}>
        <Ticket size={14} aria-hidden="true" />
        <span className="cli-limit-reset-count">{resetCredits.availableCount.toLocaleString(lang)}</span>
      </Button>
      {expiresAt && (
        <time dateTime={expiresAt.toISOString()}>
          {t.cliLimitsResetCardExpires.replace('{time}', expiresAt.toLocaleDateString(lang, { month: 'long', day: 'numeric' }))}
        </time>
      )}
    </>
  )
  const credits = account.credits
  const codexBalance = agent.id === 'codex' && credits && (
    <span className="cli-limits-credits">
      {t.cliLimitsCreditBalance}
      <strong>{cliCreditAmount(credits, lang === 'zh')}</strong>
    </span>
  )
  const summary = <>{resetSummary}{codexBalance}</>
  return (
    <section className="cli-limits-agent" aria-labelledby={`limits-heading-${agent.id}`}>
      <header className="cli-limits-agent-header">
        <div className="cli-limits-identity">
          <AgentIcon id={agent.id} size={22} />
          <div>
            <h2 id={`limits-heading-${agent.id}`}>{agent.name}</h2>
            {identity && <p>{identity}</p>}
          </div>
        </div>
        <time dateTime={updatedAt.toISOString()}>
          {t.cliLimitsCheckedAt.replace('{time}', updatedAt.toLocaleString())}
        </time>
      </header>
      {error && <p className="cli-limits-stale" role="status" title={error}><CircleAlert size={14} aria-hidden="true" />{t.cliLimitsStale}</p>}
      {(account.windows.length > 0 || resetSummary || codexBalance) && <div className="cli-limits-windows">
        {account.windows.map((window, index) => (
          <LimitRow key={`${window.name}-${index}`} window={window} now={now} lang={lang} stale={Boolean(error)}>
            {index === account.windows.length - 1 && summary}
          </LimitRow>
        ))}
        {account.windows.length === 0 && (
          <div className="cli-limit-row"><div className="cli-limit-meta">{summary}</div></div>
        )}
      </div>}
      {agent.id !== 'codex' && credits && <p className="cli-limits-credits">
        <span>{credits.label}</span>
        <strong>{cliCreditAmount(credits, lang === 'zh')}</strong>
      </p>}
      {showResetCards && resetCredits && (
        <ResetCardsDialog credits={resetCredits} lang={lang} now={now} stale={Boolean(error)} redeeming={redeeming}
          result={redemptionResult} onClose={() => { setShowResetCards(false); setRedemptionResult(null) }}
          onRedeem={creditId => void redeem(creditId)} />
      )}
    </section>
  )
}

function cliAccountIdentity(reading: ExternalCliAccount) {
  const { account } = reading
  return [account?.email || account?.name, account?.plan].filter(Boolean).join(' · ')
}

export function UsageLimitsPanel({ lang }: { lang: Lang }) {
  const t = i18n[lang]
  const { agents, failed: scanFailed } = useSyncExternalStore(subscribeRoster, getRoster, getRoster)
  const [refreshing, setRefreshing] = useState(false)
  const [now, setNow] = useState(() => Date.now())

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 30_000)
    return () => window.clearInterval(timer)
  }, [])

  const available = useMemo(() => agents?.filter(agent => agent.available && agent.capabilities.account === 'native') ?? [], [agents])
  const accounts = useCliAccounts(available)
  const readings = accounts.flatMap(({ agent, view }) => {
    const { account, error } = view
    return account && (cliAccountIdentity(account) || account.windows.length > 0 || (account.resetCredits?.availableCount ?? 0) > 0 || !!account.resetCredits?.cards?.length)
      ? [<AgentLimits key={agent.id} agent={agent} account={account} error={error} now={now} lang={lang} />]
      : []
  })
  const checking = accounts.some(account => account.checking)
  const refresh = async () => {
    setRefreshing(true)
    try {
      if (available.length === 0) {
        await detectAgents(true)
      } else {
        await Promise.allSettled(available.map(agent => refreshCliAccount(agent.id)))
      }
    } catch {
      notifyError(t.cliLimitsScanFailed)
    } finally {
      setNow(Date.now())
      setRefreshing(false)
    }
  }

  return (
    <div className="cli-limits-page" aria-label={t.cliLimitsTitle}>
      <div className="cli-limits-toolbar">
        <Button size="sm" variant="ghost" aria-label={t.cliLimitsRefresh} disabled={refreshing || (!agents && !scanFailed)} onClick={() => void refresh()}>
          <RefreshCw size={14} aria-hidden="true" className={refreshing ? 'animate-spin' : ''} />
          {lang === 'zh' ? '刷新' : 'Refresh'}
        </Button>
      </div>
      {available.length
        ? <div className="cli-limits-list">
            {readings}
            {!readings.length && <p className="cli-limits-message" role="status">
              {checking ? t.cliLimitsChecking : t.cliLimitsNoReadings}
            </p>}
          </div>
        : <p className="cli-limits-message" role="status">{scanFailed ? t.cliLimitsScanFailed : agents ? t.cliLimitsEmpty : t.cliLimitsScanning}</p>}
    </div>
  )
}
