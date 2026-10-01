import { useSyncExternalStore } from 'react'
import { api } from '../../api/tauri'
import type { ScheduledTask, ScheduledTaskRun, ScheduledTasksChangedEvent } from '../../api/scheduledTaskContracts'

interface ScheduledTasksSnapshot {
  tasks: ScheduledTask[] | null
  error: string
  revision: number
  /** Latest observed live run per task; terminal events remove the matching run. */
  liveRuns: Map<string, ScheduledTaskRun>
}

let snapshot: ScheduledTasksSnapshot = { tasks: null, error: '', revision: 0, liveRuns: new Map() }
const subscribers = new Set<() => void>()
let stopListening: (() => void) | undefined
let generation = 0
let started = false
let loading = false
let refreshPending = false

function publish(next: ScheduledTasksSnapshot) {
  snapshot = next
  subscribers.forEach(notify => notify())
}

async function loadTasks(ownerGeneration: number) {
  if (loading) { refreshPending = true; return }
  loading = true
  try {
    const tasks = await api.scheduledTasksList()
    if (ownerGeneration !== generation) return
    const taskIds = new Set(tasks.map(task => task.id))
    const liveRuns = new Map([...snapshot.liveRuns].filter(([taskId]) => taskIds.has(taskId)))
    publish({ ...snapshot, tasks, error: '', liveRuns })
  } catch (error) {
    if (ownerGeneration === generation) publish({ ...snapshot, error: String(error) })
  } finally {
    if (ownerGeneration === generation) {
      loading = false
      if (refreshPending) {
        refreshPending = false
        void loadTasks(ownerGeneration)
      }
    }
  }
}

function refresh() {
  publish({ ...snapshot, revision: snapshot.revision + 1 })
  if (started) void loadTasks(generation)
}

function onChanged({ taskId, run }: ScheduledTasksChangedEvent) {
  const liveRuns = new Map(snapshot.liveRuns)
  if (run?.status === 'queued' || run?.status === 'running') liveRuns.set(taskId, run)
  else if (run && liveRuns.get(taskId)?.id === run.id) liveRuns.delete(taskId)
  publish({ ...snapshot, liveRuns, revision: snapshot.revision + 1 })
  void loadTasks(generation)
}

function subscribe(notify: () => void) {
  subscribers.add(notify)
  if (!started) {
    started = true
    const ownerGeneration = ++generation
    // Register before reading the list so changes during startup are not lost.
    void api.onScheduledTasksChanged(event => {
      if (ownerGeneration === generation) onChanged(event)
    }).then(dispose => {
      if (ownerGeneration !== generation) dispose()
      else stopListening = dispose
    }).catch(error => {
      if (ownerGeneration === generation) publish({ ...snapshot, error: String(error) })
    })
    void loadTasks(ownerGeneration)
  }
  return () => {
    subscribers.delete(notify)
    // React StrictMode's immediate re-subscription shares the same request/listener.
    queueMicrotask(() => {
      if (subscribers.size || !started) return
      started = false
      generation += 1
      stopListening?.()
      stopListening = undefined
      loading = false
      refreshPending = false
      snapshot = { ...snapshot, liveRuns: new Map() }
    })
  }
}

const getSnapshot = () => snapshot

/** Sidebar and Tasks page share one list request and one live-event subscription. */
export function useScheduledTasks() {
  return { ...useSyncExternalStore(subscribe, getSnapshot, getSnapshot), refresh }
}
