import { useCallback, useEffect, useMemo, useState, type MouseEvent as ReactMouseEvent } from 'react'
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

type UsageView = 'logs' | 'providers' | 'models'

type UsageStatsPanelProps = {
  lang: string
  view: 'app' | 'calls'
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

/// 环形图分类色(dataviz 参考色板,固定顺序不循环;第 7 片起折叠为「其他」灰)。
const PIE_COLORS: { light: string; dark: string }[] = [
  { light: '#2a78d6', dark: '#3987e5' },
  { light: '#1baf7a', dark: '#34d399' },
  { light: '#eda100', dark: '#fbbf24' },
  { light: '#4a3aa7', dark: '#9085e9' },
  { light: '#e34948', dark: '#e66767' },
  { light: '#0891b2', dark: '#22d3ee' },
]
const PIE_OTHER_COLOR = { light: '#a8a29e', dark: '#78716c' }
const PIE_MAX_SLICES = 6

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
  const isDark = typeof document !== 'undefined' && document.documentElement.classList.contains('dark')

  const WIDTH = 640
  const HEIGHT = 168
  const PAD_L = 8
  const PAD_R = 8
  const PAD_T = 16
  const PAD_B = 8

  const geom = useMemo(() => {
    const visible = TREND_SERIES.filter(series => !hidden.has(series.key))
    const maxTokens = Math.max(1, ...points.flatMap(point => visible.map(series => point[series.key])))
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
  const gridYs = [0, 0.5, 1].map(fraction => PAD_T + geom.plotH - fraction * geom.plotH)
  // tooltip 靠左半边时显示在指针右侧，反之左侧，避免出界。
  const tooltipLeftPct = hoverIndex != null ? (geom.x(hoverIndex) / WIDTH) * 100 : 0
  const tooltipFlip = tooltipLeftPct > 55

  return (
    <div>
      <div className="mb-2 flex flex-wrap items-center gap-2">
        {TREND_SERIES.map(series => (
          <button
            key={series.key}
            type="button"
            onClick={() => toggleSeries(series.key)}
            data-tauri-drag-region="false"
            className={`inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-[11px] transition-opacity ${
              hidden.has(series.key)
                ? 'border-[var(--border)] text-[var(--text-faint)] opacity-55'
                : 'border-[var(--border)] text-[var(--text-muted)]'
            }`}
          >
            <span
              className="h-2 w-2 rounded-full"
              style={{ backgroundColor: isDark ? series.darkStroke : series.stroke }}
            />
            {lang === 'zh' ? series.labelZh : series.labelEn}
          </button>
        ))}
        <button
          type="button"
          onClick={() => toggleSeries('hitRate')}
          data-tauri-drag-region="false"
          className={`inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-[11px] transition-opacity ${
            rateHidden
              ? 'border-[var(--border)] text-[var(--text-faint)] opacity-55'
              : 'border-[var(--border)] text-[var(--text-muted)]'
          }`}
        >
          <span
            className="h-0.5 w-3 rounded-full"
            style={{
              backgroundImage: `repeating-linear-gradient(90deg, ${isDark ? HIT_RATE_COLOR.darkStroke : HIT_RATE_COLOR.stroke} 0 3px, transparent 3px 5px)`,
            }}
          />
          {lang === 'zh' ? '缓存命中率' : 'Cache hit rate'}
        </button>
      </div>
      <div className="relative">
        <svg
          viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
          className="block w-full overflow-visible"
          style={{ aspectRatio: `${WIDTH} / ${HEIGHT}` }}
          role="img"
          aria-label="token usage trend"
          onMouseMove={onMove}
          onMouseLeave={() => setHoverIndex(null)}
        >
          {gridYs.map(y => (
            <line
              key={y}
              x1={PAD_L}
              y1={y}
              x2={WIDTH - PAD_R}
              y2={y}
              stroke="currentColor"
              className="text-neutral-200 dark:text-neutral-800"
              strokeWidth="1"
            />
          ))}
          {/* 左轴 token 刻度 */}
          {[0, 0.5, 1].map(fraction => (
            <text
              key={`l-${fraction}`}
              x={PAD_L + 4}
              y={PAD_T + geom.plotH - fraction * geom.plotH + (fraction === 1 ? 12 : -4)}
              textAnchor="start"
              className="fill-neutral-400 text-[10px] tabular-nums dark:fill-neutral-500"
            >
              {formatTokens(geom.maxTokens * fraction)}
            </text>
          ))}
          {/* 右轴命中率刻度 */}
          {!rateHidden &&
            [0, 0.5, 1].map(fraction => (
              <text
                key={`r-${fraction}`}
                x={WIDTH - PAD_R - 4}
                y={PAD_T + geom.plotH - fraction * geom.plotH + (fraction === 1 ? 12 : -4)}
                textAnchor="end"
                className="text-[10px] tabular-nums"
                style={{ fill: isDark ? HIT_RATE_COLOR.darkStroke : HIT_RATE_COLOR.stroke }}
              >
                {Math.round(fraction * 100)}%
              </text>
            ))}
          {geom.seriesPaths.map(series =>
            series.path ? (
              <path
                key={series.key}
                d={series.path}
                fill="none"
                stroke={isDark ? series.darkStroke : series.stroke}
                strokeWidth="2"
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
              strokeWidth="2"
              strokeDasharray="5 4"
              strokeLinecap="round"
            />
          )}
          {hoverIndex != null && (
            <line
              x1={geom.x(hoverIndex)}
              y1={PAD_T}
              x2={geom.x(hoverIndex)}
              y2={PAD_T + geom.plotH}
              stroke="currentColor"
              className="text-neutral-300 dark:text-neutral-700"
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
                stroke={isDark ? '#0a0a0a' : '#ffffff'}
                strokeWidth="1.5"
              />
            ))}
        </svg>
        {hoverPoint && (
          <div
            className="pointer-events-none absolute top-1 z-10 min-w-36 rounded-md border border-[var(--border)] bg-[var(--bg-input)] px-2.5 py-2 text-[11px] shadow-sm"
            style={tooltipFlip ? { right: `${100 - tooltipLeftPct + 2}%` } : { left: `${tooltipLeftPct + 2}%` }}
          >
            <div className="mb-1 font-medium text-neutral-800 dark:text-neutral-100">
              {hoverPoint.label} · {formatCount(hoverPoint.requests)} {lang === 'zh' ? '次' : 'req'}
            </div>
            {TREND_SERIES.map(series => (
              <div key={series.key} className="flex items-center justify-between gap-3 text-neutral-600 dark:text-neutral-300">
                <span className="inline-flex items-center gap-1.5">
                  <span className="h-1.5 w-1.5 rounded-full" style={{ backgroundColor: isDark ? series.darkStroke : series.stroke }} />
                  {lang === 'zh' ? series.labelZh : series.labelEn}
                </span>
                <span className="tabular-nums">{formatTokens(hoverPoint[series.key])}</span>
              </div>
            ))}
            <div className="flex items-center justify-between gap-3 text-neutral-600 dark:text-neutral-300">
              <span>{lang === 'zh' ? '命中率' : 'Hit rate'}</span>
              <span className="tabular-nums">{hoverRate == null ? '--' : formatPercent(hoverRate)}</span>
            </div>
            <div className="mt-0.5 flex items-center justify-between gap-3 border-t border-[var(--divider)] pt-0.5 text-[var(--text-muted)]">
              <span>{lang === 'zh' ? '成本' : 'Cost'}</span>
              <span className="tabular-nums">{formatCost(hoverPoint.costUsd)}</span>
            </div>
          </div>
        )}
      </div>
      <div
        className="relative mt-1 h-4 text-[10.5px] text-neutral-500 dark:text-neutral-500"
        style={{ marginLeft: `${(PAD_L / WIDTH) * 100}%`, marginRight: `${(PAD_R / WIDTH) * 100}%` }}
      >
        {trendAxisLabels(points, lang).map(item => {
          const shift = item.index === 0 ? '0' : item.index === points.length - 1 ? '-100%' : '-50%'
          return (
            <span
              key={item.index}
              className="absolute whitespace-nowrap"
              style={{ left: `${item.pct}%`, transform: `translateX(${shift})` }}
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

type PieSlice = {
  label: string
  sub?: string
  value: number
  requests: number
  cost: number
  color: string
}

/// modelStats → 环形图切片:按 token 降序取前 N,余下折叠为「其他」。
function buildPieSlices(rows: UsageGroupStats[], isDark: boolean, lang: string): PieSlice[] {
  const sorted = rows.filter(row => row.totalTokens > 0)
  if (sorted.length === 0) return []
  const head = sorted.slice(0, PIE_MAX_SLICES)
  const rest = sorted.slice(PIE_MAX_SLICES)
  const slices: PieSlice[] = head.map((row, index) => ({
    label: row.label,
    sub: row.providerName ?? undefined,
    value: row.totalTokens,
    requests: row.requestCount,
    cost: row.costUsd,
    color: isDark ? PIE_COLORS[index].dark : PIE_COLORS[index].light,
  }))
  if (rest.length > 0) {
    slices.push({
      label: lang === 'zh' ? `其他 (${rest.length})` : `Other (${rest.length})`,
      value: rest.reduce((sum, row) => sum + row.totalTokens, 0),
      requests: rest.reduce((sum, row) => sum + row.requestCount, 0),
      cost: rest.reduce((sum, row) => sum + row.costUsd, 0),
      color: isDark ? PIE_OTHER_COLOR.dark : PIE_OTHER_COLOR.light,
    })
  }
  return slices
}

function donutArcPath(cx: number, cy: number, rOuter: number, rInner: number, startAngle: number, endAngle: number): string {
  // 单片占比 100% 时画整圆环(arc 命令无法画满 360°)。
  const full = endAngle - startAngle >= Math.PI * 2 - 1e-4
  if (full) {
    return [
      `M ${cx} ${cy - rOuter}`,
      `A ${rOuter} ${rOuter} 0 1 1 ${cx} ${cy + rOuter}`,
      `A ${rOuter} ${rOuter} 0 1 1 ${cx} ${cy - rOuter}`,
      `M ${cx} ${cy - rInner}`,
      `A ${rInner} ${rInner} 0 1 0 ${cx} ${cy + rInner}`,
      `A ${rInner} ${rInner} 0 1 0 ${cx} ${cy - rInner}`,
      'Z',
    ].join(' ')
  }
  const p = (r: number, angle: number) => `${(cx + r * Math.sin(angle)).toFixed(2)} ${(cy - r * Math.cos(angle)).toFixed(2)}`
  const large = endAngle - startAngle > Math.PI ? 1 : 0
  return [
    `M ${p(rOuter, startAngle)}`,
    `A ${rOuter} ${rOuter} 0 ${large} 1 ${p(rOuter, endAngle)}`,
    `L ${p(rInner, endAngle)}`,
    `A ${rInner} ${rInner} 0 ${large} 0 ${p(rInner, startAngle)}`,
    'Z',
  ].join(' ')
}

function ModelDonut({ rows, lang }: { rows: UsageGroupStats[]; lang: string }) {
  const [hover, setHover] = useState<number | null>(null)
  const isDark = typeof document !== 'undefined' && document.documentElement.classList.contains('dark')
  const slices = useMemo(() => buildPieSlices(rows, isDark, lang), [rows, isDark, lang])
  const total = useMemo(() => slices.reduce((sum, slice) => sum + slice.value, 0), [slices])

  if (slices.length === 0) {
    return (
      <div className="flex h-36 items-center justify-center rounded-md border border-dashed border-[var(--border)] text-[12px] text-[var(--text-muted)]">
        {lang === 'zh' ? '暂无模型数据' : 'No model data'}
      </div>
    )
  }

  const CX = 100
  const CY = 100
  const R_OUT = 84
  const R_IN = 62
  const gap = slices.length > 1 ? 0.035 : 0
  let angle = -gap / 2
  const arcs = slices.map((slice, index) => {
    const sweep = (slice.value / total) * Math.PI * 2
    const start = angle + gap / 2
    const end = angle + sweep - gap / 2
    angle += sweep
    return { slice, index, path: donutArcPath(CX, CY, R_OUT, R_IN, start, Math.max(end, start + 0.01)) }
  })
  const active = hover != null ? slices[hover] : null

  return (
    <div className="@container">
      {/* SettingsGroup 已是卡片外壳,这里不再套边框/背景(避免卡中卡)。
          容器查询:≥28rem 环形图与表格同行(卡不被撑高),更窄才堆叠。 */}
      <div className="flex flex-col items-center gap-6 @md:flex-row @md:items-center">
        <div className="relative shrink-0">
        <svg viewBox="0 0 200 200" className="h-40 w-40" role="img" aria-label="model token distribution">
          <circle cx={CX} cy={CY} r={(R_OUT + R_IN) / 2} fill="none" stroke="var(--bg-input-subtle)" strokeWidth={R_OUT - R_IN} />
          {arcs.map(arc => (
            <path
              key={arc.index}
              d={arc.path}
              fill={arc.slice.color}
              opacity={hover == null || hover === arc.index ? 1 : 0.28}
              onMouseEnter={() => setHover(arc.index)}
              onMouseLeave={() => setHover(null)}
              style={{ transition: 'opacity 160ms' }}
            />
          ))}
        </svg>
        <div className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center px-8 text-center">
          <div className="max-w-[4.5rem] truncate text-[10px] text-[var(--text-muted)]">
            {active ? active.label : (lang === 'zh' ? '合计' : 'Total')}
          </div>
          <div className="text-[18px] font-semibold leading-none tabular-nums text-[var(--text)]">
            {formatTokens(active ? active.value : total)}
          </div>
          <div className="mt-1 text-[10px] text-[var(--text-faint)]">
            {active
              ? formatPercent(active.value / total)
              : `${formatCount(slices.reduce((sum, slice) => sum + slice.requests, 0))} ${lang === 'zh' ? '次' : 'req'}`}
          </div>
        </div>
      </div>
      <div className="flex min-w-0 flex-1 flex-col justify-center">
        {slices.map((slice, index) => (
          <div
            key={`${slice.label}-${index}`}
            className={`flex items-center gap-3 py-1.5 ${hover === index ? 'bg-[var(--bg-hover)]' : ''}`}
            onMouseEnter={() => setHover(index)}
            onMouseLeave={() => setHover(null)}
            title={slice.sub ? `${slice.label} · ${slice.sub}` : slice.label}
          >
            <span className="h-2 w-2 shrink-0 rounded-full" style={{ backgroundColor: slice.color }} />
            <div className="min-w-0 flex-1">
              <div className="truncate text-[12px] font-medium text-neutral-800 dark:text-neutral-100">{slice.label}</div>
              <div className="truncate text-[10.5px] text-neutral-500 dark:text-neutral-500">
                {formatTokens(slice.value)} tokens{slice.sub ? ` · ${slice.sub}` : ''}
              </div>
            </div>
            <div className="shrink-0 text-right">
              <div className="text-[12px] tabular-nums text-neutral-800 dark:text-neutral-100">{formatPercent(slice.value / total)}</div>
              <div className="text-[10.5px] tabular-nums text-neutral-500">{formatCost(slice.cost)}</div>
            </div>
          </div>
        ))}
      </div>
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
    <div className="custom-scrollbar overflow-x-auto rounded-md border border-[var(--border)] bg-[var(--bg-input)]">
      <table className="min-w-[720px] w-full text-left text-[12px]">
        <thead className="border-b border-[var(--border)] text-[10.5px] uppercase tracking-wide text-[var(--text-muted)]">
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

const ACTIVITY_LEVELS = ['#e7e5e4', '#d6e6f8', '#93c2f0', '#3b82d6', '#1d4f91']

function dayKey(date: Date) {
  const y = date.getFullYear()
  const m = String(date.getMonth() + 1).padStart(2, '0')
  const d = String(date.getDate()).padStart(2, '0')
  return `${y}-${m}-${d}`
}

function mondayOf(date: Date) {
  const next = new Date(date.getFullYear(), date.getMonth(), date.getDate())
  const offset = (next.getDay() + 6) % 7
  next.setDate(next.getDate() - offset)
  return next
}

function activityLevel(value: number, max: number) {
  if (value <= 0 || max <= 0) return 0
  const ratio = value / max
  if (ratio < 0.25) return 1
  if (ratio < 0.5) return 2
  if (ratio < 0.75) return 3
  return 4
}

function TokenActivity({ points, lang }: { points: UsageTrendPoint[]; lang: string }) {
  const [mode, setMode] = useState<'day' | 'week' | 'total'>('day')
  const [hover, setHover] = useState<{ key: string; x: number; y: number } | null>(null)
  const zh = lang === 'zh'
  const byDate = useMemo(() => {
    const map = new Map<string, { tokens: number; requests: number }>()
    for (const point of points) {
      const key = point.date.slice(0, 10)
      const current = map.get(key) ?? { tokens: 0, requests: 0 }
      current.tokens += point.totalTokens
      current.requests += point.requests
      map.set(key, current)
    }
    return map
  }, [points])

  const weeks = useMemo(() => {
    const today = new Date()
    const end = mondayOf(today)
    const start = new Date(end)
    start.setDate(start.getDate() - 52 * 7)
    const columns: Date[][] = []
    for (let cursor = new Date(start); cursor <= end; cursor.setDate(cursor.getDate() + 7)) {
      const days: Date[] = []
      for (let i = 0; i < 7; i += 1) {
        const day = new Date(cursor)
        day.setDate(cursor.getDate() + i)
        days.push(day)
      }
      columns.push(days)
    }
    return columns
  }, [])

  const weekTotals = useMemo(() => weeks.map(days => days.reduce((sum, day) => sum + (byDate.get(dayKey(day))?.tokens ?? 0), 0)), [weeks, byDate])
  const cumulative = useMemo(() => {
    const map = new Map<string, number>()
    let running = 0
    for (const days of weeks) {
      for (const day of days) {
        running += byDate.get(dayKey(day))?.tokens ?? 0
        map.set(dayKey(day), running)
      }
    }
    return map
  }, [weeks, byDate])
  const max = useMemo(() => {
    if (mode === 'week') return Math.max(0, ...weekTotals)
    if (mode === 'total') return Math.max(0, ...cumulative.values())
    return Math.max(0, ...[...byDate.values()].map(day => day.tokens))
  }, [mode, weekTotals, cumulative, byDate])

  const monthSpans = weeks.reduce<{ key: string; label: string; span: number }[]>((spans, days, index) => {
    const first = days[0]
    const prev = index > 0 ? weeks[index - 1][0] : null
    const label = !prev || first.getMonth() !== prev.getMonth()
      ? (zh ? `${first.getMonth() + 1}月` : first.toLocaleString('en-US', { month: 'short' }))
      : ''
    if (label || spans.length === 0) spans.push({ key: dayKey(first), label, span: 1 })
    else spans[spans.length - 1].span += 1
    return spans
  }, [])

  return (
    <section className="relative rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] px-4 py-3">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <div className="text-[13px] font-medium text-[var(--text)]">{zh ? 'Token 活动' : 'Token activity'}</div>
        <UsageSeg
          value={mode}
          onChange={setMode}
          options={[
            { id: 'day' as const, label: zh ? '每日' : 'Daily' },
            { id: 'week' as const, label: zh ? '每周' : 'Weekly' },
            { id: 'total' as const, label: zh ? '累计' : 'Total' },
          ]}
        />
      </div>
      <div className="custom-scrollbar overflow-x-auto">
        <div className="flex min-w-[640px] gap-[3px]">
          {weeks.map((days, weekIndex) => (
            <div key={dayKey(days[0])} className="flex flex-1 flex-col gap-[3px]">
              {days.map(day => {
                const key = dayKey(day)
                const dayStats = byDate.get(key)
                const value = mode === 'total'
                  ? (cumulative.get(key) ?? 0)
                  : mode === 'week'
                    ? weekTotals[weekIndex]
                    : (dayStats?.tokens ?? 0)
                const future = day.getTime() > Date.now()
                const selected = hover?.key === key
                return (
                  <div
                    key={key}
                    className="aspect-square w-full rounded-[3px]"
                    style={{
                      backgroundColor: future ? 'transparent' : ACTIVITY_LEVELS[activityLevel(value, max)],
                      boxShadow: selected ? '0 0 0 1px var(--bg), 0 0 0 2.5px #7eb6ea' : undefined,
                      transform: selected ? 'scale(1.45)' : undefined,
                      position: selected ? 'relative' : undefined,
                      zIndex: selected ? 1 : undefined,
                      cursor: future ? undefined : 'pointer',
                    }}
                    onMouseEnter={future ? undefined : (event) => {
                      const rect = event.currentTarget.getBoundingClientRect()
                      const card = event.currentTarget.closest('section')
                      const cardRect = card?.getBoundingClientRect() ?? rect
                      setHover({
                        key,
                        x: rect.left - cardRect.left + rect.width / 2,
                        y: rect.top - cardRect.top,
                      })
                    }}
                    onMouseLeave={future ? undefined : () => setHover(current => current?.key === key ? null : current)}
                  />
                )
              })}
            </div>
          ))}
        </div>
        <div className="mt-1.5 flex min-w-[640px] text-[10px] text-[var(--text-muted)]">
          {monthSpans.map(span => (
            <div key={span.key} className="min-w-0 whitespace-nowrap" style={{ flex: span.span }}>
              {span.label}
            </div>
          ))}
        </div>
      </div>
      {hover && (() => {
        const day = byDate.get(hover.key)
        const [year, month, date] = hover.key.split('-').map(Number)
        const label = zh ? `${year}年${month}月${date}日` : hover.key
        const tokens = day?.tokens ?? 0
        const requests = day?.requests ?? 0
        return (
          <div
            className="pointer-events-none absolute z-30 whitespace-nowrap rounded-lg border border-[var(--border)] bg-[var(--bg)] px-2.5 py-1.5 text-[12px] shadow-md"
            style={{ left: hover.x, top: hover.y - 8, transform: 'translate(-50%, -100%)' }}
          >
            <div className="font-medium text-[var(--text)]">{label}</div>
            <div className="mt-0.5 text-[var(--text-muted)]">
              {formatTokens(tokens)} tokens · {formatCount(requests)} {zh ? '次请求' : 'requests'}
            </div>
          </div>
        )
      })()}
    </section>
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
        <div key={record.id} className="px-3 py-2 text-neutral-800 dark:text-neutral-100">
          <div className="flex min-w-0 items-center gap-2">
            <span className="shrink-0 tabular-nums text-[var(--text-muted)]">{formatTime(record.createdAt, lang)}</span>
            <span className="shrink-0 font-medium">{sourceLabel(record.source, lang)}</span>
            <span className="min-w-0 truncate font-mono text-[11.5px]" title={record.model}>{record.model}</span>
            <span className={`kv-tag ml-auto shrink-0 ${record.status === 'success' ? 'ok' : record.status === 'cancelled' ? 'warn' : 'danger'}`}>
              {statusLabel(record.status, lang)}
            </span>
          </div>
          <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px]">
            <span className="truncate text-[var(--text-muted)]" title={record.providerName || record.providerId}>
              {record.providerName || record.providerId}
              {record.operation ? ` · ${record.operation}` : ''}
            </span>
            <span className="text-neutral-700 dark:text-neutral-200">{formatReasoningEffort(record.reasoningEffort)}</span>
            <span className="inline-flex items-center gap-1 text-emerald-700 dark:text-emerald-400" title={lang === 'zh' ? '输入 Token' : 'Input tokens'}>
              <ArrowDown aria-hidden="true" size={12} strokeWidth={1.8} />
              <span className="text-neutral-800 dark:text-neutral-100">{formatOptionalTokens(record.inputTokens)}</span>
            </span>
            <span className="inline-flex items-center gap-1 text-violet-600 dark:text-violet-400" title={lang === 'zh' ? '输出 Token' : 'Output tokens'}>
              <ArrowUp aria-hidden="true" size={12} strokeWidth={1.8} />
              <span className="text-neutral-800 dark:text-neutral-100">
                {record.source === 'knowledge_base' ? '--' : formatOptionalTokens(record.outputTokens)}
              </span>
            </span>
            <span className="inline-flex items-center gap-1 text-sky-700 dark:text-sky-400" title={lang === 'zh' ? '缓存读取 Token' : 'Cache read tokens'}>
              <Database aria-hidden="true" size={11} strokeWidth={1.7} />
              <span>{formatOptionalTokens(record.cachedInputTokens)}</span>
              {(record.cacheCreationInputTokens ?? 0) > 0 && (
                <span className="text-amber-700 dark:text-amber-400">
                  {lang === 'zh' ? '写入' : 'write'} {formatTokens(record.cacheCreationInputTokens)}
                </span>
              )}
            </span>
            <span className="tabular-nums">{record.costUsd == null ? '--' : formatCost(record.costUsd)}</span>
            <span className="tabular-nums text-[var(--text-muted)]">
              {lang === 'zh' ? '首字' : 'First'} {formatDuration(record.firstTokenMs)}
              {' · '}
              {lang === 'zh' ? '总耗时' : 'Total'} {formatDuration(record.durationMs)}
            </span>
            {record.usageSource === 'missing' && (
              <span className="text-amber-700 dark:text-amber-400">{lang === 'zh' ? 'Usage 缺失' : 'Usage missing'}</span>
            )}
          </div>
        </div>
      ))}
    </div>
  )
}

export function UsageStatsPanel({ lang, view }: UsageStatsPanelProps) {
  const [range, setRange] = useState<UsageRange>('7d')
  const [viewMode, setViewMode] = useState<UsageView>('logs')
  const [source, setSource] = useState('all')
  const [status, setStatus] = useState('all')
  const [providerSearch, setProviderSearch] = useState('')
  const [modelSearch, setModelSearch] = useState('')
  const [debouncedProviderSearch, setDebouncedProviderSearch] = useState('')
  const [debouncedModelSearch, setDebouncedModelSearch] = useState('')
  const [logPageIndex, setLogPageIndex] = useState(0)
  const [stats, setStats] = useState<UsageStatsResponse | null>(null)
  const [activity, setActivity] = useState<UsageTrendPoint[]>([])
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
        providerSearch: debouncedProviderSearch,
        modelSearch: debouncedModelSearch,
        limit: LOG_PAGE_SIZE,
        offset: logPageIndex * LOG_PAGE_SIZE,
      })
      setStats(data)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setLoading(false)
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
  }, [loadStats])

  useEffect(() => {
    if (view !== 'app') return
    let cancelled = false
    void api.usageGetStats({ range: '365d', limit: 1, offset: 0 }).then(data => {
      if (!cancelled) setActivity(data.trend)
    }).catch(() => {
      if (!cancelled) setActivity([])
    })
    return () => {
      cancelled = true
    }
  }, [view])

  const clearStats = useCallback(async () => {
    const ok = window.confirm(lang === 'zh' ? '清空所有本地用量统计？' : 'Clear all local usage statistics?')
    if (!ok) return
    setClearing(true)
    setError('')
    try {
      await api.usageClear()
      setActivity([])
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
    { id: '1d', label: zh ? '近 1 日' : '1d' },
    { id: '7d', label: zh ? '近 7 日' : '7d' },
    { id: '30d', label: zh ? '近 30 日' : '30d' },
  ]
  const metricItems = [
    { label: zh ? '累计 Token' : 'Total tokens', value: formatTokens(summary?.totalTokens), sub: `${formatCount(summary?.totalRequests)} ${zh ? '次' : 'req'}` },
    { label: zh ? '成本' : 'Cost', value: formatCost(summary?.totalCostUsd), sub: '' },
    {
      label: zh ? '缓存命中率' : 'Cache hit rate',
      value: summary && summary.inputTokens > 0 ? formatPercent(summary.cachedInputTokens / summary.inputTokens) : '0%',
      sub: '',
    },
    { label: zh ? '平均耗时' : 'Avg duration', value: formatDuration(summary?.averageDurationMs), sub: '' },
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

  const details = (
    <SettingsGroup>
      <div className="mb-2 flex flex-wrap items-center gap-2">
        <UsageSeg value={range} onChange={updateRange} options={rangeOptions} />
        <UsageSeg
          value={viewMode}
          onChange={setViewMode}
          options={[
            { id: 'logs' as const, label: zh ? '请求日志' : 'Logs' },
            { id: 'providers' as const, label: 'Provider' },
            { id: 'models' as const, label: zh ? '模型' : 'Models' },
          ]}
        />
        <div className="ml-auto flex items-center gap-1.5">
          <Button variant="ghost" size="sm" onClick={() => void loadStats()} disabled={loading} data-tauri-drag-region="false">
            <RefreshCw size={11} className={loading ? 'animate-spin' : ''} />
            {zh ? '刷新' : 'Refresh'}
          </Button>
          <Button variant="ghost" size="sm" onClick={() => void clearStats()} disabled={clearing || loading} data-tauri-drag-region="false">
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

      {viewMode === 'logs' && <LogsTable logs={stats?.logs ?? []} lang={lang} />}
      {viewMode === 'providers' && <GroupTable rows={stats?.providerStats ?? []} lang={lang} type="provider" />}
      {viewMode === 'models' && <GroupTable rows={stats?.modelStats ?? []} lang={lang} type="model" />}

      {stats && viewMode === 'logs' && (
        <div className="mt-2 flex flex-wrap items-center justify-between gap-2 text-[11px] text-neutral-500 dark:text-neutral-500">
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

  if (view === 'calls') {
    return (
      <div className="space-y-3">
        {error && (
          <div className="kv-panel warn">
            <div className="kv-panel-body">{error}</div>
          </div>
        )}
        {details}
      </div>
    )
  }

  return (
    <div className="space-y-3">
      {error && (
        <div className="kv-panel warn">
          <div className="kv-panel-body">{error}</div>
        </div>
      )}

      <div className="grid grid-cols-2 rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] sm:grid-cols-4">
        {metricItems.map(item => (
          <div key={item.label} className="min-w-0 border-[var(--border)] px-3 py-3 text-center even:border-l sm:border-l sm:first:border-l-0">
            <div className="text-[17px] font-semibold tabular-nums text-[var(--text)]">{item.value}</div>
            <div className="mt-1 text-[11px] text-[var(--text-muted)]">{item.label}</div>
            <div className="mt-0.5 min-h-[14px] text-[10px] text-[var(--text-faint)]">{item.sub}</div>
          </div>
        ))}
      </div>

      <TokenActivity points={activity.length > 0 ? activity : (stats?.trend ?? [])} lang={lang} />

      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="text-[13px] text-[var(--text-muted)]">{zh ? '时间范围' : 'Range'}</div>
        <div className="flex flex-wrap items-center gap-3">
          <div className="flex items-center gap-1.5">
            <Button variant="ghost" size="sm" onClick={() => void loadStats()} disabled={loading} data-tauri-drag-region="false">
              <RefreshCw size={11} className={loading ? 'animate-spin' : ''} />
              {zh ? '刷新' : 'Refresh'}
            </Button>
            <Button variant="ghost" size="sm" onClick={() => void clearStats()} disabled={clearing || loading} data-tauri-drag-region="false">
              <Trash2 size={11} />
              {zh ? '清空' : 'Clear'}
            </Button>
          </div>
          <UsageSeg
            value={range}
            onChange={updateRange}
            options={rangeOptions}
          />
        </div>
      </div>

      <section className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] px-4 py-3">
        <div className="mb-2 text-[13px] font-medium text-[var(--text)]">
          {range === 'today' ? (zh ? '今日 Token 趋势' : 'Today token trend') : (zh ? '每日 Token 趋势图' : 'Daily token trend')}
        </div>
        <TrendChart points={stats?.trend ?? []} lang={lang} />
      </section>

      <section className="rounded-xl border border-[var(--border)] bg-[var(--bg-secondary)] px-4 py-3">
        <div className="mb-2 text-[13px] font-medium text-[var(--text)]">{zh ? '模型用量' : 'Model usage'}</div>
        <ModelDonut rows={stats?.modelStats ?? []} lang={lang} />
      </section>
    </div>
  )
}
