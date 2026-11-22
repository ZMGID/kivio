import { useEffect, useMemo, useState, useSyncExternalStore } from 'react'
import { CircleAlert, Clock, RefreshCw } from 'lucide-react'
import { cliCreditAmount, cliRemaining, cliResetTime, cliWindowLabel } from '../api/cliAccountFormat'
import {
  externalCliSettingsApi,
  onExternalAgentsUpdated,
  type DetectedExternalAgent,
  type ExternalCliAccount,
} from '../api/externalCliSettings'
import { refreshCliAccount, useCliAccounts } from '../api/useCliAccount'
import { AgentIcon } from '../components/AgentIcon'
import { Button } from '../components/Button'
import { notifyError } from '../components/appNoticeStore'
import { i18n, type Lang } from '../components/i18n'
import './UsageLimitsPanel.css'

type QuotaWindow = ExternalCliAccount['windows'][number]
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

function LimitRow({ window, now, lang, stale }: { window: QuotaWindow; now: number; lang: Lang; stale: boolean }) {
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
          {timeLabel && <span>{timeLabel}</span>}
        </div>
      </div>
    </div>
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
  const identity = [account.account?.email, account.account?.plan].filter(Boolean).join(' · ')
  const updatedAt = new Date(account.checkedAt * 1000)
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
      <div className="cli-limits-windows">
        {account.windows.map((window, index) => <LimitRow key={`${window.name}-${index}`} window={window} now={now} lang={lang} stale={Boolean(error)} />)}
      </div>
      {account.credits && <p className="cli-limits-credits">
        <span>{account.credits.label}</span>
        <strong>{cliCreditAmount(account.credits, lang === 'zh')}</strong>
      </p>}
    </section>
  )
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
  const readings = accounts.flatMap(({ agent, view }) => view.account?.windows.length
    ? [<AgentLimits key={agent.id} agent={agent} account={view.account} error={view.error} now={now} lang={lang} />]
    : [])
  const notices = accounts.flatMap(({ agent, view }) => {
    const reason = view.error || view.account?.unavailable
    return reason && !view.account?.windows.length
      ? [<p key={agent.id}>{agent.name} · {reason}</p>]
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
            {(!readings.length || notices.length > 0) && <div className="cli-limits-message" role="status">
              {!readings.length && <p>{checking ? t.cliLimitsChecking : t.cliLimitsNoReadings}</p>}
              {notices}
            </div>}
          </div>
        : <p className="cli-limits-message" role="status">{scanFailed ? t.cliLimitsScanFailed : agents ? t.cliLimitsEmpty : t.cliLimitsScanning}</p>}
    </div>
  )
}
