import { onExternalCliInstallLog, type ExternalCliInstallLog } from '../api/externalCliSettings'

export type CliInstallResult = 'ok' | 'fail' | null

export type CliInstallJob = { log: string[] } & (
  | { running: true; planId: string; phase: Exclude<ExternalCliInstallLog['kind'], 'output'>; result: null }
  | { running: false; result: CliInstallResult }
)

const EMPTY_JOB: CliInstallJob = { running: false, log: [], result: null }

const jobs = new Map<string, CliInstallJob>()
const inFlight = new Set<string>()
const listeners = new Set<() => void>()

function notify() {
  for (const listener of listeners) listener()
}

function snapshot(agentId: string): CliInstallJob {
  return jobs.get(agentId) ?? EMPTY_JOB
}

function write(agentId: string, next: CliInstallJob) {
  if (!next.running && next.result === null && next.log.length === 0) {
    jobs.delete(agentId)
  } else {
    jobs.set(agentId, next)
  }
  notify()
}

export function getCliInstallJob(agentId: string): CliInstallJob {
  return snapshot(agentId)
}

export function subscribeCliInstallJobs(listener: () => void): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

export function clearCliInstallJob(agentId: string) {
  if (!jobs.has(agentId)) return
  jobs.delete(agentId)
  notify()
}

/** Tests only: drop in-flight jobs so cases do not leak into each other. */
export function resetCliInstallJobsForTests() {
  jobs.clear()
  inFlight.clear()
}

/**
 * Run an install/update outside any React tree. AgentDetail unmounts when the
 * user switches CLIs; the job and its log listener must not.
 */
export async function startCliInstall(
  agentId: string,
  planId: string,
  hooks: {
    install: (id: string, planId: string) => Promise<void | 'cancelled'>
    afterDone?: (id: string, cancelled: boolean) => Promise<void>
  },
): Promise<void> {
  if (inFlight.has(agentId)) return
  inFlight.add(agentId)
  write(agentId, { running: true, planId, phase: 'running', log: [], result: null })
  let unlisten: (() => void) | undefined
  let cancelled = false
  try {
    unlisten = await onExternalCliInstallLog((event) => {
      if (event.agentId !== agentId || event.planId !== planId) return
      const current = snapshot(agentId)
      if (!current.running || current.planId !== planId) return
      write(agentId, event.kind === 'output'
        ? { ...current, log: [...current.log, event.line] }
        : { ...current, phase: event.kind })
    })
    const outcome = await hooks.install(agentId, planId)
    cancelled = outcome === 'cancelled'
    write(agentId, cancelled ? EMPTY_JOB : { log: snapshot(agentId).log, running: false, result: 'ok' })
  } catch (err) {
    const current = snapshot(agentId)
    write(agentId, {
      running: false,
      result: 'fail',
      log: [...current.log, String(err)],
    })
  } finally {
    unlisten?.()
    inFlight.delete(agentId)
    try {
      await hooks.afterDone?.(agentId, cancelled)
    } catch {
      // Refresh failures must not hide the install outcome already on screen.
    }
  }
}
