import { useLayoutEffect, useState } from 'react'
import { Scissors } from 'lucide-react'
import { formatTokens } from '../utils/tokens'
import { i18n, type Lang } from '../components/i18n'
import {
  compactionRecordTokens,
  compactionTriggerLabel,
  hasCompactionTokenDetail,
  type CompactionBoundaryView,
} from './compactionBoundary'

interface CompactionDividerProps {
  boundary: CompactionBoundaryView
  lang?: Lang
  animate?: boolean
}

function formatDurationHint(record: CompactionBoundaryView['record']): string {
  const created = record.created_at ?? record.createdAt
  if (!created) return ''
  return new Date(created * 1000).toLocaleString()
}

export function CompactionDivider({ boundary, lang = 'zh', animate = false }: CompactionDividerProps) {
  const t = i18n[lang]
  const { record } = boundary
  const tokens = compactionRecordTokens(record)
  const showTokens = hasCompactionTokenDetail(record)
  const trigger = compactionTriggerLabel(record.trigger, t)
  const [entered, setEntered] = useState(!animate)

  useLayoutEffect(() => {
    if (!animate) {
      setEntered(true)
      return
    }
    setEntered(false)
    let cancelled = false
    const frame = requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        if (!cancelled) setEntered(true)
      })
    })
    return () => {
      cancelled = true
      cancelAnimationFrame(frame)
    }
  }, [animate, record.id])

  const tooltip = showTokens
    ? [
      `${t.contextCompactionDivider} (${trigger})`,
      '',
      `${t.contextCompactionBefore}: ${tokens.before.toLocaleString('en-US')}`,
      `${t.contextCompactionAfter}: ${tokens.after.toLocaleString('en-US')}`,
      `${t.contextCompactionFreed}: ${Math.max(0, tokens.before - tokens.after).toLocaleString('en-US')}`,
      formatDurationHint(record) ? `\n${formatDurationHint(record)}` : '',
    ].join('\n').trim()
    : `${t.contextCompactionDivider} (${trigger})`

  return (
    <div
      className={`chat-compaction-divider ${entered ? 'chat-compaction-divider--animate' : 'chat-compaction-divider--pre-enter'}`}
      data-compaction-divider-id={record.id}
      title={tooltip}
      role="separator"
      aria-label={t.contextCompactionDividerAria}
    >
      <span className="chat-compaction-divider-line" aria-hidden="true" />
      <span className="chat-compaction-divider-content">
        <Scissors size={14} aria-hidden="true" />
        <span>{t.contextCompactionDivider}</span>
        {showTokens && (
          <span className="chat-compaction-divider-arrow">
            {formatTokens(tokens.before)} → {formatTokens(tokens.after)}
          </span>
        )}
      </span>
      <span className="chat-compaction-divider-line" aria-hidden="true" />
    </div>
  )
}
