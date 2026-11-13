// 工作区面板的「用量」与「上下文窗口」两段，参照 T3 Code 的 ThreadDetailsUsage / ThreadDetailsContext：
// 一行标签 + 细进度条 + 数值，悬停看重置时间等细节。只在面板可见且该项打开时才读取额度。
import { useEffect, useState } from 'react'
import { i18n, type Lang } from '../../components/i18n'
import { api, type ModelProvider } from '../../api/tauri'
import { peekSettings } from '../../api/settingsCache'
import { externalCliSettingsApi } from '../../api/externalCliSettings'
import { reportedContextTokens, CONTEXT_CRITICAL_PERCENT, CONTEXT_WARNING_PERCENT } from '../contextPanel'
import { formatTokens } from '../../utils/tokens'
import type { ConversationContextState } from '../types'
import { PanelSection } from './WorkspacePanelParts'

export type WorkspaceUsageSource = {
  providerId: string
  externalAgentId: string | null
  /** 外部 CLI 支持读取账号额度。 */
  externalAccount: boolean
}

type UsageWindow = { label: string; remaining: number | null; detail: string }

const OAUTH_USAGE_PROVIDERS = new Set(['kimi', 'codex', 'antigravity'])

function MeterRow({ label, value, percent, tone, detail }: {
  label: string
  value: string
  percent: number | null
  tone: 'normal' | 'warning' | 'critical'
  detail?: string
}) {
  const bar = tone === 'critical' ? 'bg-red-500' : tone === 'warning' ? 'bg-amber-500' : 'bg-[var(--accent)]'
  return (
    <div className="flex h-8 items-center gap-2.5 px-2.5 text-[12px]" title={detail}>
      <span className="w-20 shrink-0 truncate text-neutral-500">{label}</span>
      <span
        role="progressbar"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent ?? undefined}
        className="h-1 min-w-0 flex-1 overflow-hidden rounded-full bg-neutral-200/80 dark:bg-white/10"
      >
        {percent !== null && (
          <span className={`block h-full rounded-full transition-[width] duration-500 ease-kv-out ${bar}`} style={{ width: `${Math.max(2, Math.min(100, percent))}%` }} />
        )}
      </span>
      <span className="shrink-0 tabular-nums text-neutral-600 dark:text-neutral-300">{value}</span>
    </div>
  )
}

function Heading({ children }: { children: string }) {
  return <div className="px-2.5 pb-0.5 pt-1 text-[11px] font-medium text-neutral-400">{children}</div>
}

/** 读取当前模型账号的额度窗口；不支持额度的供应商返回空数组。 */
function useUsageWindows(source: WorkspaceUsageSource, conversationId: string | null, active: boolean, lang: Lang) {
  const [windows, setWindows] = useState<UsageWindow[] | null>(null)
  const zh = lang === 'zh'
  const { providerId, externalAgentId, externalAccount } = source
  useEffect(() => {
    setWindows(null)
    if (!active) return
    let cancelled = false
    const resetText = (value: string | number | null | undefined) => {
      if (value == null) return ''
      const date = new Date(typeof value === 'number' ? value * 1000 : value)
      return Number.isNaN(date.getTime()) ? '' : `${zh ? '重置于' : 'Resets'} ${date.toLocaleString()}`
    }
    const load = async (): Promise<UsageWindow[]> => {
      if (externalAgentId) {
        if (!externalAccount) return []
        const account = await externalCliSettingsApi.getExternalCliAccount(externalAgentId, conversationId)
        return account.windows.map((window) => ({
          label: window.name,
          remaining: Math.max(0, Math.min(100, 100 - window.usedPercent)),
          detail: resetText(window.resetsAt),
        }))
      }
      const provider = peekSettings()?.providers?.find((item: ModelProvider) => item.id === providerId)
      const oauth = provider?.request.oauth
      if (!provider || !oauth?.credentialId || !OAUTH_USAGE_PROVIDERS.has(oauth.provider ?? '')) return []
      const usage = await api.providerOAuthUsage(provider)
      return usage.windows.map((window) => ({
        label: window.label,
        remaining: window.usedPercent === null ? null : Math.max(0, Math.min(100, 100 - window.usedPercent)),
        detail: resetText(window.resetsAt) || window.resetHint || '',
      }))
    }
    load().then((next) => { if (!cancelled) setWindows(next) }).catch(() => { if (!cancelled) setWindows([]) })
    return () => { cancelled = true }
  }, [active, conversationId, externalAccount, externalAgentId, providerId, zh])
  return windows
}

export function WorkspaceUsageSection({ source, conversationId, active, lang }: {
  source: WorkspaceUsageSource
  conversationId: string | null
  active: boolean
  lang: Lang
}) {
  const t = i18n[lang]
  const windows = useUsageWindows(source, conversationId, active, lang)
  if (windows === null) return null
  return (
    <PanelSection>
      <Heading>{t.workspaceItemUsage}</Heading>
      {windows.length === 0 ? (
        <div className="px-2.5 pb-1.5 text-[12px] text-neutral-400">{t.workspaceUsageUnavailable}</div>
      ) : windows.map((window, index) => (
        <MeterRow
          key={`${window.label}-${index}`}
          label={window.label}
          value={window.remaining === null ? '—' : t.workspaceUsageLeft.replace('{n}', String(Math.round(window.remaining)))}
          percent={window.remaining}
          tone={window.remaining !== null && window.remaining <= 10 ? 'critical' : window.remaining !== null && window.remaining <= 30 ? 'warning' : 'normal'}
          detail={window.detail || undefined}
        />
      ))}
    </PanelSection>
  )
}

export function WorkspaceContextSection({ contextState, lang }: {
  contextState: ConversationContextState | null
  lang: Lang
}) {
  const t = i18n[lang]
  const used = reportedContextTokens(contextState)
  const max = contextState?.context_window_tokens ?? contextState?.contextWindowTokens ?? null
  if (used === null) return null
  const percent = max && max > 0 ? (used / max) * 100 : null
  const tone = percent === null ? 'normal' : percent >= CONTEXT_CRITICAL_PERCENT ? 'critical' : percent >= CONTEXT_WARNING_PERCENT ? 'warning' : 'normal'
  return (
    <PanelSection>
      <Heading>{t.workspaceItemContext}</Heading>
      <MeterRow
        label={t.workspaceContextUsed}
        value={max ? `${formatTokens(used)} / ${formatTokens(max)}` : formatTokens(used)}
        percent={percent}
        tone={tone}
        detail={percent === null ? undefined : t.workspaceContextPercent.replace('{n}', percent.toFixed(1))}
      />
    </PanelSection>
  )
}
