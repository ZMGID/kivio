import { useSyncExternalStore, type SetStateAction } from 'react'

/** State and in-flight commands survive React page unmounts in this window. */
export function createWindowStore<State>(initial: State) {
  let snapshot = initial
  const listeners = new Set<() => void>()
  const flights = new Map<string, Promise<void>>()
  const getSnapshot = () => snapshot
  const subscribe = (listener: () => void) => {
    listeners.add(listener)
    return () => { listeners.delete(listener) }
  }
  const setState = (update: SetStateAction<State>) => {
    const next = typeof update === 'function'
      ? (update as (previous: State) => State)(snapshot)
      : update
    if (Object.is(next, snapshot)) return
    snapshot = next
    for (const listener of listeners) listener()
  }
  const run = (key: string, command: () => Promise<void>): Promise<void> => {
    const existing = flights.get(key)
    if (existing) return existing
    // Reserve synchronously, including when two callers act before React renders.
    const flight = Promise.resolve().then(command).finally(() => {
      if (flights.get(key) === flight) flights.delete(key)
    })
    flights.set(key, flight)
    return flight
  }
  return { getSnapshot, subscribe, setState, run }
}

export function useWindowStore<State>(store: {
  getSnapshot: () => State
  subscribe: (listener: () => void) => () => void
  setState: (update: SetStateAction<State>) => void
}) {
  return [useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot), store.setState] as const
}
