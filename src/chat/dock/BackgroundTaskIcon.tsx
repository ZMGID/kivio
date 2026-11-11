import { Bot, TerminalSquare } from 'lucide-react'
import { ModelIcon } from '../../components/ModelIcon'

/** 后台任务图标：Codex 子代理用 OpenAI 图标，其他子代理用机器人，命令用终端。 */
export function BackgroundTaskIcon({ kind, size = 16, strokeWidth, className }: {
  kind: string
  size?: number
  strokeWidth?: number
  className?: string
}) {
  if (kind === 'codex_agent') return <ModelIcon model="openai" size={size} className={className} />
  const Icon = kind === 'local_agent' || kind === 'remote_agent' ? Bot : TerminalSquare
  return <Icon size={size} strokeWidth={strokeWidth} className={className} aria-hidden="true" />
}
