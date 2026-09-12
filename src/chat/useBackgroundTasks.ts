import { useCallback, useSyncExternalStore } from 'react'
import { api, type BackgroundTaskInfo } from '../api/tauri'

const POLL_MS = 2500
const EMPTY: BackgroundTaskInfo[] = []
type Entry = {
  tasks: BackgroundTaskInfo[]
  listeners: Set<() => void>
  pending: boolean
  version: number
  timer?: ReturnType<typeof setTimeout>
  dispose: () => void
}
const entries = new Map<string, Entry>()

function notify(entry: Entry) {
  for (const listener of entry.listeners) listener()
}

function stopTimer(entry: Entry) {
  clearTimeout(entry.timer)
  entry.timer = undefined
}

async function refresh(id: string, entry: Entry) {
  stopTimer(entry)
  if (entry.pending || document.hidden || entry.listeners.size === 0) return
  entry.pending = true
  const version = entry.version
  try {
    const tasks = await api.chatListBackgroundTasks(id)
    if (entries.get(id) === entry && entry.version === version) {
      // An unchanged empty list does not need to rerender either consumer.
      if (tasks.length || entry.tasks.length) {
        entry.tasks = tasks
        notify(entry)
      }
    }
  } catch {
    // Preserve the last known state on a transient IPC failure.
  } finally {
    entry.pending = false
    if (entries.get(id) === entry && entry.listeners.size && !document.hidden) {
      entry.timer = setTimeout(() => void refresh(id, entry), POLL_MS)
    }
  }
}

function subscribe(id: string, listener: () => void): () => void {
  let entry = entries.get(id)
  if (!entry) {
    const created: Entry = {
      tasks: EMPTY, listeners: new Set(), pending: false, version: 0, dispose: () => {},
    }
    const visibilityChanged = () => {
      if (document.hidden) stopTimer(created)
      else void refresh(id, created)
    }
    document.addEventListener('visibilitychange', visibilityChanged)
    created.dispose = () => {
      stopTimer(created)
      document.removeEventListener('visibilitychange', visibilityChanged)
    }
    entries.set(id, created)
    entry = created
  }
  entry.listeners.add(listener)
  if (!entry.pending && !entry.timer) void refresh(id, entry)
  const subscribed = entry
  return () => {
    subscribed.listeners.delete(listener)
    // Let StrictMode's synchronous cleanup/remount reuse the in-flight request.
    queueMicrotask(() => {
      if (!subscribed.listeners.size && entries.get(id) === subscribed) {
        subscribed.dispose()
        entries.delete(id)
      }
    })
  }
}

/** One polling lifecycle per visible conversation, shared by the header and dock. */
export function useBackgroundTasks(conversationId: string | null, active = true) {
  const id = active ? conversationId : null
  const listen = useCallback((listener: () => void) => id ? subscribe(id, listener) : () => {}, [id])
  const snapshot = useCallback(() => id ? entries.get(id)?.tasks ?? EMPTY : EMPTY, [id])
  return useSyncExternalStore(listen, snapshot, () => EMPTY)
}

/** Apply a confirmed stop/clear to every consumer; older reads cannot undo it. */
export function updateBackgroundTasks(id: string, update: (tasks: BackgroundTaskInfo[]) => BackgroundTaskInfo[]) {
  const entry = entries.get(id)
  if (!entry) return
  entry.version += 1
  entry.tasks = update(entry.tasks)
  notify(entry)
}
