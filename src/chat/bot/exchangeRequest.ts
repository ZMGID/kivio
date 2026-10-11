const listeners = new Set<(exchangeId: string) => void>()

export function requestPeerExchange(exchangeId: string) {
  if (!exchangeId) return
  listeners.forEach((listener) => listener(exchangeId))
}

export function subscribePeerExchange(listener: (exchangeId: string) => void) {
  listeners.add(listener)
  return () => { listeners.delete(listener) }
}
