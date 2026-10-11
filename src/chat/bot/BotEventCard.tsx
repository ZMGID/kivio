import { normalizeToolCallStatus } from '../toolStatus'
import { BotExchangeNotice } from './BotExchangeNotice'
import { useT } from '../../components/i18n'
import type { ToolCallRecord } from '../types'
import type { DeliveryStatus, RoomStatus } from './collaboration'
import { useBotCollaboration } from './collaborationStore'
import { collaborationCard, deliveryCardState, type BotActivityToolName } from './eventCard'
import { deliveryStatusLabel, roomStatusLabel } from './presence'

function title(tool: BotActivityToolName, name: string | undefined, t: ReturnType<typeof useT>) {
  const who = name ? ` · ${name}` : ''
  switch (tool) {
    case 'bot_peer_send': return t.chatBotMessageSent
    case 'bot_peer_reply': return `${t.chatBotKindReply}${who}`
    case 'bot_room_mention': return `${t.chatBotKindMention}${who}`
    case 'bot_room_broadcast': return `${t.chatBotBroadcast}${who}`
    case 'bot_room_handoff': return `${t.chatBotKindHandoff}${who}`
    case 'bot_exchange_query': return t.chatBotOpenExchange
    case 'bot_room_members': return t.chatBotGroupMembers
    case 'bot_create': return t.chatBotActivityCreate
    case 'bot_update': return t.chatBotActivityUpdate
    case 'bot_get': case 'bot_list': return t.chatBotActivityRead
    case 'bot_set_primary': return t.chatBotActivityPrimary
    case 'bot_memory_read': return t.chatBotActivityMemoryRead
    case 'bot_memory_update': return t.chatBotActivityMemoryUpdate
    case 'bot_memory_clear': return t.chatBotActivityMemoryClear
    case 'bot_room_log': return t.chatBotGroups
    case 'bot_room_pause': return t.chatBotPause
    case 'bot_room_continue': return t.chatBotContinue
  }
}

function statusText(status: DeliveryStatus | RoomStatus | undefined, pending: boolean, t: ReturnType<typeof useT>) {
  if (pending) return t.chatBotCardSubmitting
  if (!status) return ''
  if (status === 'active' || status === 'paused' || status === 'stopped') return roomStatusLabel(status, t)
  return deliveryStatusLabel(status, t)
}

export function BotEventCard({ toolCall }: { toolCall: ToolCallRecord }) {
  const t = useT()
  const { snapshot } = useBotCollaboration()
  const card = collaborationCard(toolCall)
  if (!card) return null
  const phase = normalizeToolCallStatus(toolCall.status)
  if (['bot_exchange_query', 'bot_room_log', 'bot_get', 'bot_list', 'bot_memory_read'].includes(card.tool) && !card.error && phase !== 'error' && phase !== 'cancelled' && phase !== 'skipped') return null
  const live = deliveryCardState(card, snapshot.deliveries, snapshot.pending)
  const error = live.error || (phase === 'error' ? t.toolCardFailed : undefined)
  const status = statusText(live.status, live.pending, t) || (error ? t.toolCardFailed : phase === 'completed' ? t.toolCardDone : phase === 'cancelled' ? t.toolCardCancelled : '')
  return (
    <BotExchangeNotice
      exchangeId={card.exchangeId}
      label={title(card.tool, undefined, t)}
      name={card.name}
      status={status}
      error={error}
    />
  )
}
