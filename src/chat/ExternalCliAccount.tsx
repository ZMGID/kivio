import { useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { RefreshCw } from 'lucide-react'
import { Button, IconButton } from '../components/Button'
import { i18n, useLang } from '../components/i18n'
import type { DetectedExternalAgent } from '../api/externalCliSettings'
import { cliCreditAmount, cliRemaining, cliResetTime, cliWindowLabel } from '../api/cliAccountFormat'
import { useCliAccount } from '../api/useCliAccount'
import { usePopoverMenu } from './usePopoverMenu'
import { useClampedMenuPosition } from './useClampedMenuPosition'

/** One read-only account view shared by the main and independent chat titlebars. */
export function ExternalCliAccount({ agent, conversationId }: {
  agent: Pick<DetectedExternalAgent, 'id' | 'name' | 'capabilities'> | undefined
  conversationId?: string | null
}) {
  const agentId = agent?.capabilities.account === 'native' ? agent.id : undefined
  const lang = useLang()
  const zh = lang === 'zh'
  const t = i18n[lang]
  const [open, setOpen] = useState(false)
  const { account: status, loading, error, refresh } = useCliAccount(agentId, conversationId)
  const [anchor, setAnchor] = useState({ left: 0, top: 0 })
  const menu = useRef<HTMLDivElement>(null)
  const hasWindows = Boolean(status?.windows.length)
  const position = useClampedMenuPosition(menu, anchor)
  usePopoverMenu(open, () => setOpen(false), menu)
  useEffect(() => { setOpen(false) }, [agentId, conversationId, hasWindows])
  // 非账号登录（API key 等）读不到额度：圆环没有可画的数据，整个不显示。
  if (!status?.windows.length) return null
  const resetLabel = (value: string | number | null) => {
    const time = cliResetTime(value)
    return time === null ? t.cliLimitsResetUnknown : t.cliLimitsResetAt.replace('{time}', new Date(time).toLocaleString())
  }
  const minRemaining = Math.min(...status.windows.map(w => cliRemaining(w.usedPercent)))
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
      }}>
      <svg viewBox="0 0 20 20" className="size-5 -rotate-90" aria-hidden="true">
        <circle cx="10" cy="10" r="8" fill="none" strokeWidth="3" className="stroke-black/[0.16] dark:stroke-white/[0.20]" />
        {minRemaining > 0 && <circle cx="10" cy="10" r="8" fill="none" stroke={tone(minRemaining)} strokeWidth="3" strokeLinecap="round"
          strokeDasharray={circumference} strokeDashoffset={circumference * (1 - minRemaining / 100)}
          className="transition-[stroke-dashoffset,stroke] duration-500 ease-kv-out" />}
      </svg>
    </button>
    {open && createPortal(<>
      <div className="fixed inset-0 z-[1000]" aria-hidden onClick={() => setOpen(false)} />
      <section ref={menu} style={position} aria-label={zh ? '账号与额度' : 'Account and usage'}
      className="kv-menu custom-scrollbar fixed z-[1001] flex max-h-[70vh] w-[248px] max-w-[calc(100vw-32px)] flex-col gap-2.5 overflow-y-auto p-2.5 text-xs">
      <div className="flex items-center justify-between gap-2">
        <div className="min-w-0">
          <strong className="text-[13px]">{agent?.name}</strong>
          <p className="truncate text-neutral-500" title={status.account?.email ?? undefined}>
            {[status.account?.email, status.account?.plan].filter(Boolean).join(' · ') || (zh ? '账号信息未知' : 'Account unavailable')}
          </p>
        </div>
        <IconButton size="xs" variant="ghost" label={zh ? '刷新账号与额度' : 'Refresh account and usage'} disabled={loading} onClick={() => void refresh()}><RefreshCw size={13} /></IconButton>
      </div>
      {error && <div role="status" title={error}><p>{t.cliLimitsStale}</p><Button size="sm" disabled={loading} onClick={() => void refresh()}>{zh ? '重试' : 'Retry'}</Button></div>}
      {status.windows.map((window, index) => {
        const left = cliRemaining(window.usedPercent)
        return <div key={`${window.name}-${index}`} className="flex flex-col gap-1">
          <div className="flex justify-between gap-3"><span>{cliWindowLabel(window, zh)}</span><span className="tabular-nums">{Math.round(left)}% {zh ? '剩余' : 'remaining'}</span></div>
          <div className="h-1.5 overflow-hidden rounded-full bg-neutral-200/80 dark:bg-white/15" role="progressbar" aria-valuenow={Math.round(left)} aria-valuemin={0} aria-valuemax={100}>
            <div className="h-full rounded-full transition-[width] duration-500 ease-kv-out" style={{ width: `${left}%`, background: tone(left) }} />
          </div>
          <p className="text-[11px] text-neutral-500">{resetLabel(window.resetsAt)}</p>
        </div>
      })}
      {status.credits && <p className="flex justify-between gap-3"><span>{status.credits.label}</span><span className="tabular-nums">{cliCreditAmount(status.credits, zh)}</span></p>}
      {status.unavailable && <p className="text-neutral-500">{status.unavailable}</p>}
      <p className="text-[11px] text-neutral-500">{zh ? '更新于' : 'Updated'} {new Date(status.checkedAt * 1000).toLocaleTimeString()}</p>
    </section></>, document.body)}
  </div>
}
