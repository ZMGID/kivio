import { useEffect, useRef, useState } from 'react'
import { api, clampedActiveKeyIndex, type ModelProvider, type ProviderOAuthUsage } from '../api/tauri'

/**
 * 供应商账户额度：OAuth 会话、带额度接口的 API Key 套餐与可查余额的 API Key 共用一份读取、缓存与文案。
 * `balance` 只有账户余额，不是使用限制，只在供应商详情显示。
 */
export type AccountUsageKind = 'kimi' | 'codex' | 'antigravity' | 'glm' | 'opencode-go' | 'minimax' | 'balance'

const USAGE_FRESHNESS_MS = 60_000
const USAGE_CACHE_LIMIT = 32
const usageByAccount = new Map<string, { data: ProviderOAuthUsage; expiresAt: number }>()
const pendingUsage = new Map<string, Promise<ProviderOAuthUsage>>()

const BALANCE_HOSTS = new Set([
  'api.deepseek.com', 'api.moonshot.cn', 'api.moonshot.ai', 'api.siliconflow.cn', 'api.siliconflow.com', 'openrouter.ai',
])
const MINIMAX_HOSTS = new Set(['api.minimaxi.com', 'api.minimax.cn', 'api.minimax.io'])

/** 与 Rust `api_key_vendor` 一致：按官方端点识别 API Key 的额度来源，不认供应商名称。 */
function apiKeyUsageKind(baseUrl: string, key: string): AccountUsageKind | null {
  let url: URL
  try {
    url = new URL(baseUrl.trim())
  } catch {
    return null
  }
  if (url.protocol !== 'https:') return null
  const host = url.hostname
  const path = url.pathname
  if ((host === 'open.bigmodel.cn' || host === 'api.z.ai') && path.startsWith('/api/coding/paas')) return 'glm'
  if (host === 'opencode.ai' && ['/zen/go', '/inference/go'].some(prefix => path === prefix || path.startsWith(`${prefix}/`))) return 'opencode-go'
  if (host === 'api.kimi.com' && path.startsWith('/coding')) return 'kimi'
  // 按量付费的 MiniMax Key（sk-api-）与 Token Plan 同一 Base URL，但没有套餐窗口。
  if (MINIMAX_HOSTS.has(host)) return key.startsWith('sk-api-') ? null : 'minimax'
  return BALANCE_HOSTS.has(host) ? 'balance' : null
}

/** API Key 套餐的使用限制（GLM、OpenCode Go、Kimi for Coding、MiniMax Token Plan）；余额与 OAuth 不算。 */
export function isApiKeyPlanUsage(provider: ModelProvider): boolean {
  const kind = accountUsageKind(provider)
  return !provider.request.oauth && kind !== null && kind !== 'balance'
}

function preferredApiKey(provider: ModelProvider): string | null {
  const keys = provider.apiKeys
  const key = keys[clampedActiveKeyIndex(keys, provider.activeKeyIndex)]?.trim()
  return key || keys.map(item => item.trim()).find(Boolean) || null
}

export function accountUsageKind(provider: ModelProvider): AccountUsageKind | null {
  const auth = provider.request.oauth
  if (auth) {
    const kind = auth.provider
    return auth.credentialId && (kind === 'kimi' || kind === 'codex' || kind === 'antigravity') ? kind : null
  }
  const key = preferredApiKey(provider)
  return key ? apiKeyUsageKind(provider.baseUrl, key) : null
}

/** 额度归属的账户标识；切换凭据、Key 或请求端点都会换一份读数。不支持时为 null。 */
export function accountUsageKey(provider: ModelProvider): string | null {
  const kind = accountUsageKind(provider)
  if (!kind) return null
  const account = provider.request.oauth ? provider.request.oauth.credentialId : preferredApiKey(provider)
  return JSON.stringify([kind, account, provider.baseUrl, provider.apiFormat])
}

function fetchUsage(provider: ModelProvider, usageKey: string) {
  const pending = pendingUsage.get(usageKey)
  if (pending) return pending
  const result = api.providerOAuthUsage(provider).then(data => {
    usageByAccount.delete(usageKey)
    usageByAccount.set(usageKey, { data, expiresAt: Date.now() + USAGE_FRESHNESS_MS })
    if (usageByAccount.size > USAGE_CACHE_LIMIT) {
      const oldest = usageByAccount.keys().next()
      if (!oldest.done) usageByAccount.delete(oldest.value)
    }
    return data
  }).finally(() => {
    pendingUsage.delete(usageKey)
  })
  pendingUsage.set(usageKey, result)
  return result
}

/**
 * 读取一个账户的额度。调用方须以 `usageKey` 作为组件 key，换账户即重挂载，
 * 这样迟到结果和刷新计数都只属于发起它的账户。`refreshSignal` 递增时由外层强制刷新。
 */
export function useAccountUsage(usageKey: string, provider: ModelProvider, refreshSignal = 0) {
  const [data, setData] = useState(() => usageByAccount.get(usageKey)?.data)
  const [status, setStatus] = useState<'idle' | 'loading' | 'error'>('idle')
  const [refreshCount, setRefreshCount] = useState(0)
  const current = useRef(provider)
  current.current = provider
  const force = refreshCount + refreshSignal
  useEffect(() => {
    const cached = usageByAccount.get(usageKey)
    if (cached) setData(cached.data)
    if (!force && cached && cached.expiresAt > Date.now() && !pendingUsage.has(usageKey)) return

    let cancelled = false
    setStatus('loading')
    void fetchUsage(current.current, usageKey).then(result => {
      if (cancelled) return
      setData(result)
      setStatus('idle')
    }).catch(() => {
      if (!cancelled) setStatus('error')
    })
    return () => { cancelled = true }
  }, [usageKey, force])
  return { data, status, refresh: () => setRefreshCount(n => n + 1) }
}

export function accountUsageLabel(label: string, zh: boolean, suffix = true): string {
  if (!zh) return label === '5h' ? '5 hours' : label
  const split = label.lastIndexOf(' · ')
  const group = split < 0 ? '' : label.slice(0, split)
  const span = split < 0 ? label : label.slice(split + ' · '.length)
  const hours = /^(\d+(?:\.\d+)?)h$/.exec(span)?.[1]
  const days = /^(\d+)d$/.exec(span)?.[1]
  const name = ({ Weekly: '每周', Monthly: '每月', Daily: '每日', Session: '会话' } as Record<string, string>)[span]
    ?? (hours ? `${hours} 小时` : days ? `${days} 天` : null)
  const prefix = group === 'Code review' ? '代码审查 · ' : group === 'MCP' ? (suffix ? 'MCP ' : 'MCP · ') : group ? `${group} · ` : ''
  if (name) return `${prefix}${name}${suffix ? '额度' : ''}`
  return label.replace('Code review · ', '代码审查 · ')
}

/** 窗口标签对应的周期时长（分钟），用于使用限制里的周期刻度；每月按 30 天估算。 */
export function accountWindowMinutes(label: string): number | null {
  const span = label.split(' · ').pop() ?? label
  const fixed: Record<string, number> = { Daily: 1440, Weekly: 10_080, Monthly: 43_200 }
  if (fixed[span]) return fixed[span]
  const hours = /^(\d+(?:\.\d+)?)h$/.exec(span)?.[1]
  if (hours) return Number(hours) * 60
  const days = /^(\d+)d$/.exec(span)?.[1]
  return days ? Number(days) * 1440 : null
}

/** 余额行的文案：金额按币种格式化。 */
export function accountBalanceText(balance: { label: string; amount: number; currency: string }, zh: boolean, lang: string): string {
  const names: Record<string, [string, string]> = { Balance: ['余额', 'Balance'], Remaining: ['剩余额度', 'Remaining'], Used: ['已用', 'Used'] }
  const name = names[balance.label]?.[zh ? 0 : 1] ?? balance.label
  let amount: string
  try {
    amount = balance.amount.toLocaleString(lang, { style: 'currency', currency: balance.currency })
  } catch {
    amount = `${balance.amount.toLocaleString(lang)} ${balance.currency}`
  }
  return `${name} ${amount}`
}
