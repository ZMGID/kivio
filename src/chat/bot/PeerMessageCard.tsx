import { ChatAttachments } from '../ChatAttachments'
import type { Attachment } from '../types'
import type { InboundPeerMessage } from './peerMessage'
import { BotExchangeNotice } from './BotExchangeNotice'
import { useT } from '../../components/i18n'

export function PeerMessageCard({ peer, attachments = [], conversationId }: {
  peer: InboundPeerMessage
  attachments?: Attachment[]
  conversationId?: string | null
}) {
  const t = useT()
  return (
    <div className="w-full min-w-0">
      <BotExchangeNotice
        exchangeId={peer.exchangeId}
        label={t.chatBotExchangeWith}
        name={peer.senderName} icon={peer.icon} color={peer.color}
      />
      {attachments.length > 0 && (
        <ChatAttachments attachments={attachments} conversationId={conversationId} variant="assistant" />
      )}
    </div>
  )
}
