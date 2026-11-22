import { TimelineDivider } from './TimelineDivider'
import { i18n, type Lang } from '../components/i18n'
import type { ContextClearBoundaryView } from './contextClearBoundary'

interface ContextClearDividerProps {
  boundary: ContextClearBoundaryView
  lang?: Lang
  animate?: boolean
}

export function ContextClearDivider({ boundary, lang = 'zh', animate = false }: ContextClearDividerProps) {
  const t = i18n[lang]
  return (
    <TimelineDivider animate={animate} data-context-clear-divider-id={boundary.record.id} label={t.contextClearDividerAria}>
      <span className="chat-context-clear-badge">{t.contextClearDivider}</span>
    </TimelineDivider>
  )
}
