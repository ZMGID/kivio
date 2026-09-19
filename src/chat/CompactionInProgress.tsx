import { i18n, type Lang } from '../components/i18n'

interface CompactionInProgressProps {
  lang?: Lang
}

export function CompactionInProgress({ lang = 'zh' }: CompactionInProgressProps) {
  const t = i18n[lang]
  return (
    <div className="chat-compaction-progress chat-motion-soft-pulse" role="status" aria-live="polite">
      <span className="chat-compaction-divider-line chat-compaction-divider-line--dim" aria-hidden="true" />
      <span className="chat-compaction-progress-label chat-motion-tool-shimmer">
        {t.contextCompressing}
      </span>
      <span className="chat-compaction-divider-line chat-compaction-divider-line--dim" aria-hidden="true" />
    </div>
  )
}
