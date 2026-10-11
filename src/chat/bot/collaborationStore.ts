import { useEffect, useSyncExternalStore } from 'react'
import {
  collaborationApi,
  decodeSnapshot,
  EMPTY_COLLABORATION,
  type CollaborationSnapshot,
  type DecodeResult,
} from './collaboration'

export interface CollaborationView {
  snapshot: CollaborationSnapshot
  error: string
  revision: number
}

const listeners = new Set<() => void>()
let view: CollaborationView = { snapshot: EMPTY_COLLABORATION, error: '', revision: 0 }
let ticket = 0
let started = false

function emit(next: CollaborationView) {
  view = next
  listeners.forEach((listener) => listener())
}

export function reduceCollaboration(state: CollaborationView, incomingRevision: number, result: DecodeResult): CollaborationView {
  if (incomingRevision < state.revision) return state
  if (!result.ok) return { ...state, error: result.error, revision: incomingRevision }
  return { snapshot: result.snapshot, error: result.snapshot.error ?? '', revision: incomingRevision }
}

export function fetchStillCurrent(fetchTicket: number, currentTicket: number) {
  return fetchTicket === currentTicket
}

function apply(result: DecodeResult) {
  emit(reduceCollaboration(view, view.revision + 1, result))
}

export async function refreshCollaboration() {
  const mine = ++ticket
  try {
    const raw = await collaborationApi.snapshot()
    if (!fetchStillCurrent(mine, ticket)) return
    apply(decodeSnapshot(raw))
  } catch (error) {
    if (!fetchStillCurrent(mine, ticket)) return
    apply({ ok: false, error: error instanceof Error ? error.message : String(error) })
  }
}

export function ensureCollaborationStarted() {
  if (started) return
  started = true
  void collaborationApi.onChanged((payload) => {
    ticket += 1
    apply(decodeSnapshot(payload))
  }).catch((error) => {
    apply({ ok: false, error: error instanceof Error ? error.message : String(error) })
  })
  void refreshCollaboration()
}

function subscribe(listener: () => void) {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

function getSnapshot() {
  return view
}

export function useBotCollaboration() {
  const current = useSyncExternalStore(subscribe, getSnapshot, getSnapshot)
  useEffect(() => { ensureCollaborationStarted() }, [])
  return current
}

export function resetCollaborationForTests() {
  view = { snapshot: EMPTY_COLLABORATION, error: '', revision: 0 }
  ticket = 0
  started = false
  listeners.clear()
}
