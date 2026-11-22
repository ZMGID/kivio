import { TimelineDivider } from './TimelineDivider'
import { Minimize2 } from 'lucide-react'
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
  const showTokens = hasCompactionTokenDetail(record)
  const trigger = compactionTriggerLabel(record.trigger, t)
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
    <TimelineDivider data-compaction-divider-id={record.id}
      title={tooltip} label={t.contextCompactionDividerAria}>
      <Minimize2 size={14} aria-hidden="true" />
      <span>{t.contextCompactionDivider}</span>
      {showTokens && <span className="chat-compaction-divider-arrow">
        {formatTokens(tokens.before)} → {formatTokens(tokens.after)}
      </span>}
    </TimelineDivider>
  )
}
