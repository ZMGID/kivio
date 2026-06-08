import { useCallback, useEffect, useMemo, useState } from 'react'
import { RefreshCw, Trash2 } from 'lucide-react'
import {
  api,
  type UsageGroupStats,
  type UsageRange,
  type UsageRecord,
  type UsageStatsResponse,
  type UsageTrendPoint,
} from '../api/tauri'
import { Input, Select, SettingsGroup } from './components'

type UsageView = 'logs' | 'providers' | 'models'

type UsageStatsPanelProps = {
  lang: string
}

const SOURCE_OPTIONS = [
  'all',
  'chat',
  'translator',
  'screenshot_translation',
  'lens',
  'chat_title_summary',
  'chat_compression',
  'chat_aux_vision',
  'chat_image_generation',
]

const STATUS_OPTIONS = ['all', 'success', 'error', 'cancelled', 'missing_usage']

function sourceLabel(source: string, lang: string) {
  const zh: Record<string, string> = {
    all: '全部来源',
    chat: 'Chat',
    translator: '输入翻译',
    screenshot_translation: '快速翻译',
    lens: 'Lens',
    chat_title_summary: '标题总结',
    chat_compression: '上下文压缩',
    chat_aux_vision: '辅助视觉',
    chat_image_generation: '图片生成',
  }
  const en: Record<string, string> = {
    all: 'All sources',
    chat: 'Chat',
    translator: 'Input translation',
    screenshot_translation: 'Quick translation',
    lens: 'Lens',
    chat_title_summary: 'Title summary',
    chat_compression: 'Context compression',
    chat_aux_vision: 'Aux vision',
    chat_image_generation: 'Image generation',
  }
  return (lang === 'zh' ? zh : en)[source] || source.replace(/_/g, ' ')
}

function statusLabel(status: string, lang: string) {
  const zh: Record<string, string> = {
    all: '全部状态',
    success: '成功',
    error: '失败',
    cancelled: '取消',
    missing_usage: '无 usage',
  }
  const en: Record<string, string> = {
    all: 'All statuses',
    success: 'Success',
    error: 'Error',
    cancelled: 'Cancelled',
    missing_usage: 'No usage',
  }
  return (lang === 'zh' ? zh : en)[status] || status
}

function formatCount(value?: number | null) {
  if (!value || !Number.isFinite(value)) return '0'
  return Math.round(value).toLocaleString()
}

function formatTokens(value?: number | null) {
  const n = Number(value ?? 0)
  if (!Number.isFinite(n) || n <= 0) return '0'
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(n >= 10_000_000 ? 1 : 2)}M`
  if (n >= 10_000) return `${(n / 1_000).toFixed(1)}K`
  return Math.round(n).toLocaleString()
}

function formatCost(value?: number | null) {
  const n = Number(value ?? 0)
  if (!Number.isFinite(n) || n <= 0) return '$0.00'
  if (n < 0.01) return `$${n.toFixed(4)}`
  return `$${n.toFixed(2)}`
}

function formatDuration(ms?: number | null) {
  const n = Number(ms ?? 0)
  if (!Number.isFinite(n) || n <= 0) return '--'
  if (n >= 1000) return `${(n / 1000).toFixed(1)}s`
  return `${Math.round(n)}ms`
}

function formatTime(seconds?: number | null, lang = 'zh') {
  if (!seconds) return '--'
  return new Date(seconds * 1000).toLocaleString(lang === 'zh' ? 'zh-CN' : 'en-US', {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  })
}

function recordTotalTokens(record: UsageRecord) {
  return record.totalTokens ?? ((record.inputTokens ?? 0) + (record.outputTokens ?? 0))
}

function SummaryTile({ label, value, sub }: { label: string; value: string; sub?: string }) {
  return (
    <div className="rounded-md border border-neutral-200 bg-white px-3 py-2.5 dark:border-neutral-800 dark:bg-neutral-950/35">
      <div className="text-[11px] font-medium text-neutral-500 dark:text-neutral-400">{label}</div>
      <div className="mt-1 truncate text-[19px] font-semibold leading-6 text-neutral-950 dark:text-neutral-50">{value}</div>
      {sub && <div className="mt-1 truncate text-[10.5px] text-neutral-500 dark:text-neutral-500">{sub}</div>}
    </div>
  )
}

function TrendChart({ points, lang }: { points: UsageTrendPoint[]; lang: string }) {
  const { path, costBars, maxTokens, maxCost } = useMemo(() => {
    const width = 420
    const height = 124
    const padX = 12
    const padY = 12
    const maxTokens = Math.max(1, ...points.map(point => point.totalTokens))
    const maxCost = Math.max(0, ...points.map(point => point.costUsd))
    const step = points.length > 1 ? (width - padX * 2) / (points.length - 1) : 0
    const coords = points.map((point, index) => {
      const x = padX + step * index
      const y = height - padY - (point.totalTokens / maxTokens) * (height - padY * 2)
      return `${x.toFixed(1)},${y.toFixed(1)}`
    })
    const barW = Math.max(2, Math.min(10, (width - padX * 2) / Math.max(points.length, 1) - 3))
    const costBars = points.map((point, index) => {
      const x = padX + step * index - barW / 2
      const h = maxCost > 0 ? (point.costUsd / maxCost) * (height - padY * 2) : 0
      return { x, y: height - padY - h, width: barW, height: Math.max(0, h) }
    })
    return {
      path: coords.length > 0 ? `M ${coords.join(' L ')}` : '',
      costBars,
      maxTokens,
      maxCost,
    }
  }, [points])

  if (points.length === 0) {
    return (
      <div className="flex h-36 items-center justify-center rounded-md border border-dashed border-neutral-200 text-[12px] text-neutral-500 dark:border-neutral-800 dark:text-neutral-400">
        {lang === 'zh' ? '暂无趋势数据' : 'No trend data'}
      </div>
    )
  }

  const firstLabel = points[0]?.label
  const lastLabel = points[points.length - 1]?.label

  return (
    <div className="rounded-md border border-neutral-200 bg-white p-3 dark:border-neutral-800 dark:bg-neutral-950/35">
      <div className="mb-2 flex items-center justify-between gap-3">
        <div className="flex items-center gap-3 text-[11px] text-neutral-500 dark:text-neutral-400">
          <span className="inline-flex items-center gap-1.5"><span className="h-2 w-2 rounded-full bg-blue-500" />Token</span>
          <span className="inline-flex items-center gap-1.5"><span className="h-2 w-2 rounded-sm bg-emerald-400/70" />USD</span>
        </div>
        <div className="truncate text-[11px] text-neutral-500 dark:text-neutral-400">
          {formatTokens(maxTokens)} / {formatCost(maxCost)}
        </div>
      </div>
      <svg viewBox="0 0 420 124" className="h-32 w-full overflow-visible" role="img" aria-label="usage trend">
        <line x1="12" y1="112" x2="408" y2="112" stroke="currentColor" className="text-neutral-200 dark:text-neutral-800" strokeWidth="1" />
        {costBars.map((bar, index) => (
          <rect
            key={`${points[index]?.date}-${index}`}
            x={bar.x}
            y={bar.y}
            width={bar.width}
            height={bar.height}
            rx="2"
            className="fill-emerald-400/45 dark:fill-emerald-300/35"
          />
        ))}
        {path && (
          <path
            d={path}
            fill="none"
            stroke="rgb(59 130 246)"
            strokeWidth="2.4"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        )}
      </svg>
      <div className="mt-1 flex justify-between text-[10.5px] text-neutral-500 dark:text-neutral-500">
        <span>{firstLabel}</span>
        <span>{lastLabel}</span>
      </div>
    </div>
  )
}

function GroupTable({ rows, lang, type }: { rows: UsageGroupStats[]; lang: string; type: 'provider' | 'model' }) {
  if (rows.length === 0) {
    return (
      <div className="kv-panel">
        <div className="kv-panel-body">{lang === 'zh' ? '暂无统计数据' : 'No usage data'}</div>
      </div>
    )
  }
  return (
    <div className="overflow-x-auto rounded-md border border-neutral-200 bg-white dark:border-neutral-800 dark:bg-neutral-950/35">
      <table className="min-w-[720px] w-full text-left text-[12px]">
        <thead className="border-b border-neutral-200 text-[10.5px] uppercase tracking-wide text-neutral-500 dark:border-neutral-800 dark:text-neutral-500">
          <tr>
            <th className="px-3 py-2 font-semibold">{type === 'provider' ? 'Provider' : 'Model'}</th>
            <th className="px-3 py-2 font-semibold">{lang === 'zh' ? '请求' : 'Req'}</th>
            <th className="px-3 py-2 font-semibold">{lang === 'zh' ? '成功率' : 'Success'}</th>
            <th className="px-3 py-2 font-semibold">Token</th>
            <th className="px-3 py-2 font-semibold">{lang === 'zh' ? '输入/输出' : 'In/Out'}</th>
            <th className="px-3 py-2 font-semibold">{lang === 'zh' ? '成本' : 'Cost'}</th>
            <th className="px-3 py-2 font-semibold">{lang === 'zh' ? '平均耗时' : 'Avg'}</th>
            <th className="px-3 py-2 font-semibold">{lang === 'zh' ? '最近' : 'Last'}</th>
          </tr>
        </thead>
        <tbody className="divide-y divide-neutral-100 dark:divide-neutral-800">
          {rows.map(row => {
            const successRate = row.requestCount > 0 ? row.successCount / row.requestCount : 0
            return (
              <tr key={row.id} className="text-neutral-800 dark:text-neutral-100">
                <td className="max-w-[220px] px-3 py-2">
                  <div className="truncate font-medium">{row.label}</div>
                  {type === 'model' && row.providerName && (
                    <div className="truncate text-[10.5px] text-neutral-500 dark:text-neutral-500">{row.providerName}</div>
                  )}
                </td>
                <td className="px-3 py-2 tabular-nums">{formatCount(row.requestCount)}</td>
                <td className="px-3 py-2 tabular-nums">{Math.round(successRate * 100)}%</td>
                <td className="px-3 py-2 tabular-nums">{formatTokens(row.totalTokens)}</td>
                <td className="px-3 py-2 tabular-nums">{formatTokens(row.inputTokens)} / {formatTokens(row.outputTokens)}</td>
                <td className="px-3 py-2 tabular-nums">{formatCost(row.costUsd)}</td>
                <td className="px-3 py-2 tabular-nums">{formatDuration(row.averageDurationMs)}</td>
                <td className="px-3 py-2 tabular-nums">{formatTime(row.lastUsedAt, lang)}</td>
              </tr>
            )
          })}
        </tbody>
      </table>
    </div>
  )
}

function LogsTable({ logs, lang }: { logs: UsageRecord[]; lang: string }) {
  if (logs.length === 0) {
    return (
      <div className="kv-panel">
        <div className="kv-panel-body">{lang === 'zh' ? '暂无请求日志' : 'No request logs'}</div>
      </div>
    )
  }
  return (
    <div className="overflow-x-auto rounded-md border border-neutral-200 bg-white dark:border-neutral-800 dark:bg-neutral-950/35">
      <table className="min-w-[920px] w-full text-left text-[12px]">
        <thead className="border-b border-neutral-200 text-[10.5px] uppercase tracking-wide text-neutral-500 dark:border-neutral-800 dark:text-neutral-500">
          <tr>
            <th className="px-3 py-2 font-semibold">{lang === 'zh' ? '时间' : 'Time'}</th>
            <th className="px-3 py-2 font-semibold">{lang === 'zh' ? '来源' : 'Source'}</th>
            <th className="px-3 py-2 font-semibold">Provider</th>
            <th className="px-3 py-2 font-semibold">Model</th>
            <th className="px-3 py-2 font-semibold">{lang === 'zh' ? '输入' : 'Input'}</th>
            <th className="px-3 py-2 font-semibold">{lang === 'zh' ? '输出' : 'Output'}</th>
            <th className="px-3 py-2 font-semibold">Token</th>
            <th className="px-3 py-2 font-semibold">{lang === 'zh' ? '成本' : 'Cost'}</th>
            <th className="px-3 py-2 font-semibold">{lang === 'zh' ? '耗时' : 'Time'}</th>
            <th className="px-3 py-2 font-semibold">{lang === 'zh' ? '状态' : 'Status'}</th>
            <th className="px-3 py-2 font-semibold">Usage</th>
          </tr>
        </thead>
        <tbody className="divide-y divide-neutral-100 dark:divide-neutral-800">
          {logs.map(record => (
            <tr key={record.id} className="text-neutral-800 dark:text-neutral-100">
              <td className="px-3 py-2 tabular-nums">{formatTime(record.createdAt, lang)}</td>
              <td className="px-3 py-2">
                <div className="truncate font-medium">{sourceLabel(record.source, lang)}</div>
                <div className="truncate text-[10.5px] text-neutral-500 dark:text-neutral-500">{record.operation}</div>
              </td>
              <td className="max-w-[140px] px-3 py-2 truncate">{record.providerName || record.providerId}</td>
              <td className="max-w-[180px] px-3 py-2 truncate font-mono text-[11.5px]">{record.model}</td>
              <td className="px-3 py-2 tabular-nums">{formatTokens(record.inputTokens)}</td>
              <td className="px-3 py-2 tabular-nums">{formatTokens(record.outputTokens)}</td>
              <td className="px-3 py-2 tabular-nums">{formatTokens(recordTotalTokens(record))}</td>
              <td className="px-3 py-2 tabular-nums">{record.costUsd == null ? '--' : formatCost(record.costUsd)}</td>
              <td className="px-3 py-2 tabular-nums">{formatDuration(record.durationMs)}</td>
              <td className="px-3 py-2">
                <span className={`kv-tag ${record.status === 'success' ? 'ok' : record.status === 'cancelled' ? 'warn' : 'danger'}`}>
                  {statusLabel(record.status, lang)}
                </span>
              </td>
              <td className="px-3 py-2">
                <span className={`kv-tag ${record.usageSource === 'missing' ? 'warn' : 'ok'}`}>
                  {record.usageSource === 'missing' ? (lang === 'zh' ? '缺失' : 'missing') : 'provider'}
                </span>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

export function UsageStatsPanel({ lang }: UsageStatsPanelProps) {
  const [range, setRange] = useState<UsageRange>('30d')
  const [view, setView] = useState<UsageView>('logs')
  const [source, setSource] = useState('all')
  const [status, setStatus] = useState('all')
  const [providerSearch, setProviderSearch] = useState('')
  const [modelSearch, setModelSearch] = useState('')
  const [stats, setStats] = useState<UsageStatsResponse | null>(null)
  const [loading, setLoading] = useState(false)
  const [clearing, setClearing] = useState(false)
  const [error, setError] = useState('')

  const loadStats = useCallback(async () => {
    setLoading(true)
    setError('')
    try {
      const data = await api.usageGetStats({
        range,
        source,
        status,
        providerSearch,
        modelSearch,
        limit: 120,
      })
      setStats(data)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setLoading(false)
    }
  }, [modelSearch, providerSearch, range, source, status])

  useEffect(() => {
    void loadStats()
  }, [loadStats])

  const clearStats = useCallback(async () => {
    const ok = window.confirm(lang === 'zh' ? '清空所有本地用量统计？' : 'Clear all local usage statistics?')
    if (!ok) return
    setClearing(true)
    setError('')
    try {
      await api.usageClear()
      await loadStats()
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setClearing(false)
    }
  }, [lang, loadStats])

  const summary = stats?.summary
  const reportedRatio = summary && summary.totalRequests > 0
    ? Math.round((summary.providerReportedRequests / summary.totalRequests) * 100)
    : 0

  return (
    <div className="space-y-3">
      <SettingsGroup title={lang === 'zh' ? '总览' : 'Overview'}>
        <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
          <div className="kv-seg">
            {(['7d', '30d', '90d', 'all'] as UsageRange[]).map(option => (
              <button
                key={option}
                type="button"
                className={range === option ? 'active' : ''}
                onClick={() => setRange(option)}
                data-tauri-drag-region="false"
              >
                {option === 'all' ? (lang === 'zh' ? '全部' : 'All') : option}
              </button>
            ))}
          </div>
          <div className="flex items-center gap-1.5">
            <button type="button" className="kv-btn sm" onClick={() => void loadStats()} disabled={loading} data-tauri-drag-region="false">
              <RefreshCw size={11} className={loading ? 'animate-spin' : ''} />
              {lang === 'zh' ? '刷新' : 'Refresh'}
            </button>
            <button type="button" className="kv-btn sm danger" onClick={() => void clearStats()} disabled={clearing || loading} data-tauri-drag-region="false">
              <Trash2 size={11} />
              {lang === 'zh' ? '清空' : 'Clear'}
            </button>
          </div>
        </div>

        {error && (
          <div className="kv-panel warn mb-3">
            <div className="kv-panel-body">{error}</div>
          </div>
        )}

        <div className="grid grid-cols-2 gap-2 lg:grid-cols-4">
          <SummaryTile label={lang === 'zh' ? '总 Token' : 'Total tokens'} value={formatTokens(summary?.totalTokens)} sub={`${formatCount(summary?.totalRequests)} ${lang === 'zh' ? '次请求' : 'requests'}`} />
          <SummaryTile label={lang === 'zh' ? '估算成本' : 'Estimated cost'} value={formatCost(summary?.totalCostUsd)} sub={lang === 'zh' ? '按本地模型价格估算' : 'From local model pricing'} />
          <SummaryTile label={lang === 'zh' ? '输入 / 输出' : 'Input / Output'} value={`${formatTokens(summary?.inputTokens)} / ${formatTokens(summary?.outputTokens)}`} sub={lang === 'zh' ? 'provider 返回 usage 时统计' : 'Provider usage only'} />
          <SummaryTile label={lang === 'zh' ? '可信度' : 'Coverage'} value={`${reportedRatio}%`} sub={`${formatCount(summary?.missingUsageRequests)} ${lang === 'zh' ? '条缺少 usage' : 'missing usage'}`} />
          <SummaryTile label={lang === 'zh' ? '缓存命中' : 'Cached input'} value={formatTokens(summary?.cachedInputTokens)} />
          <SummaryTile label={lang === 'zh' ? '缓存创建' : 'Cache creation'} value={formatTokens(summary?.cacheCreationInputTokens)} />
          <SummaryTile label={lang === 'zh' ? '推理 Token' : 'Reasoning'} value={formatTokens(summary?.reasoningTokens)} />
          <SummaryTile label={lang === 'zh' ? '平均耗时' : 'Avg duration'} value={formatDuration(summary?.averageDurationMs)} />
        </div>
      </SettingsGroup>

      <SettingsGroup title={lang === 'zh' ? '趋势' : 'Trend'}>
        <TrendChart points={stats?.trend ?? []} lang={lang} />
      </SettingsGroup>

      <SettingsGroup title={lang === 'zh' ? '明细' : 'Details'}>
        <div className="mb-3 flex flex-col gap-2">
          <div className="flex flex-wrap items-center gap-2">
            <div className="kv-seg">
              {[
                { id: 'logs' as const, label: lang === 'zh' ? '请求日志' : 'Logs' },
                { id: 'providers' as const, label: 'Provider' },
                { id: 'models' as const, label: lang === 'zh' ? '模型' : 'Models' },
              ].map(option => (
                <button
                  key={option.id}
                  type="button"
                  className={view === option.id ? 'active' : ''}
                  onClick={() => setView(option.id)}
                  data-tauri-drag-region="false"
                >
                  {option.label}
                </button>
              ))}
            </div>
            <Select
              className="w-40"
              value={source}
              onChange={setSource}
              options={SOURCE_OPTIONS.map(value => ({ value, label: sourceLabel(value, lang) }))}
            />
            <Select
              className="w-36"
              value={status}
              onChange={setStatus}
              options={STATUS_OPTIONS.map(value => ({ value, label: statusLabel(value, lang) }))}
            />
          </div>
          <div className="grid grid-cols-1 gap-2 md:grid-cols-2">
            <Input value={providerSearch} onChange={setProviderSearch} placeholder={lang === 'zh' ? '搜索 Provider' : 'Search provider'} />
            <Input value={modelSearch} onChange={setModelSearch} placeholder={lang === 'zh' ? '搜索模型' : 'Search model'} mono />
          </div>
        </div>

        {view === 'logs' && <LogsTable logs={stats?.logs ?? []} lang={lang} />}
        {view === 'providers' && <GroupTable rows={stats?.providerStats ?? []} lang={lang} type="provider" />}
        {view === 'models' && <GroupTable rows={stats?.modelStats ?? []} lang={lang} type="model" />}

        {stats && stats.totalLogs > (stats.logs?.length ?? 0) && view === 'logs' && (
          <div className="mt-2 text-[11px] text-neutral-500 dark:text-neutral-500">
            {lang === 'zh'
              ? `显示最近 ${stats.logs.length} / ${stats.totalLogs} 条`
              : `Showing latest ${stats.logs.length} / ${stats.totalLogs}`}
          </div>
        )}
      </SettingsGroup>
    </div>
  )
}
