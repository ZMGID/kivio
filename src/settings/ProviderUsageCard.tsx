import { RefreshCw } from 'lucide-react'
import type { ModelProvider } from '../api/tauri'
import { Button } from '../components/Button'
import type { Lang } from '../components/i18n'
import { accountBalanceText, accountUsageKey, accountUsageLabel, isApiKeyPlanUsage, useAccountUsage } from './providerAccountUsage'

type UsageCardProps = { provider: ModelProvider; lang: Lang }

export function ProviderUsageCard({ provider, lang }: UsageCardProps) {
  const usageKey = accountUsageKey(provider)
  if (!usageKey) return null
  return <AccountUsageCard key={usageKey} usageKey={usageKey} provider={provider} lang={lang} />
}

function AccountUsageCard({ usageKey, provider, lang }: UsageCardProps & { usageKey: string }) {
  const { data, status, refresh } = useAccountUsage(usageKey, provider)
  const zh = lang === 'zh'
  const busy = status === 'loading'
  const label = (value: string) => accountUsageLabel(value, zh)
  const balanceOnly = !!data && data.windows.length === 0 && !!data.balances?.length
  const title = balanceOnly ? (zh ? '账户余额' : 'Account balance') : (zh ? '账户用量' : 'Account usage')
  return <section aria-label={title} className="kv-provider-usage">
    <div className="flex items-center justify-between gap-3">
      <div className="flex items-center gap-2">
        <h3 className="text-sm font-medium">{title}</h3>
        {data?.plan && <span className="rounded-full border border-[var(--border)] px-2 py-0.5 text-xs opacity-70">{data.plan}</span>}
      </div>
      <Button size="sm" disabled={busy} onClick={refresh}>
        <RefreshCw size={12} className={busy ? 'animate-spin' : ''} />{zh ? '刷新' : 'Refresh'}
      </Button>
    </div>
    {busy && <p className="mt-3 text-xs opacity-60" role="status">{zh ? '正在获取账户额度…' : 'Fetching account limits…'}</p>}
    {status === 'error' && <p className="mt-3 text-xs text-[var(--danger)]" role="alert">{zh ? '暂时无法获取用量，请检查网络或授权后重试。' : 'Could not fetch usage. Check your connection or authorization and retry.'}</p>}
    {data && data.windows.length === 0 && !data.balances?.length && <p className="mt-3 text-xs opacity-60">{zh ? '供应商暂未返回额度数据' : 'No quota data returned by the provider'}</p>}
    {provider.request.oauth?.provider === 'antigravity' && data && data.windows.length > 0 && <p className="mt-2 text-xs opacity-60">{zh ? '显示接口返回的额度组；部分账号可能不提供完整周额度。' : 'Reported quota groups; full weekly limits may not be available for every account.'}</p>}
    {data?.windows.map((window, i) => {
      const remaining = window.usedPercent === null ? null : Math.max(0, Math.min(100, 100 - window.usedPercent))
      return <div key={`${window.label}-${i}`} className="mt-3">
        <div className="mb-1.5 flex items-center justify-between gap-3 text-xs">
          <span>{label(window.label)}</span>
          <span className="tabular-nums">{remaining === null ? (zh ? '暂无比例' : 'Unavailable') : `${zh ? '剩余 ' : ''}${remaining.toFixed(0)}%${zh ? '' : ' remaining'}`}</span>
        </div>
        {remaining !== null && <div role="progressbar" aria-label={label(window.label)} aria-valuemin={0} aria-valuemax={100} aria-valuenow={remaining} className="h-1.5 overflow-hidden rounded-full bg-[var(--bg-active)]">
          <div className={`h-full rounded-full ${remaining <= 10 ? 'bg-[var(--warn)]' : 'bg-[var(--accent)]'}`} style={{ width: `${remaining}%` }} />
        </div>}
        <div className="mt-1 flex flex-wrap justify-between gap-x-3 text-[11px] opacity-60">
          {window.used !== null && window.limit !== null && <span>{zh ? '已用 ' : 'Used '}{window.used.toLocaleString()} / {window.limit.toLocaleString()}</span>}
          {window.resetsAt === null && window.resetHint && <span>{window.resetHint}</span>}
          {window.resetsAt !== null && <span>{zh ? '重置于 ' : 'Resets '}{new Date(window.resetsAt * 1000).toLocaleString(zh ? 'zh-CN' : 'en-US', { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })}</span>}
        </div>
      </div>
    })}
    {data?.balances?.map((balance, i) => <p key={`${balance.label}-${i}`} className="mt-3 text-sm tabular-nums">{accountBalanceText(balance, zh, zh ? 'zh-CN' : 'en-US')}</p>)}
    {data && <p className="mt-3 text-[11px] opacity-50">{balanceOnly
      ? (zh ? '账户余额 · 更新于 ' : 'Account balance · Updated ')
      : (zh ? '账户共享额度 · 更新于 ' : 'Shared account limits · Updated ')}{new Date(data.fetchedAt * 1000).toLocaleTimeString()}</p>}
  </section>
}

/** 供应商列表行内的额度摘要：只显示最紧的模型额度窗口，完整读数在供应商详情。 */
export function ProviderUsageSummary({ provider, lang }: UsageCardProps) {
  const usageKey = isApiKeyPlanUsage(provider) ? accountUsageKey(provider) : null
  if (!usageKey) return null
  return <UsageSummary key={usageKey} usageKey={usageKey} provider={provider} lang={lang} />
}

function UsageSummary({ usageKey, provider, lang }: UsageCardProps & { usageKey: string }) {
  const { data, status } = useAccountUsage(usageKey, provider)
  const zh = lang === 'zh'
  const readings = data?.windows.flatMap(window => window.usedPercent === null ? [] : [{
    label: accountUsageLabel(window.label, zh),
    remaining: Math.max(0, Math.min(100, 100 - window.usedPercent)),
    // 分组窗口（MCP、MiniMax 视频等）单独计量，不代表模型额度。
    model: !window.label.includes(' · '),
  }]) ?? []
  const tightest = readings.filter(item => item.model).sort((a, b) => a.remaining - b.remaining)[0]
  const detail = readings.map(item => `${item.label} ${zh ? '剩余 ' : ''}${Math.round(item.remaining)}%${zh ? '' : ' left'}`).join(' · ')
  if (!tightest) {
    if (status !== 'error') return null
    return <span className="kv-provider-card-usage" data-state="error" title={zh ? '暂时无法获取额度' : 'Could not fetch usage'}>
      {zh ? '额度未知' : 'Usage unavailable'}
    </span>
  }
  const remaining = Math.round(tightest.remaining)
  const level = remaining <= 10 ? 'critical' : remaining <= 30 ? 'warning' : 'normal'
  return <span className="kv-provider-card-usage" data-level={level} title={[data?.plan, detail].filter(Boolean).join(' · ')}>
    <span className="kv-provider-card-usage-label">{tightest.label}</span>
    <span className="kv-provider-card-usage-track" role="progressbar" aria-label={tightest.label}
      aria-valuemin={0} aria-valuemax={100} aria-valuenow={remaining}>
      <span style={{ width: `${remaining}%` }} />
    </span>
    <span className="tabular-nums">{zh ? `剩余 ${remaining}%` : `${remaining}% left`}</span>
  </span>
}
