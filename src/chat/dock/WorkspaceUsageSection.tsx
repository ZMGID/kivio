// 工作区面板的「用量」与「上下文窗口」两段，参照 T3 Code 的 ThreadDetailsUsage / ThreadDetailsContext：
// 一行标签 + 细进度条 + 数值，悬停看重置时间等细节。只在面板可见且该项打开时才读取额度。
import { useEffect, useState } from 'react'
import { i18n, type Lang } from '../../components/i18n'
import { api, CHATGPT_USAGE_URL, type ModelProvider } from '../../api/tauri'
import { Button } from '../../components/Button'
import { notifyError } from '../../components/appNoticeStore'
import { peekSettings } from '../../api/settingsCache'
import { cliRemaining, cliResetTime, cliWindowLabel } from '../../api/cliAccountFormat'
import { useCliAccount } from '../../api/useCliAccount'
import { useChatCliAccountScope, type CliAccountConversation } from '../useChatCliAccountScope'
import { reportedContextTokens, CONTEXT_CRITICAL_PERCENT, CONTEXT_WARNING_PERCENT } from '../contextPanel'
import { formatTokens } from '../../utils/tokens'
import type { ConversationContextState } from '../types'
import { PanelSection } from './WorkspacePanelParts'

export type WorkspaceUsageSource = {
  providerId: string
  externalAgentId: string | null
  /** 外部 CLI 支持读取账号额度。 */
  externalAccount: boolean
  conversation: CliAccountConversation | null
  draftProjectId?: string | null
}

type UsageWindow = { label: string; remaining: number | null; detail: string }

const OAUTH_USAGE_PROVIDERS = new Set(['kimi', 'antigravity'])

function MeterRow({ label, value, percent, tone, detail }: {
  label: string
  value: string
  percent: number | null
  tone: 'normal' | 'warning' | 'critical'
  detail?: string
}) {
  const bar = tone === 'critical' ? 'bg-red-500' : tone === 'warning' ? 'bg-amber-500' : 'bg-[var(--accent)]'
  return (
    <div className="flex h-8 items-center gap-2.5 px-2.5 text-[12px]" title={detail ? `${label} · ${detail}` : label}>
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
      <span className="shrink-0 tabular-nums text-foreground">{value}</span>
    </div>
  )
}

function Heading({ children }: { children: string }) {
  return <div className="px-2.5 pb-0.5 pt-1 text-[11px] font-medium text-neutral-400">{children}</div>
}

/** 读取当前模型账号的额度窗口；不支持额度的供应商返回空数组。 */
function useUsageWindows(source: WorkspaceUsageSource, active: boolean, lang: Lang) {
  const [oauthWindows, setOauthWindows] = useState<UsageWindow[] | null>(null)
  const zh = lang === 'zh'
  const t = i18n[lang]
  const { providerId, externalAgentId, externalAccount } = source
  const target = useChatCliAccountScope(source.conversation, source.draftProjectId)
  const { account, error, loading } = useCliAccount(active && externalAccount ? externalAgentId : null, target)
  useEffect(() => {
    setOauthWindows(null)
    if (!active || externalAgentId) return
    let cancelled = false
    const resetText = (value: string | number | null | undefined) => {
      const time = cliResetTime(value ?? null)
      return time === null ? '' : t.cliLimitsResetAt.replace('{time}', new Date(time).toLocaleString())
    }
    const load = async (): Promise<UsageWindow[]> => {
      const provider = peekSettings()?.providers?.find((item: ModelProvider) => item.id === providerId)
      const oauth = provider?.request.oauth
      if (!provider || !oauth?.credentialId || !OAUTH_USAGE_PROVIDERS.has(oauth.provider ?? '')) return []
      const usage = await api.providerOAuthUsage(provider)
      return usage.windows.map((window) => ({
        label: window.label,
        remaining: window.usedPercent === null ? null : cliRemaining(window.usedPercent),
        detail: resetText(window.resetsAt) || window.resetHint || '',
      }))
    }
    load().then((next) => { if (!cancelled) setOauthWindows(next) }).catch(() => { if (!cancelled) setOauthWindows([]) })
    return () => { cancelled = true }
  }, [active, externalAgentId, providerId, t])
  if (!active || !externalAgentId) return { windows: active ? oauthWindows : null, error: null }
  const windows = account?.windows.map(window => {
    const reset = cliResetTime(window.resetsAt)
    return {
      label: cliWindowLabel(window, zh),
      remaining: cliRemaining(window.usedPercent),
      detail: reset === null ? '' : t.cliLimitsResetAt.replace('{time}', new Date(reset).toLocaleString()),
    }
  }) ?? (loading ? null : [])
  return { windows, error: error ?? account?.unavailable ?? null }
}

export function WorkspaceUsageSection({ source, active, lang }: {
  source: WorkspaceUsageSource
  active: boolean
  lang: Lang
}) {
  const t = i18n[lang]
  const { windows, error } = useUsageWindows(source, active, lang)
  if (active && !source.externalAgentId && peekSettings()?.providers.find(item => item.id === source.providerId)?.request.oauth?.provider === 'chatgpt') {
    return <PanelSection>
      <Heading>{t.workspaceItemUsage}</Heading>
      <Button size="sm" variant="ghost" onClick={() => void api.openExternal(CHATGPT_USAGE_URL).catch(err => notifyError(String(err)))}>
        {lang === 'zh' ? '管理 ChatGPT 用量 ↗' : 'Manage ChatGPT usage ↗'}
      </Button>
    </PanelSection>
  }
  if (windows === null) return null
  return (
    <PanelSection>
      <Heading>{t.workspaceItemUsage}</Heading>
      {windows.length === 0 ? (
        <div className="px-2.5 pb-1.5 text-[12px] text-neutral-400">{error ?? t.workspaceUsageUnavailable}</div>
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
      {windows.length > 0 && error && (
        <div className="px-2.5 pb-1.5 text-[12px] text-neutral-500" title={error}>{t.cliLimitsStale}</div>
      )}
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
