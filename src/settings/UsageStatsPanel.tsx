import { ErrorNotice } from '../components/AppNotice'
import { useCallback, useEffect, useId, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent } from 'react'
import { ArrowDown, ArrowUp, ChevronLeft, ChevronRight, Database, RefreshCw, Trash2 } from 'lucide-react'
import {
  api,
  type UsageGroupStats,
  type UsageRange,
  type UsageRecord,
  type UsageStatsResponse,
  type UsageTrendPoint,
} from '../api/tauri'
import { Button } from '../components/Button'
import { Input, Select, SettingsGroup } from './components'
import { confirmDialog } from '../components/dialogQueue'
import { useDocumentDark } from '../theme/useDocumentDark'

type UsageView = 'logs' | 'providers' | 'models'

export type UsageStatsPanelProps = {
  lang: string
  view: 'app' | 'calls'
}

const SOURCE_OPTIONS = [
  'all',
  'cli',
  'chat',
  'translator',
  'screenshot_translation',
  'lens',
  'chat_title_summary',
  'chat_compression',
  'chat_aux_vision',
  'chat_aux_video',
  'chat_image_generation',
  'knowledge_base',
]

const STATUS_OPTIONS = ['all', 'success', 'error', 'cancelled', 'missing_usage']
const LOG_PAGE_SIZE = 15
const SEARCH_DEBOUNCE_MS = 250

function sourceLabel(source: string, lang: string) {
  const zh: Record<string, string> = {
    all: '全部来源',
    cli: '本地 CLI',
    chat: 'Chat',
    translator: '输入翻译',
    screenshot_translation: '快速翻译',
    lens: 'Lens',
    chat_title_summary: '标题总结',
    chat_compression: '上下文压缩',
    chat_aux_vision: '辅助视觉',
    chat_aux_video: '视频分析',
    chat_image_generation: '图片生成',
    knowledge_base: '知识库',
  }
  const en: Record<string, string> = {
    all: 'All sources',
    cli: 'Local CLI',
    chat: 'Chat',
    translator: 'Input translation',
    screenshot_translation: 'Quick translation',
    lens: 'Lens',
    chat_title_summary: 'Title summary',
    chat_compression: 'Context compression',
    chat_aux_vision: 'Aux vision',
    chat_aux_video: 'Video analysis',
    chat_image_generation: 'Image generation',
    knowledge_base: 'Knowledge base',
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

function formatOptionalTokens(value?: number | null) {
  if (value == null || !Number.isFinite(Number(value))) return '--'
  return formatTokens(value)
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

function pageRangeLabel(pageIndex: number, pageSize: number, total: number) {
  if (total <= 0) return '0 / 0'
  const start = pageIndex * pageSize + 1
  const end = Math.min(total, start + pageSize - 1)
  return `${start}-${end} / ${total}`
}

/** 推理强度显示原始 effort 值（Low/Medium/High/XHigh/Ultracode…）：这是模型/API 的
 *  协议术语，翻译成中文反而对不上文档与调试信息。 */
function formatReasoningEffort(value: string | null | undefined) {
  if (!value) return '--'
  const trimmed = value.trim()
  return trimmed.charAt(0).toUpperCase() + trimmed.slice(1)
}

function formatPercent(value?: number | null) {
  const n = Number(value ?? 0)
  if (!Number.isFinite(n) || n <= 0) return '0%'
  return `${Math.round(n * 100)}%`
}

type TrendSeriesKey = 'inputTokens' | 'outputTokens' | 'cacheCreationInputTokens' | 'cachedInputTokens'

const TREND_SERIES: {
  key: TrendSeriesKey
  labelZh: string
  labelEn: string
  stroke: string
  darkStroke: string
}[] = [
  { key: 'inputTokens', labelZh: '输入', labelEn: 'Input', stroke: '#2a78d6', darkStroke: '#3987e5' },
  { key: 'outputTokens', labelZh: '输出', labelEn: 'Output', stroke: '#1baf7a', darkStroke: '#34d399' },
  { key: 'cacheCreationInputTokens', labelZh: '缓存创建', labelEn: 'Cache creation', stroke: '#eda100', darkStroke: '#fbbf24' },
  { key: 'cachedInputTokens', labelZh: '缓存命中', labelEn: 'Cache read', stroke: '#0891b2', darkStroke: '#22d3ee' },
]

const HIT_RATE_COLOR = { stroke: '#7c3aed', darkStroke: '#a78bfa' }

function trendHitRate(point: UsageTrendPoint): number | null {
  if (point.inputTokens <= 0) return null
  return Math.min(1, point.cachedInputTokens / point.inputTokens)
}

/// 单调三次插值(Fritsch-Carlson)平滑折线:曲线严格保持在相邻数据点范围内,
/// 永不过冲——尖峰旁的零值段不会被拉出负凹(Catmull-Rom 会,已踩过)。
function smoothPath(coords: { x: number; y: number }[]): string {
  const n = coords.length
  if (n === 0) return ''
  if (n === 1) return `M ${coords[0].x.toFixed(1)} ${coords[0].y.toFixed(1)}`
  // 相邻段斜率
  const dx: number[] = []
  const slope: number[] = []
  for (let i = 0; i < n - 1; i++) {
    dx.push(coords[i + 1].x - coords[i].x)
    slope.push(dx[i] !== 0 ? (coords[i + 1].y - coords[i].y) / dx[i] : 0)
  }
  // 每个点的切线:相邻段斜率异号或有零 → 0(平台/极值处走平),否则调和平均
  const tangent: number[] = [slope[0]]
  for (let i = 1; i < n - 1; i++) {
    const a = slope[i - 1]
    const b = slope[i]
    tangent.push(a * b <= 0 ? 0 : (2 * a * b) / (a + b))
  }
  tangent.push(slope[n - 2])
  let path = `M ${coords[0].x.toFixed(1)} ${coords[0].y.toFixed(1)}`
  for (let i = 0; i < n - 1; i++) {
    const h = dx[i] / 3
    const c1x = coords[i].x + h
    const c1y = coords[i].y + tangent[i] * h
    const c2x = coords[i + 1].x - h
    const c2y = coords[i + 1].y - tangent[i + 1] * h
    path += ` C ${c1x.toFixed(1)} ${c1y.toFixed(1)}, ${c2x.toFixed(1)} ${c2y.toFixed(1)}, ${coords[i + 1].x.toFixed(1)} ${coords[i + 1].y.toFixed(1)}`
  }
  return path
}

function TrendChart({ points, lang }: { points: UsageTrendPoint[]; lang: string }) {
  const [hidden, setHidden] = useState<Set<string>>(() => new Set())
  const [hoverIndex, setHoverIndex] = useState<number | null>(null)
  const isDark = useDocumentDark()

  const gradientId = useId()
  const WIDTH = 760
  const HEIGHT = 240
  const PAD_L = 56
  const PAD_R = 48
  const PAD_T = 28
  const PAD_B = 12

  const geom = useMemo(() => {
    const visible = TREND_SERIES.filter(series => !hidden.has(series.key))
    const peak = Math.max(4, ...points.flatMap(point => visible.map(series => point[series.key])))
    // Leave headroom and use readable quarter ticks without changing the underlying values.
    const magnitude = 10 ** Math.floor(Math.log10(peak / 4))
    const tickStep = Math.ceil(peak / 4 / magnitude) * magnitude
    const maxTokens = tickStep * 4
    const step = points.length > 1 ? (WIDTH - PAD_L - PAD_R) / (points.length - 1) : 0
    const plotH = HEIGHT - PAD_T - PAD_B
    // 单点(如单日区间)居中,否则从左轴按步长铺开
    const singleX = PAD_L + (WIDTH - PAD_L - PAD_R) / 2
    const x = (index: number) => (points.length > 1 ? PAD_L + step * index : singleX)
    const yTokens = (value: number) => PAD_T + plotH - (value / maxTokens) * plotH
    const yRate = (rate: number) => PAD_T + plotH - rate * plotH
    const linePath = (values: (number | null)[]) => {
      const coords = values.flatMap((value, index) =>
        value == null ? [] : [{ x: x(index), y: yTokens(value) }],
      )
      return smoothPath(coords)
    }
    const seriesPaths = visible.map(series => ({
      ...series,
      path: linePath(points.map(point => point[series.key])),
    }))
    const rateCoords = points.flatMap((point, index) => {
      const rate = trendHitRate(point)
      return rate == null ? [] : [{ x: x(index), y: yRate(rate) }]
    })
    return { maxTokens, step, x, yTokens, yRate, seriesPaths, ratePath: smoothPath(rateCoords), plotH }
  }, [hidden, points])

  const toggleSeries = useCallback((key: string) => {
    setHidden(previous => {
      const next = new Set(previous)
      if (next.has(key)) next.delete(key)
      else next.add(key)
      return next
    })
  }, [])

  const onMove = useCallback(
    (event: ReactMouseEvent<SVGSVGElement>) => {
      if (points.length === 0) return
      const rect = event.currentTarget.getBoundingClientRect()
      const px = ((event.clientX - rect.left) / rect.width) * WIDTH
      const index = geom.step > 0 ? Math.round((px - PAD_L) / geom.step) : 0
      setHoverIndex(Math.max(0, Math.min(points.length - 1, index)))
    },
    [geom.step, points.length],
  )

  if (points.length === 0) {
    return (
      <div className="flex h-36 items-center justify-center rounded-md border border-dashed border-[var(--border)] text-[12px] text-[var(--text-muted)]">
        {lang === 'zh' ? '暂无趋势数据' : 'No trend data'}
      </div>
    )
  }

  const hoverPoint = hoverIndex != null ? points[hoverIndex] : null
  const hoverRate = hoverPoint ? trendHitRate(hoverPoint) : null
  const rateHidden = hidden.has('hitRate')
  const gridYs = [0, 0.25, 0.5, 0.75, 1].map(fraction => PAD_T + geom.plotH - fraction * geom.plotH)
  // tooltip 靠左半边时显示在指针右侧，反之左侧，避免出界。
  const tooltipLeftPct = hoverIndex != null ? (geom.x(hoverIndex) / WIDTH) * 100 : 0
  const tooltipFlip = tooltipLeftPct > 55

  return (
    <div>
      <div className="mb-4 flex flex-wrap items-center gap-1">
        {TREND_SERIES.map(series => (
          <Button
            variant="ghost"
            size="sm"
            key={series.key}
            type="button"
            onClick={() => toggleSeries(series.key)}
            data-tauri-drag-region="false"
            aria-pressed={!hidden.has(series.key)}
          >
            <span
              className={`h-2 w-2 rounded-full ${hidden.has(series.key) ? 'opacity-30' : ''}`}
              style={{ backgroundColor: isDark ? series.darkStroke : series.stroke }}
            />
            {lang === 'zh' ? series.labelZh : series.labelEn}
          </Button>
        ))}
        <Button
          variant="ghost"
          size="sm"
          type="button"
          onClick={() => toggleSeries('hitRate')}
          data-tauri-drag-region="false"
          aria-pressed={!rateHidden}
        >
          <span
            className={`h-0.5 w-3 rounded-full ${rateHidden ? 'opacity-30' : ''}`}
            style={{
              backgroundImage: `repeating-linear-gradient(90deg, ${isDark ? HIT_RATE_COLOR.darkStroke : HIT_RATE_COLOR.stroke} 0 3px, transparent 3px 5px)`,
            }}
          />
          {lang === 'zh' ? '缓存命中率' : 'Cache hit rate'}
        </Button>
      </div>
      <div className="relative">
        <svg
          viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
          className="block w-full overflow-visible"
          style={{ aspectRatio: `${WIDTH} / ${HEIGHT}` }}
          role="img"
          aria-label={lang === 'zh' ? 'Token 用量趋势' : 'Token usage trend'}
          onMouseMove={onMove}
          onMouseLeave={() => setHoverIndex(null)}
        >
          <defs>
            {geom.seriesPaths.map(series => (
              <linearGradient key={series.key} id={`${gradientId}-${series.key}`} x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stopColor={isDark ? series.darkStroke : series.stroke} stopOpacity={isDark ? 0.18 : 0.12} />
                <stop offset="100%" stopColor={isDark ? series.darkStroke : series.stroke} stopOpacity="0" />
              </linearGradient>
            ))}
          </defs>
          <text x={PAD_L} y="12" className="fill-[var(--text-faint)] text-[10px]">Tokens</text>
          {!rateHidden && <text x={WIDTH - PAD_R} y="12" textAnchor="end" className="fill-[var(--text-faint)] text-[10px]">{lang === 'zh' ? '命中率' : 'Hit rate'}</text>}
          {gridYs.map(y => (
            <line
              key={y}
              x1={PAD_L}
              y1={y}
              x2={WIDTH - PAD_R}
              y2={y}
              stroke="var(--theme-surface-border)"
              strokeWidth="1"
              strokeDasharray={y === HEIGHT - PAD_B ? undefined : '3 5'}
              vectorEffect="non-scaling-stroke"
            />
          ))}
          {/* 左轴 token 刻度 */}
          {[0, 0.25, 0.5, 0.75, 1].map(fraction => (
            <text
              key={`l-${fraction}`}
              x={PAD_L - 12}
              y={PAD_T + geom.plotH - fraction * geom.plotH + 3}
              textAnchor="end"
              className="fill-[var(--text-faint)] text-[10px] tabular-nums"
            >
              {formatTokens(geom.maxTokens * fraction)}
            </text>
          ))}
          {/* 右轴命中率刻度 */}
          {!rateHidden &&
            [0, 0.5, 1].map(fraction => (
              <text
                key={`r-${fraction}`}
                x={WIDTH - PAD_R + 12}
                y={PAD_T + geom.plotH - fraction * geom.plotH + 3}
                textAnchor="start"
                className="fill-[var(--text-faint)] text-[10px] tabular-nums"
              >
                {Math.round(fraction * 100)}%
              </text>
            ))}
          {points.length > 1 && geom.seriesPaths.map(series => series.path && (
            <path key={`area-${series.key}`} d={`${series.path} L ${geom.x(points.length - 1)} ${HEIGHT - PAD_B} L ${geom.x(0)} ${HEIGHT - PAD_B} Z`} fill={`url(#${gradientId}-${series.key})`} />
          ))}
          {geom.seriesPaths.map(series =>
            series.path ? (
              <path
                key={series.key}
                d={series.path}
                fill="none"
                stroke={isDark ? series.darkStroke : series.stroke}
                strokeWidth="2.25"
                vectorEffect="non-scaling-stroke"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            ) : null,
          )}
          {/* 单点时线段不可见,补画常驻圆点 */}
          {points.length === 1 &&
            geom.seriesPaths.map(series => (
              <circle
                key={`single-${series.key}`}
                cx={geom.x(0)}
                cy={geom.yTokens(points[0][series.key])}
                r="3.5"
                fill={isDark ? series.darkStroke : series.stroke}
              />
            ))}
          {!rateHidden && geom.ratePath && (
            <path
              d={geom.ratePath}
              fill="none"
              stroke={isDark ? HIT_RATE_COLOR.darkStroke : HIT_RATE_COLOR.stroke}
              strokeWidth="1.75"
              vectorEffect="non-scaling-stroke"
              strokeDasharray="4 5"
              strokeLinecap="round"
            />
          )}
          {hoverIndex != null && (
            <line
              x1={geom.x(hoverIndex)}
              y1={PAD_T}
              x2={geom.x(hoverIndex)}
              y2={PAD_T + geom.plotH}
              stroke="var(--theme-surface-border-strong)"
              strokeWidth="1"
            />
          )}
          {hoverIndex != null &&
            geom.seriesPaths.map(series => (
              <circle
                key={`dot-${series.key}`}
                cx={geom.x(hoverIndex)}
                cy={geom.yTokens(points[hoverIndex][series.key])}
                r="3"
                fill={isDark ? series.darkStroke : series.stroke}
                stroke="var(--theme-surface)"
                strokeWidth="1.5"
              />
            ))}
        </svg>
        {hoverPoint && (
          <div
            className="pointer-events-none absolute top-1 z-10 min-w-44 rounded-xl border border-[var(--border)] bg-[var(--bg-input)] px-3 py-2.5 text-[11px] shadow-lg"
            style={tooltipFlip ? { right: `${100 - tooltipLeftPct + 2}%` } : { left: `${tooltipLeftPct + 2}%` }}
          >
            <div className="mb-2 font-medium text-[var(--text)]">
              {hoverPoint.label} · {formatCount(hoverPoint.requests)} {lang === 'zh' ? '次' : 'req'}
            </div>
            {TREND_SERIES.filter(series => !hidden.has(series.key)).map(series => (
              <div key={series.key} className="flex items-center justify-between gap-3 text-[var(--text-muted)]">
                <span className="inline-flex items-center gap-1.5">
                  <span className="h-1.5 w-1.5 rounded-full" style={{ backgroundColor: isDark ? series.darkStroke : series.stroke }} />
                  {lang === 'zh' ? series.labelZh : series.labelEn}
                </span>
                <span className="tabular-nums">{formatTokens(hoverPoint[series.key])}</span>
              </div>
            ))}
            {!rateHidden && <div className="flex items-center justify-between gap-3 text-[var(--text-muted)]">
              <span>{lang === 'zh' ? '命中率' : 'Hit rate'}</span>
              <span className="tabular-nums">{hoverRate == null ? '--' : formatPercent(hoverRate)}</span>
            </div>}
            <div className="mt-0.5 flex items-center justify-between gap-3 border-t border-[var(--divider)] pt-0.5 text-[var(--text-muted)]">
              <span>{lang === 'zh' ? '成本' : 'Cost'}</span>
              <span className="tabular-nums">{formatCost(hoverPoint.costUsd)}</span>
            </div>
          </div>
        )}
      </div>
      <div
        className="relative mt-2 h-4 text-[11px] text-[var(--text-muted)]"
        style={{ marginLeft: `${(PAD_L / WIDTH) * 100}%`, marginRight: `${(PAD_R / WIDTH) * 100}%` }}
      >
        {trendAxisLabels(points, lang).map(item => {
          const shift = points.length === 1 ? '-50%' : item.index === 0 ? '0' : item.index === points.length - 1 ? '-100%' : '-50%'
          return (
            <span
              key={item.index}
              className="absolute whitespace-nowrap"
              style={{ left: `${points.length === 1 ? 50 : item.pct}%`, transform: `translateX(${shift})` }}
            >
              {item.text}
            </span>
          )
        })}
      </div>
    </div>
  )
}

function trendAxisLabel(point: UsageTrendPoint, lang: string) {
  const match = /^(\d{4})-(\d{2})-(\d{2})/.exec(point.date)
  if (!match || point.date.includes(' ')) return point.label
  const month = Number(match[2])
  const day = Number(match[3])
  return lang === 'zh' ? `${month}月${day}日` : `${month}/${day}`
}

function trendAxisLabels(points: UsageTrendPoint[], lang: string) {
  if (points.length === 0) return []
  const indexes = points.length <= 8
    ? points.map((_, index) => index)
    : Array.from(new Set([
      0,
      ...Array.from({ length: 5 }, (_, step) => Math.round((step + 1) * (points.length - 1) / 6)),
      points.length - 1,
    ]))
  const span = Math.max(points.length - 1, 1)
  return indexes.map(index => ({
    index,
    pct: (index / span) * 100,
    text: trendAxisLabel(points[index], lang),
  }))
}

function GroupTable({ rows, lang, type }: { rows: UsageGroupStats[]; lang: string; type: 'provider' | 'model' }) {
  const zh = lang === 'zh'
  const totalCost = rows.reduce((sum, row) => sum + row.costUsd, 0)
  if (rows.length === 0) return <div className="py-12 text-center text-sm text-[var(--text-muted)]">{zh ? '这个范围内暂无用量，试试更长的时间范围或刷新本地 CLI。' : 'No usage in this range. Try a longer range or refresh local CLI history.'}</div>
  return <div className="custom-scrollbar overflow-x-auto">
    <table className="w-full min-w-[580px] text-left text-sm">
      <thead className="border-b border-[var(--border)] text-xs text-[var(--text-muted)]"><tr>
        <th className="py-3 pr-3 font-normal">#</th><th className="py-3 pr-3 font-normal">{type === 'model' ? (zh ? '模型' : 'Model') : (zh ? '来源' : 'Provider')}</th>
        <th className="py-3 px-3 text-right font-normal">{zh ? '费用' : 'Cost'}</th><th className="py-3 px-3 text-right font-normal">{zh ? '费用占比' : 'Cost share'}</th><th className="py-3 px-3 text-right font-normal">Tokens</th><th className="py-3 pl-3 text-right font-normal">{zh ? '会话' : 'Sessions'}</th>
      </tr></thead>
      <tbody className="divide-y divide-[var(--border)]">{rows.map((row,index) => <tr key={row.id} className="text-[var(--text)]">
        <td className="py-3 pr-3 text-xs text-[var(--text-faint)]">{index + 1}</td>
        <td className="py-3 pr-3"><div className="font-medium">{row.label}</div>{type === 'model' && <div className="mt-1 text-xs text-[var(--text-muted)]">{row.providerName}</div>}</td>
        <td className="py-3 px-3 text-right tabular-nums">{(row.unpricedRequests ?? 0) === row.requestCount ? '—' : formatCost(row.costUsd)}{(row.unpricedRequests ?? 0) > 0 && <div className="text-xs text-[var(--text-muted)]">{row.unpricedRequests} {zh ? '条未计价' : 'unpriced'}</div>}</td>
        <td className="py-3 px-3 text-right tabular-nums text-[var(--text-muted)]">{formatPercent(row.costUsd / (totalCost || 1))}</td>
        <td className="py-3 px-3 text-right tabular-nums">{formatTokens(row.totalTokens)}</td>
        <td className="py-3 pl-3 text-right tabular-nums text-[var(--text-muted)]">{row.sessionCount ?? 0}</td>
      </tr>)}</tbody>
    </table>
  </div>
}

function CostChart({ points, lang }: { points: UsageTrendPoint[]; lang: string }) {
  const gradient = useId()
  const width = 760, height = 245, left = 58, right = 12, top = 18, bottom = 30
  const peak = Math.max(0.01, ...points.map(point => point.costUsd))
  const max = Math.ceil(peak * 100) / 100
  const x = (i: number) => left + i * (width - left - right) / Math.max(1, points.length - 1)
  const y = (value: number) => height - bottom - value / max * (height - top - bottom)
  const path = smoothPath(points.map((point, i) => ({ x: x(i), y: y(point.costUsd) })))
  const [selected, setSelected] = useState<number | null>(null)
  const point = selected === null ? undefined : points[selected]
  return <div>
    <svg viewBox={`0 0 ${width} ${height}`} className="w-full overflow-visible" role="img" aria-label={lang === 'zh' ? '费用趋势图（美元）' : 'Cost trend in USD'}>
      <defs><linearGradient id={gradient} x1="0" y1="0" x2="0" y2="1"><stop offset="0%" stopColor="var(--accent)" stopOpacity="0.22" /><stop offset="100%" stopColor="var(--accent)" stopOpacity="0.02" /></linearGradient></defs>
      {[0, 1, 2, 3].map(tick => <g key={tick}><line x1={left} x2={width-right} y1={y(max*tick/3)} y2={y(max*tick/3)} stroke="var(--border)" /><text x={left-10} y={y(max*tick/3)+4} textAnchor="end" fontSize="11" fill="var(--text-muted)">${(max*tick/3).toFixed(2)}</text></g>)}
      {points.length > 1 && <path d={`${path} L ${x(points.length-1)} ${height-bottom} L ${x(0)} ${height-bottom} Z`} fill={`url(#${gradient})`} />}
      <path d={path} fill="none" stroke="var(--accent)" strokeWidth="2" />
      {points.map((p,i) => <g key={p.date}>
        {(i === 0 || i === points.length-1 || i === Math.floor(points.length/2)) && <text x={x(i)} y={height-6} textAnchor={i===0?'start':i===points.length-1?'end':'middle'} fontSize="11" fill="var(--text-muted)">{p.label}</text>}
        <circle cx={x(i)} cy={y(p.costUsd)} r={selected === i ? 4 : 2} fill="var(--accent)" />
        <rect x={x(i)-Math.max(6,(width-left-right)/Math.max(1,points.length-1)/2)} y={top} width={Math.max(12,(width-left-right)/Math.max(1,points.length-1))} height={height-top-bottom} fill="transparent" tabIndex={0} role="button" aria-label={`${p.date}: ${formatCost(p.costUsd)}`} onFocus={() => setSelected(i)} onBlur={() => setSelected(null)} onMouseEnter={() => setSelected(i)} onMouseLeave={() => setSelected(null)} onClick={() => setSelected(i)} onKeyDown={event => { if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); setSelected(i) } }} />
      </g>)}
    </svg>
    <div className="min-h-6 text-right text-xs tabular-nums text-[var(--text-muted)]" aria-live="polite">{point ? `${point.date} · ${formatCost(point.costUsd)} · ${formatTokens(point.totalTokens)} tokens` : (lang === 'zh' ? '悬停或聚焦图表查看明细' : 'Hover or focus the chart for details')}</div>
  </div>
}

function UsageSeg<T extends string>({ options, value, onChange }: {
  options: { id: T; label: string }[]
  value: T
  onChange: (id: T) => void
}) {
  return (
    <div className="inline-flex items-center gap-0.5 rounded-full bg-[var(--bg-input-subtle)] p-0.5">
      {options.map(option => (
        <button
          key={option.id}
          type="button"
          aria-pressed={value === option.id}
          className={`rounded-full px-2.5 py-0.5 text-[12px] leading-5 ${
            value === option.id
              ? 'bg-[var(--bg)] text-[var(--text)] shadow-sm'
              : 'text-[var(--text-muted)]'
          }`}
          onClick={() => onChange(option.id)}
          data-tauri-drag-region="false"
        >
          {option.label}
        </button>
      ))}
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
    <div className="divide-y divide-[var(--border)] rounded-md border border-[var(--border)] bg-[var(--bg-input)] text-[12px]">
      {logs.map(record => (
        <div key={record.id} className="px-3 py-2 text-[var(--text)]">
          <div className="flex min-w-0 items-center gap-2">
            <span className="shrink-0 tabular-nums text-[var(--text-muted)]">{formatTime(record.createdAt, lang)}</span>
            <span className="shrink-0 font-medium">{sourceLabel(record.source, lang)}</span>
            <span className="min-w-0 truncate font-mono text-[12px]" title={record.model}>{record.model}</span>
            <span className={`kv-tag ml-auto shrink-0 ${record.status === 'success' ? 'ok' : record.status === 'cancelled' ? 'warn' : 'danger'}`}>
              {record.source === 'cli' ? (lang === 'zh' ? '已记录' : 'Recorded') : statusLabel(record.status, lang)}
            </span>
          </div>
          <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px]">
            <span className="truncate text-[var(--text-muted)]" title={record.providerName || record.providerId}>
              {record.providerName || record.providerId}
              {record.operation ? ` · ${record.operation}` : ''}
            </span>
            <span className="text-[var(--text-muted)]">{formatReasoningEffort(record.reasoningEffort)}</span>
            <span className="inline-flex items-center gap-1 text-success" title={lang === 'zh' ? '输入 Token' : 'Input tokens'}>
              <ArrowDown aria-hidden="true" size={12} strokeWidth={1.8} />
              <span className="text-[var(--text)]">{formatOptionalTokens(record.inputTokens)}</span>
            </span>
            <span className="inline-flex items-center gap-1 text-violet-600 dark:text-violet-400" title={lang === 'zh' ? '输出 Token' : 'Output tokens'}>
              <ArrowUp aria-hidden="true" size={12} strokeWidth={1.8} />
              <span className="text-[var(--text)]">
                {record.source === 'knowledge_base' ? '--' : formatOptionalTokens(record.outputTokens)}
              </span>
            </span>
            <span className="inline-flex items-center gap-1 text-sky-700 dark:text-sky-400" title={lang === 'zh' ? '缓存读取 Token' : 'Cache read tokens'}>
              <Database aria-hidden="true" size={11} strokeWidth={1.7} />
              <span>{formatOptionalTokens(record.cachedInputTokens)}</span>
              {(record.cacheCreationInputTokens ?? 0) > 0 && (
                <span className="text-warning">
                  {lang === 'zh' ? '写入' : 'write'} {formatTokens(record.cacheCreationInputTokens)}
                </span>
              )}
            </span>
            <span className="tabular-nums">{record.costUsd == null ? '--' : formatCost(record.costUsd)}</span>
            <span className="tabular-nums text-[var(--text-muted)]">
              {lang === 'zh' ? '首字' : 'First'} {formatDuration(record.firstTokenMs)}
              {' · '}
              {lang === 'zh' ? '总耗时' : 'Total'} {formatDuration(record.source === 'cli' ? null : record.durationMs)}
            </span>
            {record.usageSource === 'missing' && (
              <span className="text-warning">{lang === 'zh' ? 'Usage 缺失' : 'Usage missing'}</span>
            )}
          </div>
        </div>
      ))}
    </div>
  )
}

export function UsageStatsPanel({ lang, view }: UsageStatsPanelProps) {
  const [range, setRange] = useState<UsageRange>('1d')
  const [viewMode, setViewMode] = useState<UsageView>(view === 'calls' ? 'logs' : 'models')
  const [source, setSource] = useState('all')
  const [status, setStatus] = useState('all')
  const [providerSearch, setProviderSearch] = useState('')
  const [modelSearch, setModelSearch] = useState('')
  const [debouncedProviderSearch, setDebouncedProviderSearch] = useState('')
  const [debouncedModelSearch, setDebouncedModelSearch] = useState('')
  const [logPageIndex, setLogPageIndex] = useState(0)
  const [stats, setStats] = useState<UsageStatsResponse | null>(null)
  const [metric, setMetric] = useState<'cost' | 'tokens'>('cost')
  const [syncing, setSyncing] = useState(false)
  const [scan, setScan] = useState<Awaited<ReturnType<typeof api.usageSyncCli>> | null>(null)
  const [loading, setLoading] = useState(false)
  const [clearing, setClearing] = useState(false)
  const [error, setError] = useState('')

  const statsGeneration = useRef(0)
  const loadStats = useCallback(async () => {
    const generation = ++statsGeneration.current
    setLoading(true)
    setError('')
    try {
      const data = await api.usageGetStats({
        range,
        source,
        status,
        providerSearch: debouncedProviderSearch,
        modelSearch: debouncedModelSearch,
        limit: LOG_PAGE_SIZE,
        offset: logPageIndex * LOG_PAGE_SIZE,
      })
      if (generation === statsGeneration.current) setStats(data)
    } catch (err) {
      if (generation === statsGeneration.current) setError(err instanceof Error ? err.message : String(err))
    } finally {
      if (generation === statsGeneration.current) setLoading(false)
    }
  }, [debouncedModelSearch, debouncedProviderSearch, logPageIndex, range, source, status])

  useEffect(() => {
    const timer = window.setTimeout(() => {
      setLogPageIndex(0)
      setDebouncedProviderSearch(providerSearch.trim())
      setDebouncedModelSearch(modelSearch.trim())
    }, SEARCH_DEBOUNCE_MS)
    return () => window.clearTimeout(timer)
  }, [modelSearch, providerSearch])

  useEffect(() => {
    void loadStats()
    return () => { statsGeneration.current += 1 }
  }, [loadStats])

  const loadStatsRef = useRef(loadStats)
  useEffect(() => { loadStatsRef.current = loadStats }, [loadStats])
  const refresh = useCallback(async () => {
    setSyncing(true)
    try {
      setScan(await api.usageSyncCli(true))
      await loadStatsRef.current()
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally { setSyncing(false) }
  }, [])
  useEffect(() => {
    if (view !== 'app') { setSyncing(false); return }
    let cancelled = false
    setSyncing(true)
    void api.usageSyncCli().then(async result => {
      if (!cancelled) { setScan(result); await loadStatsRef.current() }
    }).catch(err => {
      if (!cancelled) setError(err instanceof Error ? err.message : String(err))
    }).finally(() => { if (!cancelled) setSyncing(false) })
    return () => { cancelled = true }
  }, [view])

  const clearStats = useCallback(async () => {
    const ok = await confirmDialog({
      message: lang === 'zh' ? '清空统计账本？CLI 原生历史和 JSONL 备份会保留，已清空的历史不会再次自动导入。' : 'Clear the usage ledger? Native CLI history and JSONL backups are kept. Cleared history will not be imported again.',
      confirmLabel: lang === 'zh' ? '清空' : 'Clear',
      danger: true,
    })
    if (!ok) return
    setClearing(true)
    setError('')
    try {
      await api.usageClear()
      setScan(null)
      await loadStats()
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setClearing(false)
    }
  }, [lang, loadStats])

  const summary = stats?.summary
  const totalLogs = stats?.totalLogs ?? 0
  const pageCount = Math.max(1, Math.ceil(totalLogs / LOG_PAGE_SIZE))
  const canGoPrev = logPageIndex > 0 && !loading
  const canGoNext = logPageIndex + 1 < pageCount && !loading
  const zh = lang === 'zh'
  const rangeOptions: { id: UsageRange; label: string }[] = [
    { id: 'today', label: zh ? '当天' : 'Today' },
    { id: '1d', label: zh ? '近 24 小时' : '24h' },
    { id: '7d', label: zh ? '近 7 日' : '7d' },
    { id: '30d', label: zh ? '近 30 日' : '30d' },
    { id: '90d', label: zh ? '近 90 日' : '90d' },
    { id: 'all', label: zh ? '全部' : 'All time' },
  ]
  useEffect(() => {
    if (logPageIndex > 0 && (totalLogs === 0 || logPageIndex >= pageCount)) {
      setLogPageIndex(Math.max(0, pageCount - 1))
    }
  }, [logPageIndex, pageCount, totalLogs])

  const updateRange = useCallback((next: UsageRange) => {
    setLogPageIndex(0)
    setRange(next)
  }, [])

  const updateSource = useCallback((next: string) => {
    setLogPageIndex(0)
    setSource(next)
  }, [])

  const updateStatus = useCallback((next: string) => {
    setLogPageIndex(0)
    setStatus(next)
  }, [])

  const toolbar = <>
      <div className="mb-2 flex flex-wrap items-center gap-2">
        <UsageSeg value={range} onChange={updateRange} options={rangeOptions} />
        <UsageSeg
          value={viewMode}
          onChange={setViewMode}
          options={[
            { id: 'logs', label: zh ? '请求日志' : 'Logs' },
            { id: 'providers', label: 'Provider' },
            { id: 'models', label: zh ? '模型' : 'Models' },
          ]}
        />
        <div className="ml-auto flex items-center gap-1.5">
          <Button variant="ghost" size="sm" onClick={() => void refresh()} disabled={loading || syncing} data-tauri-drag-region="false">
            <RefreshCw size={11} className={loading || syncing ? 'animate-spin' : ''} />
            {zh ? '刷新' : 'Refresh'}
          </Button>
          <Button variant="ghost" size="sm" onClick={() => void clearStats()} disabled={clearing || loading || syncing} data-tauri-drag-region="false">
            <Trash2 size={11} />
            {zh ? '清空' : 'Clear'}
          </Button>
        </div>
      </div>
      <div className="mb-3 flex flex-wrap items-center gap-2">
        <Select
          className="w-40"
          value={source}
          onChange={updateSource}
          options={SOURCE_OPTIONS.map(value => ({ value, label: sourceLabel(value, lang) }))}
        />
        <Select
          className="w-36"
          value={status}
          onChange={updateStatus}
          options={STATUS_OPTIONS.map(value => ({ value, label: statusLabel(value, lang) }))}
        />
        <div className="grid min-w-[240px] flex-1 grid-cols-1 gap-2 sm:grid-cols-2">
          <Input value={providerSearch} onChange={setProviderSearch} placeholder={zh ? '搜索 Provider' : 'Search provider'} />
          <Input value={modelSearch} onChange={setModelSearch} placeholder={zh ? '搜索模型' : 'Search model'} mono />
        </div>
      </div>

  </>

  const details = (
    <SettingsGroup>
      {viewMode === 'logs' && <LogsTable logs={stats?.logs ?? []} lang={lang} />}
      {viewMode === 'providers' && <GroupTable rows={[...(stats?.providerStats ?? [])].sort((a, b) => metric === 'cost' ? b.costUsd - a.costUsd : b.totalTokens - a.totalTokens)} lang={lang} type="provider" />}
      {viewMode === 'models' && <GroupTable rows={[...(stats?.modelStats ?? [])].sort((a, b) => metric === 'cost' ? b.costUsd - a.costUsd : b.totalTokens - a.totalTokens)} lang={lang} type="model" />}

      {stats && viewMode === 'logs' && (
        <div className="mt-2 flex flex-wrap items-center justify-between gap-2 text-[11px] text-[var(--text-muted)]">
          <span>
            {zh
              ? `显示 ${pageRangeLabel(logPageIndex, LOG_PAGE_SIZE, totalLogs)} 条`
              : `Showing ${pageRangeLabel(logPageIndex, LOG_PAGE_SIZE, totalLogs)}`}
          </span>
          <div className="flex items-center gap-1.5">
            <Button
              size="sm"
              onClick={() => setLogPageIndex(page => Math.max(0, page - 1))}
              disabled={!canGoPrev}
              data-tauri-drag-region="false"
              title={zh ? '上一页' : 'Previous page'}
            >
              <ChevronLeft size={11} />
              {zh ? '上一页' : 'Prev'}
            </Button>
            <span className="min-w-12 text-center tabular-nums">
              {logPageIndex + 1} / {pageCount}
            </span>
            <Button
              size="sm"
              onClick={() => setLogPageIndex(page => Math.min(pageCount - 1, page + 1))}
              disabled={!canGoNext}
              data-tauri-drag-region="false"
              title={zh ? '下一页' : 'Next page'}
            >
              {zh ? '下一页' : 'Next'}
              <ChevronRight size={11} />
            </Button>
          </div>
        </div>
      )}
    </SettingsGroup>
  )

  return (
    <div className="space-y-6" aria-busy={loading || syncing}>
      {error && <ErrorNotice message={error} />}
      {toolbar}
      {view === 'app' && <>
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div>
            <h2 className="text-lg font-semibold text-[var(--text)]">{zh ? '用量概览' : 'Usage overview'}</h2>
            <p className="mt-1 text-xs text-[var(--text-muted)]">{zh ? '内置模型与本机 CLI 的用量，统一查看。' : 'Built-in models and local CLI usage, together.'}</p>
          </div>
          <UsageSeg value={metric} onChange={setMetric} options={[{ id: 'cost', label: zh ? '费用' : 'Cost' }, { id: 'tokens', label: 'Tokens' }]} />
        </div>
        <div className={`grid gap-6 lg:grid-cols-[240px_minmax(0,1fr)] ${loading || syncing ? 'opacity-60' : ''}`}>
          <div>
            <div className="text-4xl font-semibold tracking-tight tabular-nums text-[var(--text)]">
              {stats ? (metric === 'cost' ? (summary?.totalRequests && summary.unpricedRequests === summary.totalRequests ? '—' : formatCost(summary?.totalCostUsd)) : formatTokens(summary?.totalTokens)) : '—'}
            </div>
            <p className="mt-2 text-xs text-[var(--text-muted)]">
              {formatCount(summary?.sessionCount)} {zh ? '个会话' : 'sessions'} · {zh ? 'LiteLLM API 估算 / CLI 上报' : 'LiteLLM API estimate / CLI reported'}
            </p>
            <p className="mt-1 text-xs text-[var(--text-faint)]">{zh ? '不代表订阅账单或实际扣费。' : 'Not a subscription bill or actual charge.'}</p>
            <div className="mt-5 space-y-4">
              {[...(stats?.providerStats ?? [])].sort((a, b) => metric === 'cost' ? b.costUsd - a.costUsd : b.totalTokens - a.totalTokens).map(row => (
                <button type="button" key={row.id} onClick={() => setProviderSearch(row.providerId ?? row.id)} className="block w-full text-left" title={zh ? `筛选 ${row.label}` : `Filter ${row.label}`}>
                  <div className="flex items-center justify-between gap-2 text-sm text-[var(--text)]">
                    <span className="truncate font-medium">{row.label}</span>
                    <span className="shrink-0 tabular-nums">{metric === 'cost' ? ((row.unpricedRequests ?? 0) === row.requestCount ? '—' : formatCost(row.costUsd)) : formatTokens(row.totalTokens)}</span>
                  </div>
                  <div className="mt-1 text-xs text-[var(--text-muted)]">{formatPercent(metric === 'cost' ? row.costUsd / (summary?.totalCostUsd || 1) : row.totalTokens / (summary?.totalTokens || 1))} · {formatTokens(row.totalTokens)} tokens · {row.sessionCount ?? 0} {zh ? '个会话' : 'sessions'}</div>
                </button>
              ))}
            </div>
          </div>
          <section className="min-w-0" aria-label={zh ? '用量趋势' : 'Usage trend'}>
            <h3 className="mb-3 text-sm font-medium text-[var(--text)]">{metric === 'cost' ? (zh ? '费用趋势' : 'Cost over time') : (zh ? 'Token 趋势' : 'Tokens over time')}</h3>
            {metric === 'cost' ? <CostChart points={stats?.trend ?? []} lang={lang} /> : <TrendChart points={stats?.trend ?? []} lang={lang} />}
          </section>
        </div>
        <section className="border-y border-[var(--border)] py-5" aria-label={zh ? 'Token 构成' : 'Token breakdown'}>
          <div className="grid grid-cols-2 gap-5 sm:grid-cols-4">
            {[
              { label: zh ? '处理 Token' : 'Processed tokens', value: summary?.totalTokens },
              { label: zh ? '缓存读取' : 'Cached input', value: summary?.cachedInputTokens },
              { label: zh ? '非缓存输入' : 'Uncached input', value: Math.max(0, (summary?.inputTokens ?? 0) - (summary?.cachedInputTokens ?? 0) - (summary?.cacheCreationInputTokens ?? 0)) },
              { label: zh ? '输出' : 'Output', value: summary?.outputTokens },
             ].map(({label, value}) => <div key={label}><div className="text-xs text-[var(--text-muted)]">{label}</div><div className="mt-1 text-xl font-medium tabular-nums text-[var(--text)]">{formatTokens(typeof value === 'number' ? value : undefined)}</div></div>)}
          </div>
          <p className="mt-4 text-xs text-[var(--text-muted)]">{zh ? '缓存写入' : 'Cache writes'} {formatTokens(summary?.cacheCreationInputTokens)} · {zh ? '缓存命中率' : 'Cache hit rate'} {formatPercent((summary?.cachedInputTokens ?? 0) / (summary?.inputTokens || 1))}</p>
        </section>
        {(summary?.unpricedRequests ?? 0) > 0 && <p role="status" className="text-xs text-[var(--text-muted)]">{zh ? `${summary?.unpricedRequests} 条记录未计价；Token 已计入，费用合计不包含这些记录。` : `${summary?.unpricedRequests} records are unpriced. Tokens are included; their cost is not.`}</p>}
        <details className="text-xs text-[var(--text-muted)]">
          <summary className="cursor-pointer">{syncing ? (zh ? '正在同步本地 CLI…' : 'Syncing local CLI…') : (zh ? '本地 CLI 数据来源' : 'Local CLI data sources')}</summary>
          <div className="mt-2 space-y-1" role="status">{scan?.sources.map(item => <div key={item.id}>{item.label} · {item.files} {zh ? '个历史文件' : 'history files'}{item.skipped > 0 ? ` · ${item.skipped} ${zh ? '行无法读取' : 'unreadable lines'}` : ''}{item.errors.map(message => <p key={message} className="break-all">{message}</p>)}</div>)}</div>
          <p className="mt-2">{zh ? '读取本机原生历史，刷新时同步。未找到记录不表示零用量。Cursor、Antigravity 等尚未接入此历史扫描。' : 'Reads local native history on refresh. Missing history does not mean zero usage. Cursor, Antigravity and other sources are not scanned yet.'}</p>
        </details>
      </>}
      <section aria-label={zh ? '用量明细' : 'Usage breakdown'}><h3 className="mb-3 text-sm font-medium text-[var(--text)]">{zh ? '明细' : 'Breakdown'}</h3>{details}</section>
    </div>
  )
}
