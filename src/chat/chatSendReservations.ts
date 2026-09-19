/** A send owns its target synchronously, before any conversation creation or
 * draft persistence can await. The blank composer is its own target (`null`). */
export function createChatSendReservations() {
  const owners = new Map<string | null, symbol>()

  const claim = (conversationId: string | null) => {
    if (owners.has(conversationId)) return null
    const token = Symbol('chat-send')
    let ownedId: string | null = conversationId
    let released = false
    owners.set(ownedId, token)

    return {
      bind(nextId: string): boolean {
        if (released) return false
        if (ownedId === nextId) return true
        if (owners.has(nextId)) return false
        owners.delete(ownedId)
        ownedId = nextId
        owners.set(ownedId, token)
        return true
      },
      release(): void {
        if (released) return
        released = true
        if (owners.get(ownedId) === token) owners.delete(ownedId)
      },
    }
  }

  return { claim }
}
