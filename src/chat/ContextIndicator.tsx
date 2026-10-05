import { Button, IconButton } from '../components/Button'
import { Archive, Eraser, RefreshCw, Square } from 'lucide-react'
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import {
  autoCompactPercent,
  contextBreakdown,
  CONTEXT_CRITICAL_PERCENT,
  CONTEXT_WARNING_PERCENT,
  reportedContextTokens,
} from './contextPanel'
import { i18n, type I18n, type Lang } from '../components/i18n'
import type { ConversationContextState } from './types'
import { usePopoverMenu } from './usePopoverMenu'
import { formatTokens } from '../utils/tokens'

const PANEL_WIDTH = 280
const PANEL_GAP = 8
const VIEW_MARGIN = 8
// 弹层尽量贴底栏，限制高度，少盖住上方对话消息。
const PANEL_MAX_H = 360
// 圆环：viewBox 20×20 里留 1.5px 描边半宽 + 1px 余量。
const RING_RADIUS = 7.5
const RING_CIRCUMFERENCE = 2 * Math.PI * RING_RADIUS

const SEGMENT_LABELS: Record<string, keyof I18n> = {
  system_prompt: 'contextSegmentSystemPrompt',
  tools: 'contextSegmentTools',
  conversation: 'contextSegmentConversation',
}

interface ContextIndicatorProps {
  contextState?: ConversationContextState | null
  messageCount?: number
  lastMessageId?: string
  loading?: boolean
  compressing?: boolean
  generating?: boolean
  error?: string
  usesExternalRuntime?: boolean
  onRefresh?: () => void
  onCompress?: () => void
  onStopCompression?: () => void
  onClear?: () => void
  placement?: 'up' | 'down'
  lang?: Lang
}

function valueFrom<T>(snake: T | undefined, camel: T | undefined, fallback: T): T {
  return snake ?? camel ?? fallback
}

function statusColor(status: string, ratio: number | null): string {
  if (status === 'stale') return '#A15C2F'
  if (status === 'compressed') return '#3E8B60'
  if (status === 'critical' || (ratio ?? 0) >= CONTEXT_CRITICAL_PERCENT / 100) return '#C24135'
  if (status === 'warning' || (ratio ?? 0) >= CONTEXT_WARNING_PERCENT / 100) return '#B7791F'
  return '#3E8B60'
}



function messageCountLabel(messageCount: number, compressedMessageCount: number, t: I18n): string {
  if (compressedMessageCount > 0) {
    return t.contextMessagesCompressed
      .replace('{count}', String(messageCount))
      .replace('{compressed}', String(compressedMessageCount))
  }
  return t.contextMessages.replace('{count}', String(messageCount))
}

export function ContextIndicator({
  contextState,
  messageCount = 0,
  lastMessageId,
  loading = false,
  compressing = false,
  generating = false,
  error = '',
  usesExternalRuntime = false,
  onRefresh,
  onCompress,
  onStopCompression,
  onClear,
  placement: _placement = 'down',
  lang = 'zh',
}: ContextIndicatorProps) {
  void _placement
  const t = i18n[lang]
  const [open, setOpen] = useState(false)
  // 用 bottom/right 锚定，避免量高不准时整块飘到对话消息中间。
  const [pos, setPos] = useState<{ bottom: number; right: number; maxH: number; width: number } | null>(null)
  const triggerRef = useRef<HTMLDivElement>(null)
  const popoverRef = useRef<HTMLDivElement>(null)
  usePopoverMenu(open, () => setOpen(false), popoverRef)

  const reportedTokens = reportedContextTokens(contextState)
  const contextWindowTokens = valueFrom(
    contextState?.context_window_estimated, contextState?.contextWindowEstimated, false,
  ) ? null : valueFrom(
    contextState?.context_window_tokens,
    contextState?.contextWindowTokens,
    null,
  )
  const usageRatio = reportedTokens != null && contextWindowTokens != null && contextWindowTokens > 0
    ? reportedTokens / contextWindowTokens : null
  const status = contextState?.status ?? 'unknown'
  const contextSource = valueFrom(contextState?.context_source, contextState?.contextSource, null)
  const isExternalContext =
    usesExternalRuntime || contextSource === 'external_cli'
  const compressedMessageCount = valueFrom(
    contextState?.compressed_message_count,
    contextState?.compressedMessageCount,
    0,
  )
  const compressionCount = valueFrom(
    contextState?.compression_count,
    contextState?.compressionCount,
    0,
  )
  const color = statusColor(status, usageRatio)
  const percentFormat = new Intl.NumberFormat(lang === 'zh' ? 'zh-CN' : 'en', {
    style: 'percent', maximumFractionDigits: 1,
  })
  const windowPart = contextWindowTokens ? formatTokens(contextWindowTokens) : '—'
  const displayMetric = reportedTokens == null && contextWindowTokens == null
    ? '—'
    : `${reportedTokens == null ? '—' : formatTokens(reportedTokens)}/${windowPart}`
      + (usageRatio == null ? '' : ` (${percentFormat.format(usageRatio)})`)
  const sourceLabel = reportedTokens == null
    ? ''
    : (isExternalContext ? t.contextSourceCliReported : t.contextUsageLastPrompt)
  const ringRatio = usageRatio == null ? 0 : Math.max(0, Math.min(1, usageRatio))
  const segments = contextBreakdown(contextState?.segments).map((segment) => ({
    ...segment, label: t[SEGMENT_LABELS[segment.id]],
  }))
  const cacheRate = contextState?.cache_hit_rate ?? contextState?.cacheHitRate
  const showCache = !isExternalContext && cacheRate != null && Number.isFinite(cacheRate)
    && cacheRate >= 0.78 && cacheRate <= 1
  const lastClearUntilId = (
    contextState?.clear_boundaries ?? contextState?.clearBoundaries ?? []
  ).at(-1)?.source_until_message_id
    ?? (contextState?.clear_boundaries ?? contextState?.clearBoundaries ?? []).at(-1)?.sourceUntilMessageId
    ?? null
  const liveContextEmpty = lastMessageId != null && lastMessageId === lastClearUntilId
  const canCompress = Boolean(onCompress) && !compressing && !loading && messageCount > 2 && !liveContextEmpty
  const canClear = Boolean(onClear)
    && !compressing
    && !loading
    && !generating
    && messageCount > 0
    && lastMessageId != null
    && !liveContextEmpty
  const compressLabel = isExternalContext
    ? (compressing ? t.contextCliCompacting : t.contextCliCompact)
    : (compressing ? t.contextCompressing : t.contextCompress)
  // Automatic compaction runs inside a generation, and stopping it stops that generation.
  const stopLabel = generating ? t.contextStopGeneration : t.contextStopCompression
  const autoPercent = autoCompactPercent(contextState)
  const autoHint = isExternalContext || autoPercent == null
    ? null
    : t.contextPanelAutoCompress.replace('{auto}', String(autoPercent))
  // 只在真正压过时露出次数；自动压缩阈值放压缩按钮 title，不占正文。
  const compressMeta = compressionCount > 0
    ? t.contextCompressionCount.replace('{count}', String(compressionCount))
    : null

  // 锚定在圆环旁：优先左侧 + 底边与按钮对齐（贴底栏，少盖对话消息）；
  // 左侧不够再翻到按钮正上方。用 bottom/right，不依赖量高。
  const place = useCallback(() => {
    const trigger = triggerRef.current
    if (!trigger) return
    const rect = trigger.getBoundingClientRect()
    const width = Math.min(PANEL_WIDTH, window.innerWidth - VIEW_MARGIN * 2)
    const spaceLeft = rect.left - VIEW_MARGIN
    const spaceAbove = rect.top - VIEW_MARGIN - PANEL_GAP
    const openLeft = spaceLeft >= width + PANEL_GAP

    if (openLeft) {
      // 底边对齐圆环底 → 弹层坐在底栏高度带，只向上长一截
      const bottom = Math.max(VIEW_MARGIN, window.innerHeight - rect.bottom)
      const right = window.innerWidth - rect.left + PANEL_GAP
      const maxH = Math.max(96, Math.min(PANEL_MAX_H, window.innerHeight - bottom - VIEW_MARGIN))
      setPos({ bottom, right, maxH, width })
      return
    }

    // 回退：贴在圆环正上方，右缘对齐
    const bottom = window.innerHeight - rect.top + PANEL_GAP
    const right = Math.max(VIEW_MARGIN, window.innerWidth - rect.right)
    const maxH = Math.max(96, Math.min(PANEL_MAX_H, spaceAbove))
    setPos({ bottom, right, maxH, width })
  }, [])

  useLayoutEffect(() => {
    if (!open) {
      setPos(null)
      return
    }
    place()
    window.addEventListener('resize', place)
    window.addEventListener('scroll', place, true)
    return () => {
      window.removeEventListener('resize', place)
      window.removeEventListener('scroll', place, true)
    }
  }, [open, place, displayMetric, compressMeta, error])

  useEffect(() => {
    if (!open) return
    const onDown = (e: MouseEvent) => {
      const target = e.target as Node
      if (triggerRef.current?.contains(target) || popoverRef.current?.contains(target)) return
      setOpen(false)
    }
    document.addEventListener('mousedown', onDown)
    return () => document.removeEventListener('mousedown', onDown)
  }, [open])

  const panel = open
    ? createPortal(
        <div
          ref={popoverRef}
          className="chat-motion-popover fixed z-[200] flex flex-col overflow-hidden kv-menu"
          style={{
            padding: 10,
            bottom: pos?.bottom ?? 0,
            right: pos?.right ?? 0,
            width: pos?.width ?? PANEL_WIDTH,
            maxHeight: pos?.maxH,
            visibility: pos ? 'visible' : 'hidden',
            ['--chat-popover-origin' as string]: 'bottom right',
          }}
          data-tauri-drag-region="false"
        >
          <div className="mb-1.5 flex items-center justify-between gap-2" title={sourceLabel || undefined}>
            <span className="text-[12px] font-medium text-neutral-800">{t.contextPanelTitle}</span>
            <span className="text-[11px] tabular-nums text-neutral-500">{displayMetric}</span>
          </div>

          <div className="h-1 shrink-0 overflow-hidden rounded-full bg-neutral-100"
            role="progressbar" aria-label={t.contextPanelTitle}
            aria-valuenow={usageRatio == null ? undefined : Math.min(100, usageRatio * 100)}
            aria-valuemin={0} aria-valuemax={100} aria-valuetext={displayMetric}>
            {usageRatio != null && (
              <div className="flex h-full bg-[var(--accent)]" style={{ width: `${ringRatio * 100}%` }}>
                {!isExternalContext && segments.map((segment) => (
                  <span key={segment.id} className="h-full"
                    style={{ width: `${segment.percent * 100}%`, backgroundColor: segment.color }} />
                ))}
              </div>
            )}
          </div>
          {!isExternalContext && segments.length > 0 && (
            <div className="custom-scrollbar mt-2 min-h-0 overflow-y-auto" title={t.contextBreakdownHint}>
                <div className="pb-1 text-right text-[10px] leading-none text-neutral-400">{t.contextCharacterShare}</div>
                <dl className="space-y-1">
                  {segments.map((segment) => (
                    <div key={segment.id} className="flex items-center gap-2 text-[12px]">
                      <span
                        className="size-2 shrink-0 rounded-sm"
                        style={{ backgroundColor: segment.color }}
                        aria-hidden="true"
                      />
                      <dt className="min-w-0 flex-1 truncate" title={segment.label}>{segment.label}</dt>
                      <dd className="flex shrink-0 items-baseline gap-2 tabular-nums">
                        <span className="text-neutral-700">
                          {segment.estimatedTokens == null ? '—' : `≈ ${formatTokens(segment.estimatedTokens)}`}
                        </span>
                        <span className="min-w-[38px] text-right text-neutral-500" title={t.contextCharacterShare}>{percentFormat.format(segment.percent)}</span>
                      </dd>
                    </div>
                  ))}
                </dl>
            </div>
          )}
          {showCache && (
            <div className="mt-1.5 flex items-center justify-between border-t border-[var(--theme-surface-border)] pt-1.5 text-[11px] text-neutral-500"
              title={t.contextCacheHint}>
              <span>{t.contextCacheHitRate}</span>
              <span className="tabular-nums">{percentFormat.format(cacheRate)}</span>
            </div>
          )}
          <div className="mt-1.5 flex items-center justify-end gap-1 border-t border-[var(--theme-surface-border)] pt-1">
            {compressMeta && (
              <span className="min-w-0 flex-1 truncate text-[10px] text-neutral-400 dark:text-neutral-500" title={compressMeta}>
                {compressMeta}
              </span>
            )}
            <IconButton
              variant="ghost"
              size="sm"
              label={t.contextRefreshAria}
              title={t.contextRefresh}
              onClick={onRefresh}
              disabled={loading}
            >
              <RefreshCw size={13} strokeWidth={1.9} className={loading ? 'animate-spin' : ''} />
            </IconButton>
            <Button
              variant="ghost"
              size="sm"
              aria-label={compressing && onStopCompression ? stopLabel : t.contextCompressAria}
              title={autoHint ? `${compressLabel} · ${autoHint}` : compressLabel}
              onClick={compressing && onStopCompression ? onStopCompression : onCompress}
              disabled={compressing ? !onStopCompression : !canCompress}
            >
              {compressing ? <Square size={13} /> : <Archive size={13} strokeWidth={1.9} />}
              <span>{compressing && onStopCompression ? stopLabel : compressLabel}</span>
            </Button>
            {onClear && (
              <Button
                variant="ghost"
                size="sm"
                aria-label={t.contextClearAria}
                title={t.contextClearAria}
                onClick={() => {
                  onClear()
                  setOpen(false)
                }}
                disabled={!canClear}
              >
                <Eraser size={13} strokeWidth={1.9} />
                <span>{t.contextClear}</span>
              </Button>
            )}
          </div>

          {error && (
            <p className="mt-1 text-[10px] text-danger">
              {error}
            </p>
          )}
        </div>,
        document.body,
      )
    : null

  return (
    <div className="relative" ref={triggerRef} data-tauri-drag-region="false">
      <button
        type="button"
        className="grid size-7 shrink-0 place-items-center rounded-full text-neutral-600 transition-colors hover:bg-neutral-100 active:scale-[0.97]"
        aria-label={t.contextTriggerAria}
        title={loading
          ? t.contextTriggerLoading
          : [
            displayMetric,
            sourceLabel,
            messageCount > 0 ? messageCountLabel(messageCount, compressedMessageCount, t) : '',
          ].filter(Boolean).join(' · ')}
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        {/* SVG 描边环，不用 conic-gradient + 白心盖中间：白心在点阵/半透底上就是一坨实白，
            且 conic 的边缘是锯齿。stroke 的圆心天然透空，底色直接透上来。 */}
        <svg viewBox="0 0 20 20" className="size-5 -rotate-90" aria-hidden="true">
          <circle
            cx="10"
            cy="10"
            r={RING_RADIUS}
            fill="none"
            strokeWidth="3"
            className="stroke-black/[0.16] dark:stroke-white/[0.20]"
          />
          {ringRatio > 0 && (
            <circle
              cx="10"
              cy="10"
              r={RING_RADIUS}
              fill="none"
              stroke={color}
              strokeWidth="3"
              strokeLinecap="round"
              strokeDasharray={RING_CIRCUMFERENCE}
              strokeDashoffset={RING_CIRCUMFERENCE * (1 - ringRatio)}
              className="transition-[stroke-dashoffset,stroke] duration-500 ease-out"
            />
          )}
        </svg>
      </button>
      {panel}
    </div>
  )
}
