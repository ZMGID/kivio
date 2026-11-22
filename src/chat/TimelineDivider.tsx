import type { ReactNode } from 'react'
import { ArrowLeftRight, ArrowRight, SlidersHorizontal } from 'lucide-react'
import { Button } from '../components/Button'
import type { Lang } from '../components/i18n'
import { formatEffortLabel } from './externalRuntimeCatalog'
import type { TurnModel } from './types'

interface TimelineDividerProps {
  children: ReactNode
  label: string
  title?: string
  animate?: boolean
  pending?: boolean
  action?: { label: string; open: () => void }
  'data-compaction-divider-id'?: string
  'data-context-clear-divider-id'?: string
}

export function TimelineDivider({ children, label, title, animate, pending, action, ...attributes }: TimelineDividerProps) {
  const content = <span className="chat-timeline-divider-content">{children}</span>
  // An interactive child of an ARIA separator is presentational. Keep the action accessible.
  return <div {...attributes} role={action ? 'group' : pending ? 'status' : 'separator'}
    aria-label={label} title={title}
    className={`chat-timeline-divider${animate ? ' chat-timeline-divider--animate' : ''}`}>
    <span className="chat-timeline-divider-line" aria-hidden="true" />
    {action ? <Button size="sm" variant="ghost" aria-label={action.label} onClick={action.open}>{content}</Button> : content}
    <span className="chat-timeline-divider-line" aria-hidden="true" />
  </div>
}

export function TurnModelDivider({ turn, lang }: { turn: TurnModel; lang: Lang }) {
  const from = turn.previous
  const to = turn.current
  if (!from) return null
  const provider = from.provider !== to.provider
  const model = provider || from.model !== to.model
  const effort = from.effort !== to.effort && (!model || Boolean(from.effort && to.effort))
  if (!model && !effort) return null
  const endpoint = (selection: TurnModel['current']) => [
    provider && selection.provider_name,
    model && (selection.model_name || selection.model || 'Auto'),
    effort && (selection.effort ? formatEffortLabel(selection.effort, selection.effort) : 'Auto'),
  ].filter(Boolean).join(' ')
  const left = endpoint(from)
  const right = endpoint(to)
  const Icon = provider ? ArrowLeftRight : SlidersHorizontal
  const change = `${left} → ${right}`
  const label = lang === 'zh' ? `模型设置变更，${change}` : `Model selection changed, ${change}`
  return <TimelineDivider label={label} title={change}>
    <Icon size={13} aria-hidden="true" />
    <span>{left}</span><ArrowRight size={12} aria-hidden="true" /><span>{right}</span>
  </TimelineDivider>
}
