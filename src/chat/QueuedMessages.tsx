import { ArrowUp, Clock3, Pencil, X } from 'lucide-react'
import { IconButton } from '../components/Button'
import { MessageBubble } from './MessageBubble'
import { isQueuedSubmitted, type QueuedMessage } from './hooks/useMessageQueue'
import { i18n, type Lang } from '../components/i18n'

interface QueuedMessagesProps {
  messages: QueuedMessage[]
  conversationId?: string | null
  canInsert?: boolean
  onInsert?: (messageId: string) => void
  onRemove?: (messageId: string) => void
  onRestore?: (messageId: string) => void
  lang: Lang
}

/** Pending messages share the transcript's user bubble; promotion is an explicit action. */
export function QueuedMessages({
  messages, conversationId, canInsert = true, onInsert, onRemove, onRestore, lang,
}: QueuedMessagesProps) {
  if (messages.length === 0) return null
  const t = i18n[lang]
  return (
    <div className="not-prose w-full">
      {messages.map(message => {
        const submitted = isQueuedSubmitted(message)
        return (
          <div key={message.id} data-chat-message-list-item="injected-user">
            <MessageBubble conversationId={conversationId} message={{
              id: message.id, role: 'user', content: message.content, timestamp: 0,
              attachments: message.attachments,
            }} />
            {!submitted && (
              <div className="flex flex-wrap items-center justify-end gap-1 pb-2 text-xs text-neutral-500">
                <Clock3 size={12} aria-hidden="true" />
                <span role="status">{message.steerRejected ? t.chatSteerRejected : message.followUpRejected ? t.chatFollowUpRejected : t.chatQueuedWaiting}</span>
                {onInsert && message.kind !== 'compact' && (
                  <IconButton size="xs" variant="ghost" label={t.chatQueueInsert}
                    title={canInsert ? t.chatQueueInsert : t.chatQueueInsertUnavailable}
                    disabled={!canInsert} onClick={() => onInsert(message.id)}>
                    <ArrowUp size={13} />
                  </IconButton>
                )}
                {onRestore && (
                  <IconButton size="xs" variant="ghost" label={t.chatQueueRestore}
                    onClick={() => onRestore(message.id)}><Pencil size={13} /></IconButton>
                )}
                {onRemove && (
                  <IconButton size="xs" variant="ghost" label={t.chatQueueRemove}
                    onClick={() => onRemove(message.id)}><X size={13} /></IconButton>
                )}
              </div>
            )}
          </div>
        )
      })}
    </div>
  )
}
