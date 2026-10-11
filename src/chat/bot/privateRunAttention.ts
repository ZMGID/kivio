import { isDedicatedMemberConversation } from '../botIdentity'
import type { MemberAttention } from '../runInteractionInbox'

export interface PrivateRunCard {
  botId: string
  name: string
  conversationId: string
  kind: MemberAttention['kind']
}

/** Room-local labels for a member run that is waiting on the person.
 * The chat sync inbox owns the prompt and tool arguments. This list never includes them. */
export function privateRunCards(
  members: readonly { botId: string; name: string; conversationId: string }[],
  attention: readonly MemberAttention[],
): PrivateRunCard[] {
  const byConversation = new Map(members.map((member) => [member.conversationId, member]))
  const cards: PrivateRunCard[] = []
  for (const item of attention) {
    const member = byConversation.get(item.conversationId)
    if (!member || !isDedicatedMemberConversation(member.conversationId)) continue
    cards.push({
      botId: member.botId,
      name: member.name,
      conversationId: member.conversationId,
      kind: item.kind,
    })
  }
  return cards
}
