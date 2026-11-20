import type { ExternalCliAccount } from './externalCliSettings'

export const cliRemaining = (usedPercent: number) => Math.max(0, Math.min(100, 100 - usedPercent))

export function cliResetTime(value: string | number | null): number | null {
  if (value === null) return null
  const time = typeof value === 'number' ? new Date(value * 1000).getTime() : Date.parse(value)
  return Number.isFinite(time) ? time : null
}

export function cliWindowLabel(window: ExternalCliAccount['windows'][number], zh: boolean): string {
  const { name, windowDurationMins: mins } = window
  const labels: Record<string, string> = zh
    ? {
      five_hour: '5 小时', seven_day: '7 天', seven_day_opus: 'Opus · 7 天',
      seven_day_sonnet: 'Sonnet · 7 天', seven_day_oauth_apps: 'OAuth 应用 · 7 天',
      '5-hour limit': '5 小时', 'Weekly limit': '每周', 'Monthly limit': '每月',
      Weekly: '每周', Monthly: '每月', Subscription: '订阅',
    }
    : {
      five_hour: '5 hours', seven_day: 'Weekly', seven_day_opus: 'Opus · Weekly',
      seven_day_sonnet: 'Sonnet · Weekly', seven_day_oauth_apps: 'OAuth apps · Weekly',
    }
  if (labels[name]) return labels[name]
  if (zh && name.endsWith(' · 5h')) return `${name.slice(0, -2)}5 小时`
  if (zh && name.endsWith(' · 7d')) return `${name.slice(0, -2)}7 天`
  if (!mins || !/ (primary|secondary)$/.test(name)) return name
  const duration = mins < 60 ? `${mins}${zh ? ' 分钟' : 'm'}`
    : mins < 1440 ? `${mins / 60}${zh ? ' 小时' : 'h'}` : `${mins / 1440}${zh ? ' 天' : 'd'}`
  const group = name.replace(/ (primary|secondary)$/, '')
  return `${group === 'codex' ? 'Codex' : group} · ${duration}`
}

export function cliCreditAmount(credits: NonNullable<ExternalCliAccount['credits']>, zh: boolean): string {
  if (credits.unlimited) return zh ? '不限量' : 'Unlimited'
  if (credits.balance != null) {
    const amount = Number(credits.balance)
    return Number.isFinite(amount) ? amount.toLocaleString(undefined, { maximumFractionDigits: 2 }) : credits.balance
  }
  return `${credits.used ?? '—'} / ${credits.limit ?? '—'} ${credits.currency ?? ''}`.trim()
}
