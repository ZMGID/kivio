import { Archive, CircleHelp, Eraser, ListChecks, MessageSquarePlus, Network, Paperclip, Puzzle, Settings, Sparkles, Target, Terminal, Wrench } from 'lucide-react'
import { AgentIcon } from '../components/AgentIcon'
import { SkillIcon } from '../settings/public/icons'
import type { SlashCommandDefinition } from './slashCommands'

export function SlashCommandIcon({ command, size = 16, className }: {
  command: SlashCommandDefinition; size?: number; className?: string
}) {
  if (command.kind === 'skill') {
    return <SkillIcon size={size} className={className} />
  }
  if (command.kind === 'cli' && command.agentId) {
    return <AgentIcon id={command.agentId} size={size} className={className} />
  }
  const icons = {
    help: CircleHelp, plan: ListChecks, goal: Target, orchestrate: Network,
    new: MessageSquarePlus, compact: Archive, clear: Eraser, settings: Settings,
    tools: Wrench, attach: Paperclip,
  }
  const Icon = command.kind === 'action' ? icons[command.id as keyof typeof icons] ?? Sparkles
    : command.slash.includes(':') ? Puzzle : command.kind === 'cli' ? Terminal : Sparkles
  return <Icon size={size} className={className} aria-hidden="true" />
}
