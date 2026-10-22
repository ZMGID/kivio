import { useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { RefreshCw, UserRound } from 'lucide-react'
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
  if (agentId !== 'codex' && agentId !== 'claude') return null
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
  return <div className="relative">
    <IconButton variant="ghost" size="sm" label={zh ? 'CLI 账号与额度' : 'CLI account and usage'} aria-expanded={open}
      onClick={event => {
        const rect = event.currentTarget.getBoundingClientRect()
        setAnchor({ left: rect.left, top: rect.bottom + 8 })
        setOpen(!open)
        if (!open) void refresh()
      }}><UserRound size={15} /></IconButton>
    {open && createPortal(<>
      <div className="fixed inset-0 z-[1000]" aria-hidden onClick={() => setOpen(false)} />
      <section ref={menu} style={position} aria-label={zh ? '账号与额度' : 'Account and usage'}
      className="kv-menu custom-scrollbar fixed z-[1001] flex max-h-[70vh] w-[320px] max-w-[calc(100vw-32px)] flex-col gap-3 overflow-y-auto p-3 text-sm">
      <div className="flex items-center justify-between gap-2">
        <strong>{agentId === 'codex' ? 'Codex' : 'Claude Code'}</strong>
        <IconButton size="xs" variant="ghost" label={zh ? '刷新账号与额度' : 'Refresh account and usage'} disabled={loading} onClick={() => void refresh()}><RefreshCw size={13} /></IconButton>
      </div>
      {loading && <p role="status">{zh ? '正在读取 CLI 账号…' : 'Reading CLI account…'}</p>}
      {error && <div role="alert"><p>{error}</p><Button size="sm" onClick={() => void refresh()}>{zh ? '重试' : 'Retry'}</Button></div>}
      {status && <>
        <div className="break-words">
          <p>{status.account?.email ?? (zh ? 'CLI 未提供账号邮箱' : 'No account email provided')}</p>
          <p className="text-neutral-500 dark:text-neutral-400">{[status.account?.type, status.account?.plan].filter(Boolean).join(' · ') || (zh ? '账号类型未知' : 'Account type unavailable')}</p>
        </div>
        {status.windows.map((window, index) => <div key={`${window.name}-${index}`}>
          <div className="flex justify-between gap-3"><span>{windowName(window.name, window.windowDurationMins)}</span><span>{Math.round(100 - window.usedPercent)}% {zh ? '剩余' : 'remaining'}</span></div>
          <p className="text-xs text-neutral-500 dark:text-neutral-400">{resetLabel(window.resetsAt)}</p>
        </div>)}
        {status.credits && <p>{status.credits.label} · {status.credits.unlimited ? (zh ? '不限量' : 'Unlimited') : status.credits.balance != null ? status.credits.balance : `${status.credits.used ?? '—'} / ${status.credits.limit ?? '—'} ${status.credits.currency ?? ''}`}</p>}
        {status.unavailable && <p className="text-neutral-500 dark:text-neutral-400">{status.unavailable}</p>}
        <p className="text-xs text-neutral-500 dark:text-neutral-400">{zh ? '更新于' : 'Updated'} {new Date(status.checkedAt * 1000).toLocaleTimeString()}</p>
      </>}
    </section></>, document.body)}
  </div>
}
