import { chatApi } from './api'
import type { CliImportResult } from './types'
import { createWindowStore } from '../utils/windowStore'

type CliImportItem = { agentId: string; sessionId: string }

type CliImportOperation = {
  phase: 'idle' | 'running' | 'success' | 'error'
  items: CliImportItem[]
  error: string
  importedIds: string[]
  failures: CliImportResult['failures']
  settledGeneration: number
  notifiedGeneration: number
  refreshedGeneration: number
}

type CliImportStoreState = {
  byProject: Record<string, CliImportOperation>
}

function emptyCliImportOperation(): CliImportOperation {
  return {
    phase: 'idle',
    items: [],
    error: '',
    importedIds: [],
    failures: [],
    settledGeneration: 0,
    notifiedGeneration: 0,
    refreshedGeneration: 0,
  }
}

export function cliImportItemKey(item: CliImportItem): string {
  return `${item.agentId}::${item.sessionId}`
}

export const cliImportStore = createWindowStore<CliImportStoreState>({ byProject: {} })

let flightEpoch = 0

function dedupeItems(items: CliImportItem[]): CliImportItem[] {
  const seen = new Set<string>()
  const next: CliImportItem[] = []
  for (const item of items) {
    const agentId = item.agentId.trim()
    const sessionId = item.sessionId.trim()
    if (!agentId || !sessionId) continue
    const key = cliImportItemKey({ agentId, sessionId })
    if (seen.has(key)) continue
    seen.add(key)
    next.push({ agentId, sessionId })
  }
  return next
}

function writeProject(
  state: CliImportStoreState,
  projectId: string,
  operation: CliImportOperation,
): CliImportStoreState {
  return { byProject: { ...state.byProject, [projectId]: operation } }
}

/** CLI import has no backend cancel. Closing the dialog only unmounts the view. */
export function startCliImport(projectId: string, items: CliImportItem[]): Promise<void> {
  const deduped = dedupeItems(items)
  if (!projectId || deduped.length === 0) return Promise.resolve()
  const epoch = flightEpoch
  return cliImportStore.run(`import:${epoch}:${projectId}`, async () => {
    if (flightEpoch !== epoch) return
    cliImportStore.setState((state) => {
      const current = state.byProject[projectId] ?? emptyCliImportOperation()
      return writeProject(state, projectId, {
        ...current,
        phase: 'running',
        items: deduped,
        error: '',
        importedIds: [],
        failures: [],
      })
    })
    try {
      const result = await chatApi.importCliSessions(projectId, deduped)
      if (flightEpoch !== epoch) return
      const importedIds = (result.imported ?? []).map((item) => item.conversationId)
      const failures = result.failures ?? []
      cliImportStore.setState((state) => {
        const current = state.byProject[projectId] ?? emptyCliImportOperation()
        return writeProject(state, projectId, {
          ...current,
          phase: failures.length > 0 ? 'error' : 'success',
          items: deduped,
          error: '',
          importedIds,
          failures,
          settledGeneration: current.settledGeneration + 1,
        })
      })
    } catch (err) {
      if (flightEpoch !== epoch) return
      cliImportStore.setState((state) => {
        const current = state.byProject[projectId] ?? emptyCliImportOperation()
        return writeProject(state, projectId, {
          ...current,
          phase: 'error',
          items: deduped,
          error: err instanceof Error ? err.message : String(err),
          importedIds: [],
          failures: [],
          settledGeneration: current.settledGeneration + 1,
        })
      })
    }
  })
}

export function markCliImportNotified(projectId: string, generation: number) {
  cliImportStore.setState((state) => {
    const current = state.byProject[projectId]
    if (!current || current.phase === 'running' || current.settledGeneration !== generation) return state
    if (current.notifiedGeneration === generation) return state
    return writeProject(state, projectId, { ...current, notifiedGeneration: generation })
  })
}

export function markCliImportRefreshed(projectId: string, generation: number) {
  cliImportStore.setState((state) => {
    const current = state.byProject[projectId]
    if (!current || current.phase === 'running' || current.settledGeneration !== generation) return state
    if (current.refreshedGeneration === generation) return state
    return writeProject(state, projectId, { ...current, refreshedGeneration: generation })
  })
}

export function finishCliImportSuccess(projectId: string, generation: number) {
  cliImportStore.setState((state) => {
    const current = state.byProject[projectId]
    if (!current || current.phase === 'running' || current.settledGeneration !== generation) return state
    const byProject = { ...state.byProject }
    delete byProject[projectId]
    return { byProject }
  })
}

export function resetCliImportStoreForTests() {
  flightEpoch += 1
  cliImportStore.setState({ byProject: {} })
}
