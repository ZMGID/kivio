import type { DeliveryKind } from '../../generated/botCollaboration'
import type { ChatMessage } from '../types'

/** Resolved colleague delivery. Null unless the backend peer contract is complete. */
export type InboundPeerMessage = {
  exchangeId: string
  senderName: string
  icon?: string
  color?: string
  kind: DeliveryKind
}

export function peerMessageOf(message: ChatMessage): InboundPeerMessage | null {
  const peer = message.peer_message
  if (!peer) return null
  return {
    exchangeId: peer.exchangeId,
    senderName: peer.from.kind === 'bot' ? peer.from.name : '你',
    icon: peer.from.kind === 'bot' ? peer.from.icon ?? undefined : undefined,
    color: peer.from.kind === 'bot' ? peer.from.color ?? undefined : undefined,
    kind: peer.kind,
  }
}
