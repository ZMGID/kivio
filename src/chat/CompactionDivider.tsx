import { TimelineDivider } from './TimelineDivider'
import { Minimize2 } from 'lucide-react'
import { formatTokens } from '../utils/tokens'
import { i18n, type Lang } from '../components/i18n'
import {
  compactionRecordTokens,
  compactionTriggerLabel,
  type CompactionBoundaryView,
} from './compactionBoundary'

interface CompactionDividerProps {
  boundary: CompactionBoundaryView
  lang?: Lang
}

function formatDurationHint(record: CompactionBoundaryView['record']): string {
  const created = record.created_at ?? record.createdAt
  if (!created) return ''
  return new Date(created * 1000).toLocaleString()
}

export function CompactionDivider({ boundary, lang = 'zh' }: CompactionDividerProps) {
  const t = i18n[lang]
  const { record } = boundary
  const tokens = compactionRecordTokens(record)
  // Boundary records use zero as the placeholder for an unreported CLI count.
  const hasPair = tokens.before > 0 && tokens.after > 0
  const trigger = compactionTriggerLabel(record.trigger, t)
  const lines = []
  if (tokens.before > 0) lines.push(`${t.contextCompactionBefore}: ${tokens.before.toLocaleString('en-US')}`)
  if (tokens.after > 0) lines.push(`${t.contextCompactionAfter}: ${tokens.after.toLocaleString('en-US')}`)
  if (hasPair) {
    lines.push(`${t.contextCompactionFreed}: ${Math.max(0, tokens.before - tokens.after).toLocaleString('en-US')}`)
  }
  const tooltip = [
    `${t.contextCompactionDivider} (${trigger})`,
    lines.join('\n'),
    formatDurationHint(record),
  ].filter(Boolean).join('\n\n')

  return (
    <TimelineDivider data-compaction-divider-id={record.id}
      title={tooltip} label={t.contextCompactionDividerAria}>
      <Minimize2 size={14} aria-hidden="true" />
      <span>{t.contextCompactionDivider}</span>
      {hasPair && <span className="chat-compaction-divider-arrow">
        {formatTokens(tokens.before)} → {formatTokens(tokens.after)}
      </span>}
    </TimelineDivider>
  )
}
