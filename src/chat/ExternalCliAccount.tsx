import { useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { RefreshCw } from 'lucide-react'
import { Button, IconButton } from '../components/Button'
import { useLang } from '../components/i18n'
import { externalCliSettingsApi, type ExternalCliAccount as Account } from '../api/externalCliSettings'
import { usePopoverMenu } from './usePopoverMenu'
import { useClampedMenuPosition } from './useClampedMenuPosition'

/** One read-only account view shared by the main and independent chat titlebars. */
export function ExternalCliAccount({ agentId, conversationId }: { agentId: string; conversationId?: string | null }) {
  const zh = useLang() === 'zh'
  const [open, setOpen] = useState(false)
  const [status, setStatus] = useState<Account | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')
  const request = useRef({ version: 0 })
  const [anchor, setAnchor] = useState({ left: 0, top: 0 })
  const menu = useRef<HTMLDivElement>(null)
  const position = useClampedMenuPosition(menu, anchor)
  usePopoverMenu(open, () => setOpen(false), menu)
  useEffect(() => {
    const tracker = request.current
    tracker.version++
    setStatus(null)
    setLoading(false)
    setError('')
    setOpen(false)
    return () => { tracker.version++ }
  }, [agentId, conversationId])
  const refresh = async () => {
    const id = ++request.current.version
    setLoading(true)
    setError('')
    try {
      const result = await externalCliSettingsApi.getExternalCliAccount(agentId, conversationId)
      if (request.current.version === id) setStatus(result)
    } catch (error) {
      if (request.current.version === id) setError(error instanceof Error ? error.message : String(error))
    } finally {
      if (request.current.version === id) setLoading(false)
    }
  }
  // 圆环要有数据才能画：进会话 / 换 CLI 就读一次，不等用户点开。
  useEffect(() => {
    if (agentId === 'codex' || agentId === 'claude') void refresh()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [agentId, conversationId])
  if (agentId !== 'codex' && agentId !== 'claude') return null
  // 非账号登录（API key 等）读不到额度：圆环没有可画的数据，整个不显示。
  if (!status?.windows.length) return null
  const resetLabel = (value: string | number | null) => {
    if (value == null) return zh ? '重置时间未知' : 'Reset time unavailable'
    const date = new Date(typeof value === 'number' ? value * 1000 : value)
    return Number.isNaN(date.getTime()) ? (zh ? '重置时间未知' : 'Reset time unavailable')
      : `${zh ? '重置于' : 'Resets'} ${date.toLocaleString()}`
  }
  const windowName = (name: string, mins: number | null) => {
    const labels: Record<string, string> = zh ? { five_hour: '5 小时', seven_day: '7 天', seven_day_opus: 'Opus · 7 天', seven_day_sonnet: 'Sonnet · 7 天', seven_day_oauth_apps: 'OAuth 应用 · 7 天' } : {}
    return labels[name] ?? (mins ? `${name.replace(/primary|secondary/g, '').trim()} · ${mins < 1440 ? `${mins / 60}h` : `${mins / 1440}d`}` : name)
  }
  const remaining = (usedPercent: number) => Math.max(0, Math.min(100, 100 - usedPercent))
  const minRemaining = Math.min(...status.windows.map(w => remaining(w.usedPercent)))
  const tone = (left: number) => left <= 10 ? '#ef4444' : left <= 30 ? '#f59e0b' : 'var(--accent)'
  const circumference = 2 * Math.PI * 8
  return <div className="relative" data-tauri-drag-region="false">
    <button type="button" aria-label={zh ? 'CLI 账号与额度' : 'CLI account and usage'} aria-expanded={open}
      title={`${zh ? '剩余' : 'Remaining'} ${Math.round(minRemaining)}%`}
      className="grid size-7 shrink-0 place-items-center rounded-full text-neutral-600 transition-colors hover:bg-neutral-100 active:scale-[0.97]"
      onClick={event => {
        const rect = event.currentTarget.getBoundingClientRect()
        setAnchor({ left: rect.left, top: rect.bottom + 8 })
        setOpen(!open)
        if (!open) void refresh()
      }}>
      <svg viewBox="0 0 20 20" className="size-5 -rotate-90" aria-hidden="true">
        <circle cx="10" cy="10" r="8" fill="none" strokeWidth="3" className="stroke-black/[0.16] dark:stroke-white/[0.20]" />
        {minRemaining > 0 && <circle cx="10" cy="10" r="8" fill="none" stroke={tone(minRemaining)} strokeWidth="3" strokeLinecap="round"
          strokeDasharray={circumference} strokeDashoffset={circumference * (1 - minRemaining / 100)}
          className="transition-[stroke-dashoffset,stroke] duration-500 ease-out" />}
      </svg>
    </button>
    {open && createPortal(<>
      <div className="fixed inset-0 z-[1000]" aria-hidden onClick={() => setOpen(false)} />
      <section ref={menu} style={position} aria-label={zh ? '账号与额度' : 'Account and usage'}
      className="kv-menu custom-scrollbar fixed z-[1001] flex max-h-[70vh] w-[248px] max-w-[calc(100vw-32px)] flex-col gap-2.5 overflow-y-auto p-2.5 text-xs">
      <div className="flex items-center justify-between gap-2">
        <div className="min-w-0">
          <strong className="text-[13px]">{agentId === 'codex' ? 'Codex' : 'Claude Code'}</strong>
          <p className="truncate text-neutral-500 dark:text-neutral-400" title={status.account?.email ?? undefined}>
            {[status.account?.email, status.account?.plan].filter(Boolean).join(' · ') || (zh ? '账号信息未知' : 'Account unavailable')}
          </p>
        </div>
        <IconButton size="xs" variant="ghost" label={zh ? '刷新账号与额度' : 'Refresh account and usage'} disabled={loading} onClick={() => void refresh()}><RefreshCw size={13} /></IconButton>
      </div>
      {error && <div role="alert"><p>{error}</p><Button size="sm" onClick={() => void refresh()}>{zh ? '重试' : 'Retry'}</Button></div>}
      {status.windows.map((window, index) => {
        const left = remaining(window.usedPercent)
        return <div key={`${window.name}-${index}`} className="flex flex-col gap-1">
          <div className="flex justify-between gap-3"><span>{windowName(window.name, window.windowDurationMins)}</span><span className="tabular-nums">{Math.round(left)}% {zh ? '剩余' : 'remaining'}</span></div>
          <div className="h-1.5 overflow-hidden rounded-full bg-neutral-200/80 dark:bg-white/15" role="progressbar" aria-valuenow={Math.round(left)} aria-valuemin={0} aria-valuemax={100}>
            <div className="h-full rounded-full transition-[width] duration-500" style={{ width: `${left}%`, background: tone(left) }} />
          </div>
          <p className="text-[11px] text-neutral-500 dark:text-neutral-400">{resetLabel(window.resetsAt)}</p>
        </div>
      })}
      {status.credits && <p className="flex justify-between gap-3"><span>{status.credits.label}</span><span className="tabular-nums">{status.credits.unlimited ? (zh ? '不限量' : 'Unlimited') : status.credits.balance != null ? Number(status.credits.balance).toLocaleString(undefined, { maximumFractionDigits: 2 }) : `${status.credits.used ?? '—'} / ${status.credits.limit ?? '—'} ${status.credits.currency ?? ''}`}</span></p>}
      {status.unavailable && <p className="text-neutral-500 dark:text-neutral-400">{status.unavailable}</p>}
      <p className="text-[11px] text-neutral-500 dark:text-neutral-400">{zh ? '更新于' : 'Updated'} {new Date(status.checkedAt * 1000).toLocaleTimeString()}</p>
    </section></>, document.body)}
  </div>
}
