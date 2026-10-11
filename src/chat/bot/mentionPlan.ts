export interface MentionMember {
  botId: string
  name: string
}

export type RoomPost =
  | { mode: 'broadcast' }
  | { mode: 'mention'; ids: string[] }

/** Unaddressed text is one group round. A checkbox or exact @id / @name stays a mention. */
export function roomPost(members: readonly MentionMember[], selected: readonly string[], body: string): RoomPost {
  const ids = mentionPlan(members, selected, body)
  return ids.length === 0 ? { mode: 'broadcast' } : { mode: 'mention', ids }
}

/** Selected members plus explicit @id or @name tokens. Never expands to every member. */
export function mentionPlan(members: readonly MentionMember[], selected: readonly string[], body: string): string[] {
  const byId = new Map(members.map((member) => [member.botId, member]))
  const byName = new Map(members.map((member) => [member.name, member.botId]))
  const ids: string[] = []
  const seen = new Set<string>()
  const add = (botId: string | undefined) => {
    if (!botId || seen.has(botId) || !byId.has(botId)) return
    seen.add(botId)
    ids.push(botId)
  }
  for (const botId of selected) add(botId)
  for (const match of body.matchAll(/@([^\s@]+)/g)) add(byId.has(match[1]) ? match[1] : byName.get(match[1]))
  return ids
}

/** Sends in order and stops at the first rejection so an accepted recipient is not sent again. */
export async function sendMentions(ids: readonly string[], send: (botId: string) => Promise<unknown>): Promise<{ sent: string[]; error: unknown | null }> {
  const sent: string[] = []
  for (const id of ids) {
    try {
      await send(id)
      sent.push(id)
    } catch (error) {
      return { sent, error }
    }
  }
  return { sent, error: null }
}
