import { BotExchangeNotice } from './BotExchangeNotice'
import { useT } from '../../components/i18n'
import type { ToolCallRecord } from '../types'
import type { CollaborationToolName, DeliveryStatus, RoomStatus } from './collaboration'
import { useBotCollaboration } from './collaborationStore'
import { collaborationCard, deliveryCardState } from './eventCard'
import { deliveryStatusLabel, roomStatusLabel } from './presence'

function title(tool: CollaborationToolName, name: string | undefined, t: ReturnType<typeof useT>) {
  const who = name ? ` · ${name}` : ''
  switch (tool) {
    case 'bot_peer_send': return t.chatBotMessageSent
    case 'bot_peer_reply': return `${t.chatBotKindReply}${who}`
    case 'bot_room_mention': return `${t.chatBotKindMention}${who}`
    case 'bot_room_broadcast': return `${t.chatBotBroadcast}${who}`
    case 'bot_room_handoff': return `${t.chatBotKindHandoff}${who}`
    case 'bot_exchange_query': return t.chatBotOpenExchange
    case 'bot_room_members': return t.chatBotPendingAdd
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
  const live = deliveryCardState(card, snapshot.deliveries)
  const status = statusText(live.status, live.pending, t)
  const error = live.error
  return (
    <BotExchangeNotice
      exchangeId={card.exchangeId}
      label={title(card.tool, card.tool === 'bot_peer_send' ? undefined : card.name, t)}
      name={card.tool === 'bot_peer_send' ? card.name : undefined}
      status={status}
      error={error}
    />
  )
}
