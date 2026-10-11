import type { DeliveryKind } from '../../generated/botCollaboration'
import type { ChatMessage } from '../types'

/** Resolved colleague delivery. Null unless the backend peer contract is complete. */
export type InboundPeerMessage = {
  exchangeId: string
  senderName: string
  kind: DeliveryKind
}

export function peerMessageOf(message: ChatMessage): InboundPeerMessage | null {
  const peer = message.peer_message
  if (!peer) return null
  return {
    exchangeId: peer.exchangeId,
    senderName: peer.from.kind === 'bot' ? peer.from.name : '你',
    kind: peer.kind,
  }
}
