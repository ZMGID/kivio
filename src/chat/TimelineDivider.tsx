import type { ReactNode } from 'react'
import { ArrowLeftRight, ArrowRight, SlidersHorizontal } from 'lucide-react'
import { AgentIcon } from '../components/AgentIcon'
import { Button } from '../components/Button'
import type { Lang } from '../components/i18n'
import { formatEffortLabel } from './externalRuntimeCatalog'
import type { AgentHandoff, AgentIdentity, TurnModel } from './types'

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

function handoffAgentName(agent: AgentIdentity) {
  if (agent.kind === 'external') return agent.name || agent.agent_id
  const runtime = agent.kind === 'chat' ? 'Kivio Chat' : 'Kivio Agent'
  return [runtime, agent.name || agent.provider_id].filter(Boolean).join(' · ')
}

export function AgentHandoffDivider({ handoff, pending, lang }: { handoff: AgentHandoff; pending: boolean; lang: Lang }) {
  const label = lang === 'zh'
    ? pending ? '待交接上下文' : '上下文交接'
    : pending ? 'Context handoff pending' : 'Context handoff'
  const endpoint = (agent: AgentIdentity) => {
    const name = handoffAgentName(agent)
    const model = agent.model === 'default' ? undefined : agent.model
    return <span className="inline-flex min-w-0 items-center gap-1" title={[name, model].filter(Boolean).join(' · ')}>
      {agent.kind === 'external'
        ? <AgentIcon id={agent.agent_id} size={13} />
        : <img src="/logo-mark.png" alt="" width={13} height={13} className="dark:invert" />}
      <span>{model && handoff.from.model !== handoff.to.model ? model : name}</span>
    </span>
  }
  const detail = `${handoffAgentName(handoff.from)} → ${handoffAgentName(handoff.to)}`
  return <TimelineDivider label={`${label} · ${detail}`} title={detail}>
    <ArrowLeftRight size={13} aria-hidden="true" />
    <span>{label}</span>
    {endpoint(handoff.from)}<ArrowRight size={12} aria-hidden="true" />{endpoint(handoff.to)}
  </TimelineDivider>
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
