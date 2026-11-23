import { bundledExperts } from './bundledExperts'
import type { ChatAssistant } from './types'

const iconsById = new Map(bundledExperts.map((expert) => [expert.id, expert.icon]))

const sizes = {
  menu: 'size-4 text-[14px]',
  card: 'size-9 rounded-lg text-[22px]',
  detail: 'size-16 rounded-xl text-[38px]',
}

export function AssistantAvatar({ assistant, size }: {
  assistant: Pick<ChatAssistant, 'id' | 'icon' | 'color'>
  size: keyof typeof sizes
}) {
  const configured = assistant.icon?.trim()
  const icon = configured === 'bot' ? '🤖' : configured || iconsById.get(assistant.id) || '🤖'
  const tint = assistant.color || 'var(--accent)'

  return (
    <span
      aria-hidden="true"
      className={`grid shrink-0 place-items-center overflow-hidden leading-none ${sizes[size]}`}
      style={size === 'menu' ? undefined : {
        backgroundColor: `color-mix(in srgb, ${tint} 10%, var(--theme-surface))`,
      }}
    >
      {icon}
    </span>
  )
}
