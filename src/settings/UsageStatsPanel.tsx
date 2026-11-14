import { ErrorNotice } from '../components/AppNotice'
import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type ReactNode,
} from 'react'
import { ArrowDown, ArrowUp, ChevronLeft, ChevronRight, Database, Info, RefreshCw, SlidersHorizontal, Trash2 } from 'lucide-react'
import {
  api,
  type UsageGroupStats,
  type UsageRange,
  type UsageRecord,
  type UsageStatsResponse,
  type UsageSummary,
  type UsageTrendPoint,
} from '../api/tauri'
import { AgentIcon } from '../components/AgentIcon'
import { Button, IconButton } from '../components/Button'
import { ModelIcon, ProviderIcon } from '../components/ModelIcon'
import { SearchInput, Select } from './components'
import { confirmDialog } from '../components/dialogQueue'
import { useDocumentDark } from '../theme/useDocumentDark'

type UsageView = 'logs' | 'providers' | 'models'
type UsageMetric = 'cost' | 'tokens'

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
/** 概览左侧最多单列的来源数，其余并入「其他」。 */
const SOURCE_ROW_LIMIT = 5

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
  return Math.round(value).toLocaleString('en-US')
}

function formatTokens(value?: number | null) {
  const n = Number(value ?? 0)
  if (!Number.isFinite(n) || n <= 0) return '0'
  if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(2)}B`
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(n >= 10_000_000 ? 1 : 2)}M`
  if (n >= 10_000) return `${(n / 1_000).toFixed(1)}K`
  return Math.round(n).toLocaleString('en-US')
}

function formatOptionalTokens(value?: number | null) {
  if (value == null || !Number.isFinite(Number(value))) return '--'
  return formatTokens(value)
}

function formatCost(value?: number | null) {
  const n = Number(value ?? 0)
  if (!Number.isFinite(n) || n <= 0) return '$0.00'
  if (n < 0.01) return `$${n.toFixed(4)}`
  return `$${n.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`
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
  if (!value) return ''
  const trimmed = value.trim()
  return trimmed.charAt(0).toUpperCase() + trimmed.slice(1)
}

function formatPercent(value?: number | null) {
  const n = Number(value ?? 0)
  if (!Number.isFinite(n) || n <= 0) return '0%'
  return `${Math.round(n * 100)}%`
}

/** 占比：小份额保留一位小数，极小份额不显示成误导性的 0%。 */
function formatShare(value: number) {
  if (!Number.isFinite(value) || value <= 0) return '0%'
  const pct = value * 100
  if (pct < 0.1) return '<0.1%'
  if (pct < 10) return `${pct.toFixed(1)}%`
  return `${Math.round(pct)}%`
}

/** 坐标轴刻度：三位有效数字并去掉多余的 0，不与正文数值的固定小数位混用。 */
function formatAxis(value: number, metric: UsageMetric) {
  if (value <= 0) return metric === 'cost' ? '$0' : '0'
  if (metric === 'cost') return `$${Number(value.toPrecision(3)).toLocaleString('en-US', { maximumFractionDigits: 4 })}`
  const [divisor, unit] = value >= 1e9 ? [1e9, 'B'] : value >= 1e6 ? [1e6, 'M'] : value >= 1e3 ? [1e3, 'K'] : [1, '']
  return `${Number((value / divisor).toPrecision(3))}${unit}`
}

/** 把峰值向上取整到 1 / 2 / 2.5 / 5 × 10ⁿ 的四等分刻度。 */
function niceMax(peak: number) {
  const raw = peak / 4
  const magnitude = 10 ** Math.floor(Math.log10(raw))
  const normalized = raw / magnitude
  const step = (normalized <= 1 ? 1 : normalized <= 2 ? 2 : normalized <= 2.5 ? 2.5 : normalized <= 5 ? 5 : 10) * magnitude
  return step * 4
}

type TokenTotals = Pick<UsageTrendPoint, 'inputTokens' | 'outputTokens' | 'cachedInputTokens' | 'cacheCreationInputTokens'>

/** Token 的四个互斥组成部分；趋势图与构成条共用同一口径和配色，四项相加即处理总量。 */
const TOKEN_PARTS: {
  key: string
  labelZh: string
  labelEn: string
  light: string
  dark: string
  value: (totals: TokenTotals) => number
}[] = [
  { key: 'cached', labelZh: '缓存读取', labelEn: 'Cache read', light: '#0891b2', dark: '#22d3ee', value: t => t.cachedInputTokens },
  {
    key: 'uncached',
    labelZh: '非缓存输入',
    labelEn: 'Uncached input',
    light: '#2a78d6',
    dark: '#5b9bf0',
    value: t => Math.max(0, t.inputTokens - t.cachedInputTokens - t.cacheCreationInputTokens),
  },
  { key: 'cacheWrite', labelZh: '缓存写入', labelEn: 'Cache write', light: '#d99000', dark: '#fbbf24', value: t => t.cacheCreationInputTokens },
  { key: 'output', labelZh: '输出', labelEn: 'Output', light: '#1baf7a', dark: '#34d399', value: t => t.outputTokens },
]

const HIT_RATE_COLOR = { light: '#7c3aed', dark: '#a78bfa' }

/** 来源配色：左栏份额条、来源列表与趋势图堆叠共用，明暗主题下都能分辨。 */
const SOURCE_COLORS = ['#3b7cf5', '#12b5a0', '#8b5cf6', '#f0a02c', '#ec5f85']
function shareColor(index: number) {
  return SOURCE_COLORS[Math.min(index, SOURCE_COLORS.length - 1)]
}
const OTHER_COLOR = 'var(--text-faint)'

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

function trendAxisLabel(point: UsageTrendPoint, lang: string) {
  const match = /^(\d{4})-(\d{2})-(\d{2})/.exec(point.date)
  if (!match || point.date.includes(' ')) return point.label
  const month = Number(match[2])
  const day = Number(match[3])
  return lang === 'zh' ? `${month}月${day}日` : `${month}/${day}`
}

/** 悬停卡片标题：按小时聚合时补上日期，跨天的 24 小时范围才不会分不清是哪天。 */
function trendPointTitle(point: UsageTrendPoint, lang: string) {
  const hourly = /^\d{4}-(\d{2})-(\d{2}) (\d{2})/.exec(point.date)
  if (!hourly) return trendAxisLabel(point, lang)
  const month = Number(hourly[1])
  const day = Number(hourly[2])
  return lang === 'zh' ? `${month}月${day}日 ${hourly[3]}:00` : `${month}/${day} ${hourly[3]}:00`
}

function trendAxisIndexes(count: number, maxLabels: number) {
  if (count <= maxLabels) return Array.from({ length: count }, (_, index) => index)
  const gaps = maxLabels - 1
  return Array.from(new Set(Array.from({ length: maxLabels }, (_, step) => Math.round((step * (count - 1)) / gaps))))
}

/** 图表按容器实际像素宽度绘制，文字不随 viewBox 缩放而变形。 */
function useElementWidth(fallback: number) {
  const ref = useRef<HTMLDivElement | null>(null)
  const [width, setWidth] = useState(fallback)
  useLayoutEffect(() => {
    const element = ref.current
    if (!element) return
    const initial = Math.round(element.getBoundingClientRect().width)
    if (initial > 0) setWidth(initial)
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(entries => {
      const next = Math.round(entries[0]?.contentRect.width ?? 0)
      if (next > 0) setWidth(next)
    })
    observer.observe(element)
    return () => observer.disconnect()
  }, [])
  return [ref, width] as const
}

const CHART_HEIGHT = 264

type ChartSource = { id: string; label: string; color: string }
type ChartSeries = { key: string; label: string; color: string; value: (point: UsageTrendPoint) => number }

/** 趋势图：各来源（或 Token 各部分）堆叠成面积，叠加均值线与峰值标记，下方附请求量柱条。 */
function TrendChart({ points, lang, metric, sources, otherLabel, highlightId }: {
  points: UsageTrendPoint[]
  lang: string
  metric: UsageMetric
  /** 概览左栏列出的来源，顺序与配色同左栏；费用口径按它们堆叠。 */
  sources: ChartSource[]
  /** 左栏有「其他」汇总行时的名称，对应堆叠里的剩余部分。 */
  otherLabel?: string
  /** 左栏正在悬停的来源，图中其余来源淡出。 */
  highlightId?: string | null
}) {
  const zh = lang === 'zh'
  const isDark = useDocumentDark()
  const gradientId = useId()
  const [wrapRef, width] = useElementWidth(560)
  const [hidden, setHidden] = useState<Set<string>>(() => new Set())
  const [hoverIndex, setHoverIndex] = useState<number | null>(null)
  const hover = hoverIndex != null && hoverIndex < points.length ? hoverIndex : null
  const formatValue = metric === 'cost' ? formatCost : formatTokens

  const series = useMemo<ChartSeries[]>(() => {
    if (metric === 'tokens') {
      return TOKEN_PARTS.filter(part => !hidden.has(part.key)).map(part => ({
        key: part.key,
        label: zh ? part.labelZh : part.labelEn,
        color: isDark ? part.dark : part.light,
        value: part.value,
      }))
    }
    const total: ChartSeries = { key: 'cost', label: zh ? '费用' : 'Cost', color: 'var(--accent)', value: point => point.costUsd }
    if (sources.length === 0 || !points.some(point => point.sources?.length)) return [total]
    const sourceCost = (point: UsageTrendPoint, id: string) => point.sources?.find(item => item.providerId === id)?.costUsd ?? 0
    const stacked: ChartSeries[] = sources.map(source => ({
      key: source.id,
      label: source.label,
      color: source.color,
      value: point => sourceCost(point, source.id),
    }))
    if (otherLabel) {
      stacked.push({
        key: 'other',
        label: otherLabel,
        color: OTHER_COLOR,
        value: point => Math.max(0, point.costUsd - sources.reduce((sum, source) => sum + sourceCost(point, source.id), 0)),
      })
    }
    // 这个范围内没有费用的来源不占层，否则它的轮廓会和下面一层重叠。
    const visible = stacked.filter(item => points.some(point => item.value(point) > 0))
    return visible.length > 0 ? visible : [total]
  }, [hidden, isDark, metric, otherLabel, points, sources, zh])
  const showRate = metric === 'tokens' && !hidden.has('hitRate')
  const rateColor = isDark ? HIT_RATE_COLOR.dark : HIT_RATE_COLOR.light

  const padLeft = 46
  const padRight = showRate ? 36 : 8
  const padTop = 20
  const stripHeight = 28
  const plotWidth = Math.max(40, width - padLeft - padRight)
  const plotHeight = CHART_HEIGHT - padTop - 14 - stripHeight - 22
  const baseline = padTop + plotHeight
  const stripTop = baseline + 14
  const stripBottom = stripTop + stripHeight
  const step = points.length > 1 ? plotWidth / (points.length - 1) : 0
  const barWidth = points.length > 1 ? Math.max(1.5, Math.min(12, step * 0.62)) : 12

  const geom = useMemo(() => {
    // 逐层累加：tops[k][i] 是第 k 层在第 i 个时间点的上边界。
    const tops: number[][] = []
    let running = points.map(() => 0)
    for (const item of series) {
      running = running.map((value, index) => value + Math.max(0, item.value(points[index])))
      tops.push(running)
    }
    const totals = running
    const peak = Math.max(0, ...totals)
    const max = niceMax(peak > 0 ? peak : metric === 'cost' ? 0.01 : 4)
    const x = (index: number) => (points.length > 1 ? padLeft + step * index : padLeft + plotWidth / 2)
    const y = (value: number) => baseline - (value / max) * plotHeight
    const coords = (values: number[]) => values.map((value, index) => ({ x: x(index), y: y(value) }))
    const right = x(points.length - 1)
    const left = x(0)
    const layers = series.map((item, layer) => {
      const line = smoothPath(coords(tops[layer]))
      // 下边界：底层是基线，其余层沿下一层的上边界反向走回去，闭合成带状面积。
      const floor = layer === 0
        ? ` L ${right.toFixed(1)} ${baseline} L ${left.toFixed(1)} ${baseline}`
        : ` ${smoothPath(coords(tops[layer - 1]).reverse()).replace(/^M/, 'L')}`
      const sum = points.reduce((acc, point) => acc + Math.max(0, item.value(point)), 0)
      return { ...item, line, area: `${line}${floor} Z`, top: tops[layer], sum }
    })
    const grand = layers.reduce((acc, layer) => acc + layer.sum, 0)
    // 份额不足 1% 的薄层只填色不描边：它的线会贴着下一层，把主要来源的轮廓染成杂色。
    const outlined = layers.filter((layer, index) => index === 0 || layer.sum >= grand * 0.01)
    const ratePath = smoothPath(points.flatMap((point, index) => {
      const rate = trendHitRate(point)
      return rate == null ? [] : [{ x: x(index), y: baseline - rate * plotHeight }]
    }))
    const peakIndex = totals.indexOf(peak)
    const average = totals.length > 0 ? totals.reduce((sum, value) => sum + value, 0) / totals.length : 0
    const maxRequests = Math.max(1, ...points.map(point => point.requests))
    return { max, x, y, layers, outlined, ratePath, totals, peak, peakIndex, average, maxRequests }
  }, [baseline, metric, plotHeight, plotWidth, points, series, step])

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
      const index = step > 0 ? Math.round((event.clientX - rect.left - padLeft) / step) : 0
      setHoverIndex(Math.max(0, Math.min(points.length - 1, index)))
    },
    [points.length, step],
  )

  const onKeyDown = useCallback(
    (event: ReactKeyboardEvent<HTMLDivElement>) => {
      if (points.length === 0) return
      if (event.key === 'Escape') { setHoverIndex(null); return }
      if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return
      event.preventDefault()
      const delta = event.key === 'ArrowLeft' ? -1 : 1
      setHoverIndex(previous => Math.max(0, Math.min(points.length - 1, (previous ?? (delta > 0 ? -1 : points.length)) + delta)))
    },
    [points.length],
  )

  const legendToggle = (key: string, label: string, swatch: ReactNode) => (
    <button
      key={key}
      type="button"
      onClick={() => toggleSeries(key)}
      aria-pressed={!hidden.has(key)}
      className={`inline-flex items-center gap-1.5 rounded px-1.5 py-0.5 text-[11px] transition-opacity hover:bg-[var(--bg-hover)] ${
        hidden.has(key) ? 'text-[var(--text-faint)] opacity-60' : 'text-[var(--text-muted)]'
      }`}
      data-tauri-drag-region="false"
    >
      {swatch}
      {label}
    </button>
  )

  const stacked = geom.layers.length > 1
  const dimmed = (key: string) => stacked && highlightId != null && geom.layers.some(layer => layer.key === highlightId) && key !== highlightId
  const hoverPoint = hover != null ? points[hover] : null
  const hoverX = hover != null ? geom.x(hover) : 0
  const tooltipFlip = hoverX > width * 0.55
  const maxLabels = Math.max(2, Math.min(7, Math.floor(plotWidth / 84)))
  const showMarks = points.length > 1 && geom.peak > 0
  const peakX = geom.x(Math.max(0, geom.peakIndex))
  const peakAnchor = peakX < padLeft + 36 ? 'start' : peakX > padLeft + plotWidth - 36 ? 'end' : 'middle'
  const averageY = geom.y(geom.average)
  const axisText = 'fill-[var(--text-faint)] text-[10px] tabular-nums'

  return (
    <div ref={wrapRef} className="min-w-0">
      {metric === 'tokens' && points.length > 0 && (
        <div className="-ml-1.5 mb-1 flex flex-wrap items-center gap-x-1 gap-y-0.5">
          {TOKEN_PARTS.map(part => legendToggle(
            part.key,
            zh ? part.labelZh : part.labelEn,
            <span className={SWATCH} style={{ backgroundColor: isDark ? part.dark : part.light }} />,
          ))}
          {legendToggle(
            'hitRate',
            zh ? '缓存命中率' : 'Cache hit rate',
            <span
              className="h-0.5 w-3 rounded-full"
              style={{ backgroundImage: `repeating-linear-gradient(90deg, ${rateColor} 0 3px, transparent 3px 5px)` }}
            />,
          )}
        </div>
      )}
      {points.length === 0 ? (
        <div
          className="flex items-center justify-center rounded-lg border border-dashed border-[var(--border)] text-xs text-[var(--text-faint)]"
          style={{ height: CHART_HEIGHT }}
        >
          {zh ? '这个范围内暂无趋势数据' : 'No trend data in this range'}
        </div>
      ) : (
        <div
          className="relative rounded-md outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent-ring)]"
          tabIndex={0}
          onKeyDown={onKeyDown}
          onBlur={() => setHoverIndex(null)}
          title={zh ? '悬停或用左右方向键查看各时间点' : 'Hover or use arrow keys to inspect each point'}
          data-tauri-drag-region="false"
        >
          <svg
            width={width}
            height={CHART_HEIGHT}
            viewBox={`0 0 ${width} ${CHART_HEIGHT}`}
            className="block overflow-visible"
            role="img"
            aria-label={metric === 'cost' ? (zh ? '费用趋势图（美元）' : 'Cost trend in USD') : (zh ? 'Token 用量趋势' : 'Token usage trend')}
            onMouseMove={onMove}
            onMouseLeave={() => setHoverIndex(null)}
          >
            <defs>
              <linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stopColor="var(--accent)" stopOpacity="0.24" />
                <stop offset="100%" stopColor="var(--accent)" stopOpacity="0.02" />
              </linearGradient>
            </defs>
            {[0, 1, 2, 3, 4].map(tick => {
              const y = baseline - (tick / 4) * plotHeight
              return (
                <g key={tick}>
                  <line
                    x1={padLeft}
                    x2={padLeft + plotWidth}
                    y1={y}
                    y2={y}
                    stroke={tick === 0 ? 'var(--theme-surface-border-strong)' : 'var(--theme-surface-border)'}
                    strokeDasharray={tick === 0 ? undefined : '2 4'}
                  />
                  <text x={padLeft - 10} y={y + 3.5} textAnchor="end" className={axisText}>
                    {formatAxis((geom.max * tick) / 4, metric)}
                  </text>
                  {showRate && tick % 2 === 0 && (
                    <text x={padLeft + plotWidth + 8} y={y + 3.5} className={axisText}>{tick * 25}%</text>
                  )}
                </g>
              )
            })}
            {points.length > 1 && geom.layers.map(layer => (
              <path
                key={`area-${layer.key}`}
                d={layer.area}
                fill={stacked ? layer.color : `url(#${gradientId})`}
                fillOpacity={stacked ? (dimmed(layer.key) ? 0.06 : highlightId === layer.key ? 0.42 : 0.26) : 1}
                className="transition-[fill-opacity] duration-150"
              />
            ))}
            {/* 自上而下描边：贴在一起的薄层不会盖住下方主要来源的轮廓。 */}
            {[...geom.outlined].reverse().map(layer => (
              <path
                key={layer.key}
                d={layer.line}
                fill="none"
                stroke={layer.color}
                strokeWidth={stacked ? 1.5 : 2}
                strokeOpacity={dimmed(layer.key) ? 0.25 : 1}
                strokeLinecap="round"
                strokeLinejoin="round"
                className="transition-[stroke-opacity] duration-150"
              />
            ))}
            {showRate && geom.ratePath && (
              <path d={geom.ratePath} fill="none" stroke={rateColor} strokeWidth="1.5" strokeDasharray="3 4" strokeLinecap="round" />
            )}
            {/* 单点(如单日区间)线段不可见,补画常驻圆点 */}
            {points.length === 1 && geom.layers.map(layer => (
              <circle key={`single-${layer.key}`} cx={geom.x(0)} cy={geom.y(layer.top[0])} r="3.5" fill={layer.color} />
            ))}
            {showMarks && (
              <g className={hoverPoint ? 'opacity-30' : ''}>
                <line
                  x1={padLeft}
                  x2={padLeft + plotWidth}
                  y1={averageY}
                  y2={averageY}
                  stroke="var(--text-muted)"
                  strokeOpacity="0.55"
                  strokeDasharray="5 4"
                />
                <text x={padLeft + plotWidth} y={averageY - 5} textAnchor="end" className="fill-[var(--text-muted)] text-[10px] tabular-nums">
                  {zh ? '均值' : 'Avg'} {formatValue(geom.average)}
                </text>
                <circle cx={peakX} cy={geom.y(geom.peak)} r="3" fill="var(--bg)" stroke="var(--text)" strokeWidth="1.5" />
                <text x={peakX} y={geom.y(geom.peak) - 8} textAnchor={peakAnchor} className="fill-[var(--text)] text-[10px] font-medium tabular-nums">
                  {formatValue(geom.peak)}
                </text>
              </g>
            )}
            {/* 请求量柱条：与上方共用时间轴，看得出费用高是因为请求多还是单次贵。 */}
            <text x={padLeft - 10} y={stripBottom - 1} textAnchor="end" className={axisText}>{zh ? '请求' : 'Req'}</text>
            <line x1={padLeft} x2={padLeft + plotWidth} y1={stripBottom} y2={stripBottom} stroke="var(--theme-surface-border)" />
            {points.map((point, index) => point.requests > 0 && (
              <rect
                key={point.date}
                x={geom.x(index) - barWidth / 2}
                y={stripBottom - Math.max(1.5, (point.requests / geom.maxRequests) * stripHeight)}
                width={barWidth}
                height={Math.max(1.5, (point.requests / geom.maxRequests) * stripHeight)}
                rx={Math.min(1.5, barWidth / 2)}
                fill="var(--text)"
                fillOpacity={hover === index ? 0.55 : 0.16}
              />
            ))}
            {trendAxisIndexes(points.length, maxLabels).map(index => (
              <text
                key={index}
                x={geom.x(index)}
                y={CHART_HEIGHT - 6}
                textAnchor={points.length === 1 ? 'middle' : index === 0 ? 'start' : index === points.length - 1 ? 'end' : 'middle'}
                className={axisText}
              >
                {trendAxisLabel(points[index], lang)}
              </text>
            ))}
            {hoverPoint && hover != null && (
              <>
                <line x1={hoverX} x2={hoverX} y1={padTop} y2={stripBottom} stroke="var(--theme-surface-border-strong)" />
                {geom.outlined.map((layer, index) => (stacked ? layer.value(hoverPoint) > 0 : index === 0) && (
                  <circle
                    key={`dot-${layer.key}`}
                    cx={hoverX}
                    cy={geom.y(layer.top[hover])}
                    r="3.5"
                    fill={layer.color}
                    stroke="var(--bg)"
                    strokeWidth="2"
                  />
                ))}
              </>
            )}
          </svg>
          {hoverPoint && hover != null && (
            <div
              className="pointer-events-none absolute top-1 z-10 min-w-44 rounded-lg border border-[var(--border)] bg-[var(--bg-input)] px-3 py-2 text-[11px] shadow-[var(--shadow-md)]"
              style={tooltipFlip ? { right: width - hoverX + 12 } : { left: hoverX + 12 }}
              role="status"
            >
              <div className="mb-1.5 flex items-baseline justify-between gap-4">
                <span className="text-[var(--text-muted)]">{trendPointTitle(hoverPoint, lang)}</span>
                <span className="text-[13px] font-semibold tabular-nums text-[var(--text)]">{formatValue(geom.totals[hover])}</span>
              </div>
              {stacked && geom.layers.map(layer => layer.value(hoverPoint) > 0 && (
                <TooltipRow
                  key={layer.key}
                  label={layer.label}
                  value={formatValue(layer.value(hoverPoint))}
                  color={layer.color}
                  detail={formatShare(geom.totals[hover] > 0 ? layer.value(hoverPoint) / geom.totals[hover] : 0)}
                />
              ))}
              {showRate && (
                <TooltipRow
                  label={zh ? '缓存命中率' : 'Cache hit rate'}
                  value={trendHitRate(hoverPoint) == null ? '--' : formatPercent(trendHitRate(hoverPoint))}
                  color={rateColor}
                />
              )}
              <div className="mt-1.5 border-t border-[var(--divider)] pt-1.5">
                {metric === 'cost'
                  ? <TooltipRow label="Tokens" value={formatTokens(hoverPoint.totalTokens)} />
                  : <TooltipRow label={zh ? '费用' : 'Cost'} value={formatCost(hoverPoint.costUsd)} />}
                <TooltipRow label={zh ? '请求' : 'Requests'} value={formatCount(hoverPoint.requests)} />
                {metric === 'cost' && hoverPoint.requests > 0 && (
                  <TooltipRow label={zh ? '单次请求' : 'Per request'} value={formatCost(hoverPoint.costUsd / hoverPoint.requests)} />
                )}
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  )
}

/** 趋势图下方的一行读数：峰值、均值、单次请求与综合单价。 */
function TrendStats({ points, metric, lang, hourly, summary }: {
  points: UsageTrendPoint[]
  metric: UsageMetric
  lang: string
  hourly: boolean
  summary: UsageSummary | undefined
}) {
  const zh = lang === 'zh'
  if (points.length === 0) return null
  const formatValue = metric === 'cost' ? formatCost : formatTokens
  const values = points.map(point => (metric === 'cost' ? point.costUsd : point.totalTokens))
  const total = values.reduce((sum, value) => sum + value, 0)
  const peak = Math.max(0, ...values)
  const active = points.filter(point => point.requests > 0).length
  const requests = summary?.totalRequests ?? 0
  const cost = summary?.totalCostUsd ?? 0
  const tokens = summary?.totalTokens ?? 0
  const unit = hourly ? (zh ? '小时' : 'hours') : (zh ? '天' : 'days')
  const items = [
    {
      label: zh ? '峰值' : 'Peak',
      value: peak > 0 ? formatValue(peak) : '—',
      detail: peak > 0 ? trendPointTitle(points[values.indexOf(peak)], lang) : (zh ? '暂无用量' : 'No usage'),
    },
    {
      label: hourly ? (zh ? '平均每小时' : 'Hourly average') : (zh ? '平均每天' : 'Daily average'),
      value: formatValue(total / points.length),
      detail: zh ? `${active}/${points.length} ${unit}有用量` : `${active}/${points.length} ${unit} active`,
    },
    {
      label: zh ? '单次请求' : 'Per request',
      value: requests > 0 ? formatValue(total / requests) : '—',
      detail: zh ? `${formatCount(requests)} 次请求` : `${formatCount(requests)} requests`,
    },
    {
      label: zh ? '每百万 Token' : 'Per 1M tokens',
      value: tokens > 0 && cost > 0 ? formatCost((cost / tokens) * 1_000_000) : '—',
      detail: zh ? '含缓存的综合单价' : 'Blended, cache included',
    },
  ]
  return (
    <dl className="mt-3 grid grid-cols-2 gap-x-4 gap-y-3 border-t border-[var(--divider)] pt-3 sm:grid-cols-4">
      {items.map(item => (
        <div key={item.label} className="min-w-0">
          <dt className="truncate text-[11px] text-[var(--text-muted)]">{item.label}</dt>
          <dd className="mt-0.5 text-sm font-medium tabular-nums text-[var(--text)]">{item.value}</dd>
          <dd className="truncate text-[10.5px] leading-4 tabular-nums text-[var(--text-faint)]" title={item.detail}>{item.detail}</dd>
        </div>
      ))}
    </dl>
  )
}

function TooltipRow({ label, value, color, detail }: { label: string; value: string; color?: string; detail?: string }) {
  return (
    <div className="flex items-center justify-between gap-4 leading-[18px] text-[var(--text-muted)]">
      <span className="inline-flex min-w-0 items-center gap-1.5">
        {color && <span className="h-1.5 w-1.5 shrink-0 rounded-full" style={{ backgroundColor: color }} />}
        <span className="truncate">{label}</span>
      </span>
      <span className="shrink-0 tabular-nums text-[var(--text)]">
        {value}
        {detail && <span className="ml-1.5 inline-block w-9 text-right text-[var(--text-faint)]">{detail}</span>}
      </span>
    </div>
  )
}

function Seg<T extends string>({ options, value, onChange, ariaLabel }: {
  options: { id: T; label: string }[]
  value: T
  onChange: (id: T) => void
  ariaLabel: string
}) {
  return (
    <div className="kv-seg" role="group" aria-label={ariaLabel} data-tauri-drag-region="false">
      {options.map(option => (
        <button
          key={option.id}
          type="button"
          aria-pressed={value === option.id}
          className={value === option.id ? 'active' : ''}
          onClick={() => onChange(option.id)}
          data-tauri-drag-region="false"
        >
          {option.label}
        </button>
      ))}
    </div>
  )
}

function SourceIcon({ row, size = 16 }: { row: UsageGroupStats; size?: number }) {
  const id = row.providerId ?? row.id
  if (id.startsWith('cli:')) return <AgentIcon id={id.slice(4)} size={size} className="shrink-0" />
  return <ProviderIcon name={row.providerName || row.label} size={size} />
}

function isUnpriced(row: UsageGroupStats) {
  return row.requestCount > 0 && (row.unpricedRequests ?? 0) === row.requestCount
}

function rowMetric(row: UsageGroupStats, metric: UsageMetric) {
  return metric === 'cost' ? row.costUsd : row.totalTokens
}

function sortByMetric(rows: UsageGroupStats[] | undefined, metric: UsageMetric) {
  return [...(rows ?? [])].sort((a, b) => rowMetric(b, metric) - rowMetric(a, metric))
}

const TRACK = 'bg-[color-mix(in_srgb,var(--text)_7%,transparent)]'
const SWATCH = 'h-2 w-2 shrink-0 rounded-[2px]'

/** 概览左栏：一条份额条 + 按份额排序的来源列表，点击某一来源即按它筛选整页。 */
function SourceShare({ rows, metric, lang, activeId, onToggle, onHover }: {
  rows: UsageGroupStats[]
  metric: UsageMetric
  lang: string
  activeId: string
  onToggle: (id: string) => void
  /** 悬停或聚焦某一来源时通知外层，用于在趋势图里突出它。 */
  onHover: (id: string | null) => void
}) {
  const zh = lang === 'zh'
  if (rows.length === 0) return null
  const total = rows.reduce((sum, row) => sum + rowMetric(row, metric), 0)
  const top = rows.slice(0, SOURCE_ROW_LIMIT)
  const rest = rows.slice(SOURCE_ROW_LIMIT)
  const restValue = rest.reduce((sum, row) => sum + rowMetric(row, metric), 0)
  const restTokens = rest.reduce((sum, row) => sum + row.totalTokens, 0)
  const share = (value: number) => (total > 0 ? value / total : 0)
  return (
    <div className="mt-5">
      <div className={`flex h-1.5 gap-0.5 overflow-hidden rounded-full ${TRACK}`} aria-hidden="true">
        {top.map((row, index) => rowMetric(row, metric) > 0 && (
          <i key={row.id} className="min-w-[3px]" style={{ flexGrow: rowMetric(row, metric), flexBasis: 0, backgroundColor: shareColor(index) }} />
        ))}
        {restValue > 0 && <i className="min-w-[3px]" style={{ flexGrow: restValue, flexBasis: 0, backgroundColor: OTHER_COLOR }} />}
      </div>
      <div className="-mx-2 mt-3 space-y-0.5">
        {top.map((row, index) => {
          const id = row.providerId ?? row.id
          const active = activeId === id
          return (
            <button
              type="button"
              key={row.id}
              onClick={() => onToggle(id)}
              onMouseEnter={() => onHover(id)}
              onMouseLeave={() => onHover(null)}
              onFocus={() => onHover(id)}
              onBlur={() => onHover(null)}
              aria-pressed={active}
              title={active ? (zh ? `取消筛选 ${row.label}` : `Clear ${row.label} filter`) : (zh ? `筛选 ${row.label}` : `Filter ${row.label}`)}
              className={`block w-full rounded-md px-2 py-1.5 text-left transition-colors hover:bg-[var(--bg-hover)] ${active ? 'bg-[var(--bg-hover)]' : ''}`}
              data-tauri-drag-region="false"
            >
              <div className="flex items-center gap-2 text-[13px] text-[var(--text)]">
                <span className={SWATCH} style={{ backgroundColor: shareColor(index) }} />
                <SourceIcon row={row} size={15} />
                <span className="min-w-0 flex-1 truncate font-medium">{row.label}</span>
                <span className="shrink-0 tabular-nums">
                  {metric === 'cost' ? (isUnpriced(row) ? '—' : formatCost(row.costUsd)) : formatTokens(row.totalTokens)}
                </span>
              </div>
              <div className="mt-0.5 pl-4 text-[11px] tabular-nums text-[var(--text-muted)]">
                {formatShare(share(rowMetric(row, metric)))}
                {metric === 'cost' ? ` · ${formatTokens(row.totalTokens)} tokens` : isUnpriced(row) ? '' : ` · ${formatCost(row.costUsd)}`}
                {` · ${formatCount(row.sessionCount)} ${zh ? '个会话' : 'sessions'}`}
              </div>
            </button>
          )
        })}
        {rest.length > 0 && (
          <div className="px-2 py-1.5">
            <div className="flex items-center gap-2 text-[13px] text-[var(--text-muted)]">
              <span className={SWATCH} style={{ backgroundColor: OTHER_COLOR }} />
              <span className="min-w-0 flex-1 truncate">{zh ? `其他 ${rest.length} 个来源` : `${rest.length} other sources`}</span>
              <span className="shrink-0 tabular-nums">{metric === 'cost' ? formatCost(restValue) : formatTokens(restValue)}</span>
            </div>
            <div className="mt-0.5 pl-4 text-[11px] tabular-nums text-[var(--text-faint)]">
              {formatShare(share(restValue))}{metric === 'cost' ? ` · ${formatTokens(restTokens)} tokens` : ''}
            </div>
          </div>
        )}
      </div>
    </div>
  )
}

function TokenComposition({ totals, lang }: { totals: TokenTotals | undefined; lang: string }) {
  const zh = lang === 'zh'
  const isDark = useDocumentDark()
  const parts = TOKEN_PARTS.map(part => ({
    key: part.key,
    label: zh ? part.labelZh : part.labelEn,
    color: isDark ? part.dark : part.light,
    value: totals ? part.value(totals) : 0,
  }))
  const total = parts.reduce((sum, part) => sum + part.value, 0)
  return (
    <section className="border-t border-[var(--border)] pt-5" aria-label={zh ? 'Token 构成' : 'Token breakdown'}>
      <div className="flex items-baseline justify-between gap-3">
        <h3 className="text-[13px] font-medium text-[var(--text)]">
          {zh ? 'Token 构成' : 'Token breakdown'}
          <span className="ml-2 font-normal tabular-nums text-[var(--text-muted)]">{formatTokens(total)}</span>
        </h3>
        <span className="text-xs text-[var(--text-muted)]">
          {zh ? '缓存命中率' : 'Cache hit rate'}
          <span className="ml-1.5 font-medium tabular-nums text-[var(--text)]">
            {formatPercent((totals?.cachedInputTokens ?? 0) / (totals?.inputTokens || 1))}
          </span>
        </span>
      </div>
      <div className={`mt-3 flex h-2 gap-0.5 overflow-hidden rounded-full ${TRACK}`} aria-hidden="true">
        {parts.map(part => part.value > 0 && (
          <i key={part.key} className="min-w-[3px]" style={{ flexGrow: part.value, flexBasis: 0, backgroundColor: part.color }} />
        ))}
      </div>
      <dl className="mt-4 grid grid-cols-2 gap-x-6 gap-y-4 sm:grid-cols-4">
        {parts.map(part => (
          <div key={part.key} className="min-w-0">
            <dt className="flex items-center gap-1.5 text-xs text-[var(--text-muted)]">
              <span className={SWATCH} style={{ backgroundColor: part.color }} />
              <span className="truncate">{part.label}</span>
            </dt>
            <dd className="mt-1 flex items-baseline gap-2">
              <span className="text-lg font-medium leading-6 tabular-nums text-[var(--text)]">{formatTokens(part.value)}</span>
              <span className="text-[11px] tabular-nums text-[var(--text-faint)]">{formatShare(total > 0 ? part.value / total : 0)}</span>
            </dd>
          </div>
        ))}
      </dl>
    </section>
  )
}

function EmptyRows({ children }: { children: ReactNode }) {
  return <div className="border-y border-[var(--border)] py-12 text-center text-xs text-[var(--text-muted)]">{children}</div>
}

function GroupTable({ rows, lang, type, metric }: { rows: UsageGroupStats[]; lang: string; type: 'provider' | 'model'; metric: UsageMetric }) {
  const zh = lang === 'zh'
  if (rows.length === 0) {
    return <EmptyRows>{zh ? '这个范围内暂无用量，试试更长的时间范围或刷新本地 CLI。' : 'No usage in this range. Try a longer range or refresh local CLI history.'}</EmptyRows>
  }
  const total = rows.reduce((sum, row) => sum + rowMetric(row, metric), 0)
  const peak = Math.max(0, ...rows.map(row => rowMetric(row, metric)))
  const head = 'py-2 text-[11px] font-normal text-[var(--text-muted)]'
  const cell = 'py-2.5 text-right align-top leading-5 tabular-nums'
  return (
    <div className="custom-scrollbar overflow-x-auto">
      <table className="w-full min-w-[560px] table-fixed border-collapse text-left text-[13px]">
        <thead>
          <tr className="border-b border-[var(--border)]">
            <th className={`${head} w-8 pr-2`}>#</th>
            <th className={`${head} pr-3`}>{type === 'model' ? (zh ? '模型' : 'Model') : (zh ? '来源' : 'Provider')}</th>
            <th className={`${head} w-28 px-3 text-right`}>{zh ? '费用' : 'Cost'}</th>
            <th className={`${head} w-20 px-3 text-right`}>{zh ? '占比' : 'Share'}</th>
            <th className={`${head} w-24 px-3 text-right`}>Tokens</th>
            <th className={`${head} w-16 pl-3 pr-1 text-right`}>{zh ? '会话' : 'Sessions'}</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row, index) => {
            const value = rowMetric(row, metric)
            return (
              <tr key={row.id} className="border-b border-[var(--divider)] text-[var(--text)] last:border-b-0">
                <td className="py-2.5 pr-2 align-top text-[11px] leading-5 tabular-nums text-[var(--text-faint)]">{index + 1}</td>
                <td className="py-2.5 pr-3">
                  <div className="flex min-w-0 items-center gap-2">
                    {type === 'model' ? <ModelIcon model={row.model || row.label} size={15} /> : <SourceIcon row={row} size={15} />}
                    <span className={`min-w-0 truncate font-medium ${type === 'model' ? 'font-mono text-[12px]' : ''}`} title={row.label}>{row.label}</span>
                    {type === 'model' && row.providerName && (
                      <span className="shrink-0 text-[11px] text-[var(--text-faint)]">{row.providerName}</span>
                    )}
                  </div>
                  <div className={`mt-1.5 h-[3px] rounded-full ${TRACK}`} aria-hidden="true">
                    <div
                      className="h-full rounded-full bg-[var(--accent)] opacity-75"
                      style={{ width: `${peak > 0 && value > 0 ? Math.max(1, (value / peak) * 100) : 0}%` }}
                    />
                  </div>
                </td>
                <td className={`${cell} px-3`}>
                  {isUnpriced(row) ? <span className="text-[var(--text-faint)]">—</span> : formatCost(row.costUsd)}
                  {(row.unpricedRequests ?? 0) > 0 && (
                    <div className="text-[11px] leading-4 text-[var(--text-faint)]">{row.unpricedRequests} {zh ? '条未计价' : 'unpriced'}</div>
                  )}
                </td>
                <td className={`${cell} px-3 text-[var(--text-muted)]`}>{formatShare(total > 0 ? value / total : 0)}</td>
                <td className={`${cell} px-3`}>{formatTokens(row.totalTokens)}</td>
                <td className={`${cell} pl-3 pr-1 text-[var(--text-muted)]`}>{formatCount(row.sessionCount)}</td>
              </tr>
            )
          })}
        </tbody>
      </table>
    </div>
  )
}

function LogsTable({ logs, lang }: { logs: UsageRecord[]; lang: string }) {
  const zh = lang === 'zh'
  if (logs.length === 0) return <EmptyRows>{zh ? '暂无请求日志' : 'No request logs'}</EmptyRows>
  return (
    <div className="custom-scrollbar overflow-x-auto">
      <div className="min-w-[600px] border-t border-[var(--border)] text-[12px]">
        {logs.map(record => {
          const isCli = record.source === 'cli'
          const tone = record.status === 'success' ? 'var(--success)' : record.status === 'cancelled' ? 'var(--warning)' : 'var(--danger)'
          const status = isCli ? (zh ? '已记录' : 'Recorded') : statusLabel(record.status, lang)
          const meta = [
            sourceLabel(record.source, lang),
            record.providerName || record.providerId,
            record.operation,
            formatReasoningEffort(record.reasoningEffort),
          ].filter(Boolean).join(' · ')
          return (
            <div
              key={record.id}
              className="grid grid-cols-[92px_minmax(0,1fr)_auto_60px_60px] items-center gap-x-4 border-b border-[var(--divider)] py-2 text-[var(--text)]"
            >
              <div className="flex items-center gap-2 tabular-nums text-[var(--text-muted)]" title={status}>
                <span className="h-1.5 w-1.5 shrink-0 rounded-full" style={{ backgroundColor: tone }} aria-hidden="true" />
                <span className="sr-only">{status}</span>
                {formatTime(record.createdAt, lang)}
              </div>
              <div className="min-w-0">
                <div className="flex min-w-0 items-center gap-2">
                  <ModelIcon model={record.model} size={14} />
                  <span className="min-w-0 truncate font-mono" title={record.model}>{record.model}</span>
                  {!isCli && record.status !== 'success' && (
                    <span className={`kv-tag shrink-0 ${record.status === 'cancelled' ? 'warn' : 'danger'}`}>{status}</span>
                  )}
                  {record.usageSource === 'missing' && <span className="kv-tag warn shrink-0">{zh ? 'Usage 缺失' : 'Usage missing'}</span>}
                </div>
                <div className="mt-0.5 truncate text-[11px] text-[var(--text-muted)]" title={meta}>{meta}</div>
              </div>
              <div className="grid grid-cols-[62px_54px_100px] items-center gap-x-2 text-[11px] tabular-nums text-[var(--text-muted)]">
                <span className="inline-flex items-center gap-1" title={zh ? '输入 Token' : 'Input tokens'}>
                  <ArrowDown aria-hidden="true" size={11} strokeWidth={1.8} className="text-[var(--text-faint)]" />
                  <span className="text-[var(--text)]">{formatOptionalTokens(record.inputTokens)}</span>
                </span>
                <span className="inline-flex items-center gap-1" title={zh ? '输出 Token' : 'Output tokens'}>
                  <ArrowUp aria-hidden="true" size={11} strokeWidth={1.8} className="text-[var(--text-faint)]" />
                  <span className="text-[var(--text)]">
                    {record.source === 'knowledge_base' ? '--' : formatOptionalTokens(record.outputTokens)}
                  </span>
                </span>
                <span
                  className="inline-flex items-center gap-1"
                  title={`${zh ? '缓存读取 Token' : 'Cache read tokens'}${(record.cacheCreationInputTokens ?? 0) > 0 ? ` · ${zh ? '写入' : 'write'} ${formatTokens(record.cacheCreationInputTokens)}` : ''}`}
                >
                  <Database aria-hidden="true" size={10} strokeWidth={1.7} className="text-[var(--text-faint)]" />
                  {formatOptionalTokens(record.cachedInputTokens)}
                  {(record.cacheCreationInputTokens ?? 0) > 0 && (
                    <span className="text-[var(--text-faint)]">+{formatTokens(record.cacheCreationInputTokens)}</span>
                  )}
                </span>
              </div>
              <div className="text-right tabular-nums">
                {record.costUsd == null ? <span className="text-[var(--text-faint)]">—</span> : formatCost(record.costUsd)}
              </div>
              <div className="text-right tabular-nums text-[var(--text-muted)]" title={zh ? '总耗时 / 首字耗时' : 'Total / time to first token'}>
                <div>{formatDuration(isCli ? null : record.durationMs)}</div>
                {(record.firstTokenMs ?? 0) > 0 && (
                  <div className="text-[10px] leading-4 text-[var(--text-faint)]">{zh ? '首字' : 'TTFT'} {formatDuration(record.firstTokenMs)}</div>
                )}
              </div>
            </div>
          )
        })}
      </div>
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
  const [filtersOpen, setFiltersOpen] = useState(view === 'calls')
  const [logPageIndex, setLogPageIndex] = useState(0)
  const [stats, setStats] = useState<UsageStatsResponse | null>(null)
  const [metric, setMetric] = useState<UsageMetric>('cost')
  const [syncing, setSyncing] = useState(false)
  const [scan, setScan] = useState<Awaited<ReturnType<typeof api.usageSyncCli>> | null>(null)
  const [loading, setLoading] = useState(false)
  const [clearing, setClearing] = useState(false)
  const [error, setError] = useState('')
  const [hoverSource, setHoverSource] = useState<string | null>(null)

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
  const busy = loading || syncing
  const hourly = range === 'today' || range === '1d'
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

  const activeProvider = providerSearch.trim()
  const toggleProviderFilter = useCallback((id: string) => {
    setProviderSearch(previous => (previous.trim() === id ? '' : id))
    setFiltersOpen(true)
  }, [])

  const activeFilters = [source !== 'all', status !== 'all', activeProvider !== '', modelSearch.trim() !== ''].filter(Boolean).length
  const providerRows = useMemo(() => sortByMetric(stats?.providerStats, metric), [metric, stats?.providerStats])
  const modelRows = useMemo(() => sortByMetric(stats?.modelStats, metric), [metric, stats?.modelStats])
  const trend = stats?.trend ?? []
  const chartSources = useMemo(
    () => providerRows.slice(0, SOURCE_ROW_LIMIT).map((row, index) => ({ id: row.providerId ?? row.id, label: row.label, color: shareColor(index) })),
    [providerRows],
  )
  const otherSources = providerRows.length - chartSources.length
  const allUnpriced = Boolean(summary?.totalRequests) && summary?.unpricedRequests === summary?.totalRequests
  const heroValue = !stats ? '—' : metric === 'cost' ? (allUnpriced ? '—' : formatCost(summary?.totalCostUsd)) : formatTokens(summary?.totalTokens)

  const toolbar = (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-2">
        <Seg value={range} onChange={updateRange} options={rangeOptions} ariaLabel={zh ? '时间范围' : 'Time range'} />
        <div className="ml-auto flex items-center gap-1">
          {view === 'app' && (
            <div className="mr-1">
              <Seg
                value={metric}
                onChange={setMetric}
                options={[{ id: 'cost', label: zh ? '费用' : 'Cost' }, { id: 'tokens', label: 'Tokens' }]}
                ariaLabel={zh ? '统计口径' : 'Metric'}
              />
            </div>
          )}
          <span className="relative inline-flex">
            <IconButton
              variant="ghost"
              label={zh ? '筛选' : 'Filters'}
              aria-pressed={filtersOpen}
              aria-expanded={filtersOpen}
              onClick={() => setFiltersOpen(open => !open)}
              data-tauri-drag-region="false"
            >
              <SlidersHorizontal size={14} />
            </IconButton>
            {activeFilters > 0 && !filtersOpen && (
              <span className="pointer-events-none absolute right-1 top-1 h-1.5 w-1.5 rounded-full bg-[var(--accent)]" aria-hidden="true" />
            )}
          </span>
          <IconButton variant="ghost" label={zh ? '刷新' : 'Refresh'} onClick={() => void refresh()} disabled={busy} data-tauri-drag-region="false">
            <RefreshCw size={14} className={busy ? 'animate-spin' : ''} />
          </IconButton>
          <IconButton variant="danger" label={zh ? '清空' : 'Clear'} onClick={() => void clearStats()} disabled={clearing || busy} data-tauri-drag-region="false">
            <Trash2 size={14} />
          </IconButton>
        </div>
      </div>
      {filtersOpen && (
        <div className="flex flex-wrap items-center gap-2">
          <Select
            className="w-36"
            value={source}
            onChange={updateSource}
            ariaLabel={zh ? '来源' : 'Source'}
            options={SOURCE_OPTIONS.map(value => ({ value, label: sourceLabel(value, lang) }))}
          />
          <Select
            className="w-32"
            value={status}
            onChange={updateStatus}
            ariaLabel={zh ? '状态' : 'Status'}
            options={STATUS_OPTIONS.map(value => ({ value, label: statusLabel(value, lang) }))}
          />
          <SearchInput
            className="min-w-[150px] flex-1"
            value={providerSearch}
            onChange={setProviderSearch}
            placeholder={zh ? '搜索 Provider' : 'Search provider'}
            clearLabel={zh ? '清除 Provider 筛选' : 'Clear provider filter'}
          />
          <SearchInput
            className="min-w-[150px] flex-1"
            value={modelSearch}
            onChange={setModelSearch}
            placeholder={zh ? '搜索模型' : 'Search model'}
            clearLabel={zh ? '清除模型筛选' : 'Clear model filter'}
          />
        </div>
      )}
    </div>
  )

  const overview = (
    <>
      <section className={`grid gap-x-10 gap-y-6 pt-1 transition-opacity lg:grid-cols-[minmax(0,5fr)_minmax(0,9fr)] ${busy ? 'opacity-60' : ''}`}>
        <div className="min-w-0">
          <div className="text-xs text-[var(--text-muted)]">{metric === 'cost' ? (zh ? '预估费用' : 'Estimated cost') : (zh ? '处理 Token' : 'Processed tokens')}</div>
          <div className="mt-2 text-[40px] font-semibold leading-none tracking-[-0.03em] tabular-nums text-[var(--text)]">{heroValue}</div>
          <p className="mt-3 text-xs tabular-nums text-[var(--text-muted)]">
            {formatCount(summary?.sessionCount)} {zh ? '个会话' : 'sessions'}
            {' · '}{formatCount(summary?.totalRequests)} {zh ? '次请求' : 'requests'}
            {' · '}{metric === 'cost' ? `${formatTokens(summary?.totalTokens)} tokens` : (allUnpriced ? '—' : formatCost(summary?.totalCostUsd))}
          </p>
          <SourceShare rows={providerRows} metric={metric} lang={lang} activeId={activeProvider} onToggle={toggleProviderFilter} onHover={setHoverSource} />
        </div>
        <div className="min-w-0" role="group" aria-label={zh ? '用量趋势' : 'Usage trend'}>
          <div className="mb-1 flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
            <h3 className="text-[13px] font-medium text-[var(--text)]">
              {metric === 'cost' ? (zh ? '费用趋势' : 'Cost over time') : (zh ? 'Token 趋势' : 'Tokens over time')}
            </h3>
            <span className="text-[11px] text-[var(--text-faint)]">
              {metric === 'cost' ? (zh ? '按来源堆叠' : 'Stacked by source') : (zh ? '按类型堆叠' : 'Stacked by type')}
              {' · '}{hourly ? (zh ? '每小时' : 'hourly') : (zh ? '每天' : 'daily')}
            </span>
          </div>
          <TrendChart
            points={trend}
            lang={lang}
            metric={metric}
            sources={chartSources}
            otherLabel={otherSources > 0 ? (zh ? `其他 ${otherSources} 个来源` : `${otherSources} other sources`) : undefined}
            highlightId={hoverSource}
          />
          <TrendStats points={trend} metric={metric} lang={lang} hourly={hourly} summary={summary} />
        </div>
      </section>

      <TokenComposition totals={summary} lang={lang} />
    </>
  )

  const notes = (
    <footer className="space-y-2 border-t border-[var(--border)] pt-4 text-[11px] leading-[18px] text-[var(--text-muted)]">
      <div className="flex gap-2">
        <Info size={12} className="mt-[3px] shrink-0 text-[var(--text-faint)]" aria-hidden="true" />
        <div className="min-w-0 space-y-1">
          <p>
            <span>{zh ? '费用按 LiteLLM API 价格估算，或由 CLI 上报；' : 'Cost is estimated from LiteLLM API prices or reported by the CLI. '}</span>
            <span>{zh ? '不代表订阅账单或实际扣费。' : 'Not a subscription bill or actual charge.'}</span>
          </p>
          {(summary?.unpricedRequests ?? 0) > 0 && (
            <p role="status">
              {zh
                ? `${summary?.unpricedRequests} 条记录未计价；Token 已计入，费用合计不包含这些记录。`
                : `${summary?.unpricedRequests} records are unpriced. Tokens are included; their cost is not.`}
            </p>
          )}
          <details>
            <summary className="w-fit cursor-pointer select-none hover:text-[var(--text)]">
              {syncing ? (zh ? '正在同步本地 CLI…' : 'Syncing local CLI…') : (zh ? '本地 CLI 数据来源' : 'Local CLI data sources')}
            </summary>
            <div className="mt-1.5 space-y-1" role="status">
              {scan?.sources.map(item => (
                <div key={item.id}>
                  <span className="text-[var(--text)]">{item.label}</span>
                  {' · '}{item.files} {zh ? '个历史文件' : 'history files'}
                  {item.skipped > 0 ? ` · ${item.skipped} ${zh ? '行无法读取' : 'unreadable lines'}` : ''}
                  {item.errors.map(message => <p key={message} className="break-all text-[var(--danger)]">{message}</p>)}
                </div>
              ))}
            </div>
            <p className="mt-1.5">
              {zh
                ? '读取本机原生历史，刷新时同步。未找到记录不表示零用量。Cursor、Antigravity 等尚未接入此历史扫描。'
                : 'Reads local native history on refresh. Missing history does not mean zero usage. Cursor, Antigravity and other sources are not scanned yet.'}
            </p>
          </details>
        </div>
      </div>
    </footer>
  )

  const breakdown = (
    <section className={view === 'app' ? 'border-t border-[var(--border)] pt-5' : ''} aria-label={zh ? '用量明细' : 'Usage breakdown'}>
      <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
        <h3 className="text-[13px] font-medium text-[var(--text)]">
          {zh ? '明细' : 'Breakdown'}
          {viewMode === 'logs' && stats && (
            <span className="ml-2 font-normal tabular-nums text-[var(--text-muted)]">{formatCount(totalLogs)}</span>
          )}
        </h3>
        <Seg
          value={viewMode}
          onChange={setViewMode}
          ariaLabel={zh ? '明细维度' : 'Breakdown view'}
          options={[
            { id: 'models', label: zh ? '模型' : 'Models' },
            { id: 'providers', label: 'Provider' },
            { id: 'logs', label: zh ? '请求日志' : 'Logs' },
          ]}
        />
      </div>
      <div className={`transition-opacity ${busy ? 'opacity-60' : ''}`}>
        {viewMode === 'logs' && <LogsTable logs={stats?.logs ?? []} lang={lang} />}
        {viewMode === 'providers' && <GroupTable rows={providerRows} lang={lang} type="provider" metric={metric} />}
        {viewMode === 'models' && <GroupTable rows={modelRows} lang={lang} type="model" metric={metric} />}
      </div>
      {stats && viewMode === 'logs' && totalLogs > 0 && (
        <div className="mt-3 flex flex-wrap items-center justify-between gap-2 text-[11px] text-[var(--text-muted)]">
          <span className="tabular-nums">
            {zh
              ? `显示 ${pageRangeLabel(logPageIndex, LOG_PAGE_SIZE, totalLogs)} 条`
              : `Showing ${pageRangeLabel(logPageIndex, LOG_PAGE_SIZE, totalLogs)}`}
          </span>
          <div className="flex items-center gap-1">
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setLogPageIndex(page => Math.max(0, page - 1))}
              disabled={!canGoPrev}
              data-tauri-drag-region="false"
            >
              <ChevronLeft size={11} />
              {zh ? '上一页' : 'Prev'}
            </Button>
            <span className="min-w-12 text-center tabular-nums">
              {logPageIndex + 1} / {pageCount}
            </span>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setLogPageIndex(page => Math.min(pageCount - 1, page + 1))}
              disabled={!canGoNext}
              data-tauri-drag-region="false"
            >
              {zh ? '下一页' : 'Next'}
              <ChevronRight size={11} />
            </Button>
          </div>
        </div>
      )}
    </section>
  )

  return (
    <div className="space-y-6 pb-4" aria-busy={busy}>
      {error && <ErrorNotice message={error} />}
      {toolbar}
      {view === 'app' && overview}
      {breakdown}
      {view === 'app' && notes}
    </div>
  )
}
