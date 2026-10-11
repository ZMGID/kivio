import { useSyncExternalStore } from 'react'
import { ChevronDown } from 'lucide-react'
import { useT } from '../components/i18n'
import { useBotCollaboration } from './bot/collaborationStore'
import { contactAvatarMood } from './bot/presence'
import { getSnapshot, subscribeSnapshot } from './streamingStore'
import { resolveBlobMood } from './kivioBlobSim'
import type { ExecutionFeedback } from './chatExecutionOwner'
import type { ChatAssistant } from './types'
import { BotAvatar } from './BotAvatar'
import { botContactId } from './botIdentity'
import type { Conversation } from './types'

/** One contact identity; conversation controls disclose without changing their owners. */
export function BotConversationHeader({ conversation, onEdit, detailsOpen, active = false, attention = false, error = false, executionFeedback, appearance }: {
  conversation: Conversation | null
  onEdit: () => void
  detailsOpen: boolean
  active?: boolean
  attention?: boolean
  error?: boolean
  executionFeedback?: ExecutionFeedback
  appearance?: Pick<ChatAssistant, 'icon' | 'color'>
}) {
  const t = useT()
  const assistant = conversation?.assistant_snapshot ?? conversation?.assistantSnapshot
  const assistantId = botContactId(conversation?.id)
  const { snapshot } = useBotCollaboration()
  const phase = useSyncExternalStore(subscribeSnapshot, () => {
    const stream = getSnapshot()
    if (!active) return 'idle'
    const running = stream.toolCalls.filter(tool => tool.status === 'running')
    if (!stream.streaming || (!stream.reasoningStreaming && !running.length && !stream.content.length)) return 'work'
    return resolveBlobMood({ active, contentLen: stream.content.length, reasoningStreaming: stream.reasoningStreaming,
      runningToolNames: running.map(tool => tool.tool_name || tool.toolName || tool.name || '') })
  })
  const deliveries = snapshot.deliveries.filter(item => item.toBotId === assistantId)
  const baseMood = contactAvatarMood({ generating: active, attention, error, deliveries,
    pending: snapshot.pending.some(item => item.botId === assistantId && item.status === 'awaiting_user') })
  const mood = baseMood === 'work' && active ? phase : baseMood
  const latestSuccess = [...deliveries].filter(delivery => delivery.status === 'succeeded').sort((a, b) => b.updatedAt - a.updatedAt || b.id.localeCompare(a.id))[0]
  const privateSuccess = executionFeedback?.status === 'completed' ? executionFeedback.key : ''
  const collaborationSuccess = latestSuccess?.id ?? ''
  const successKey = privateSuccess || collaborationSuccess ? `${privateSuccess}|${collaborationSuccess}` : undefined
  const statusLabel = t.chatBotAvatarMoods.split('|')[({ idle: 0, think: 1, search: 2, work: 3, speak: 4, wait: 5, error: 6, done: 7 })[mood]]
  if (!assistantId || !assistant) return null
  return <div className="relative flex min-w-0 flex-1 items-center justify-center" data-tauri-drag-region>
    <button type="button" className="flex max-w-full items-center gap-2 rounded-lg px-2 py-1 text-[14px] font-semibold text-neutral-900 transition-colors hover:bg-neutral-900/[0.04]"
      title={`${assistant.name} · ${t.chatBotDetails}`} aria-expanded={detailsOpen} aria-controls="bot-settings-sidebar" onClick={onEdit} data-tauri-drag-region="false">
      <BotAvatar key={conversation?.id} icon={appearance?.icon} color={appearance?.color} size={32} mood={mood} successKey={successKey} label={statusLabel} /><span className="truncate">{assistant.name}</span><ChevronDown size={12} aria-hidden="true" className="shrink-0 text-neutral-500" /><span className="sr-only">{t.chatBotDetails}</span>
    </button>
  </div>
}
