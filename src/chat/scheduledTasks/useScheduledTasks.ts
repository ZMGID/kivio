import { useSyncExternalStore } from 'react'
import { api } from '../../api/tauri'
import type { ScheduledTask, ScheduledTaskRun, ScheduledTasksChangedEvent } from '../../api/scheduledTaskContracts'

interface ScheduledTasksView {
  tasks: ScheduledTask[] | null
  error: string
  revision: number
  /** One live run per task. A queued run wins; otherwise the newest createdAt. */
  liveRuns: Map<string, ScheduledTaskRun>
}

let view: ScheduledTasksView = { tasks: null, error: '', revision: 0, liveRuns: new Map() }
/** Every queued/running run, so a current run and the next queued run can coexist. */
let runsByTask = new Map<string, ScheduledTaskRun[]>()
const subscribers = new Set<() => void>()
let stopListening: (() => void) | undefined
let generation = 0
let started = false
let loading = false
let refreshPending = false
/** Events and manual refresh move this. A snapshot issued earlier must not land. */
let mutation = 0
let booting: Promise<void> | null = null
let bootGeneration = -1

function publish(next: ScheduledTasksView) {
  view = next
  subscribers.forEach(notify => notify())
}

function isLive(status: ScheduledTaskRun['status']) {
  return status === 'queued' || status === 'running'
}

function beats(run: ScheduledTaskRun, chosen: ScheduledTaskRun) {
  const runQueued = run.status === 'queued'
  const chosenQueued = chosen.status === 'queued'
  if (runQueued !== chosenQueued) return runQueued
  return run.createdAt >= chosen.createdAt
}

function representative(runs: readonly ScheduledTaskRun[]) {
  let chosen: ScheduledTaskRun | undefined
  for (const run of runs) {
    if (!isLive(run.status)) continue
    if (!chosen || beats(run, chosen)) chosen = run
  }
  return chosen
}

function visibleRuns(tasks: readonly ScheduledTask[] | null) {
  const ids = tasks ? new Set(tasks.map(task => task.id)) : null
  const liveRuns = new Map<string, ScheduledTaskRun>()
  for (const [taskId, runs] of runsByTask) {
    if (ids && !ids.has(taskId)) continue
    const chosen = representative(runs)
    if (chosen) liveRuns.set(taskId, chosen)
  }
  return liveRuns
}

function remember(run: ScheduledTaskRun) {
  const next = (runsByTask.get(run.taskId) ?? []).filter(item => item.id !== run.id)
  if (isLive(run.status)) next.push(run)
  if (next.length) runsByTask.set(run.taskId, next)
  else runsByTask.delete(run.taskId)
}

function replaceFromSnapshot(tasks: readonly ScheduledTask[], runs: readonly ScheduledTaskRun[]) {
  const ids = new Set(tasks.map(task => task.id))
  const next = new Map<string, ScheduledTaskRun[]>()
  for (const run of runs) {
    if (!ids.has(run.taskId) || !isLive(run.status)) continue
    const list = next.get(run.taskId)
    if (list) {
      const index = list.findIndex(item => item.id === run.id)
      if (index >= 0) list[index] = run
      else list.push(run)
    } else next.set(run.taskId, [run])
  }
  runsByTask = next
}

async function loadTasks(ownerGeneration: number) {
  if (ownerGeneration !== generation || !stopListening || subscribers.size === 0) return
  if (loading) {
    refreshPending = true
    return
  }
  loading = true
  const issuedAt = mutation
  try {
    const payload = await api.scheduledTasksSnapshot()
    if (ownerGeneration !== generation || issuedAt !== mutation || subscribers.size === 0) return
    replaceFromSnapshot(payload.tasks, payload.liveRuns)
    publish({ tasks: payload.tasks, error: '', revision: view.revision, liveRuns: visibleRuns(payload.tasks) })
  } catch (error) {
    if (ownerGeneration === generation && issuedAt === mutation && subscribers.size > 0) {
      publish({ ...view, error: String(error) })
    }
  } finally {
    if (ownerGeneration === generation) {
      loading = false
      if (refreshPending && subscribers.size > 0) {
        refreshPending = false
        void loadTasks(ownerGeneration)
      } else refreshPending = false
    }
  }
}

function refresh() {
  mutation += 1
  publish({ ...view, revision: view.revision + 1 })
  if (!started) return
  if (!stopListening) {
    void boot(generation)
    return
  }
  void loadTasks(generation)
}

function onChanged({ run }: ScheduledTasksChangedEvent) {
  mutation += 1
  // Save, enable, and delete emit `run: null` after queued runs that were not
  // admitted are dropped. Deleting history uses the same payload. Leave the
  // local runs in place: the following snapshot removes them, and this bump
  // stops a snapshot issued earlier from putting a deleted task or queue back.
  if (run) remember(run)
  publish({ ...view, liveRuns: visibleRuns(view.tasks), revision: view.revision + 1 })
  void loadTasks(generation)
}

function boot(ownerGeneration: number) {
  if (booting && bootGeneration === ownerGeneration) return booting
  bootGeneration = ownerGeneration
  const pending = Promise.resolve().then(async () => {
    try {
      const dispose = await api.onScheduledTasksChanged(event => {
        if (ownerGeneration === generation) onChanged(event)
      })
      if (ownerGeneration !== generation) {
        dispose()
        return
      }
      stopListening = dispose
      await loadTasks(ownerGeneration)
    } catch (error) {
      if (ownerGeneration === generation) publish({ ...view, error: String(error) })
    } finally {
      if (booting === pending) booting = null
    }
  })
  booting = pending
  return pending
}

function subscribe(notify: () => void) {
  subscribers.add(notify)
  if (!started) {
    started = true
    const ownerGeneration = ++generation
    void boot(ownerGeneration)
  }
  return () => {
    subscribers.delete(notify)
    // React StrictMode unsubscribes and resubscribes synchronously. Tear down
    // only if that second subscribe did not arrive before this microtask.
    queueMicrotask(() => {
      if (subscribers.size || !started) return
      started = false
      generation += 1
      mutation += 1
      bootGeneration = -1
      loading = false
      refreshPending = false
      runsByTask = new Map()
      view = { ...view, liveRuns: new Map() }
      const dispose = stopListening
      stopListening = undefined
      dispose?.()
    })
  }
}

const getSnapshot = () => view

/** Sidebar and Tasks page share one snapshot read and one live-event subscription. */
export function useScheduledTasks() {
  return { ...useSyncExternalStore(subscribe, getSnapshot, getSnapshot), refresh }
}
