import { save } from '@tauri-apps/plugin-dialog'
import { chatApi } from './api'
import { conversationMarkdownFilename } from './conversationExport'
import { createWindowStore } from '../utils/windowStore'

/**
 * Library mutations keep running after SessionCenter unmounts.
 * Archive, delete, pin, move, and export have no backend cancel.
 * One flight owns the window: a second start joins it and does not mutate again.
 * A settled flight writes only if it is still the current mutation.
 */
export type SessionExportTarget = { id: string; title: string }

export type SessionBatchAction =
  | { kind: 'archive'; archived: boolean; ids: string[] }
  | { kind: 'archive-one'; archived: boolean; ids: string[] }
  | { kind: 'delete'; ids: string[] }
  | { kind: 'delete-one'; ids: string[] }
  | { kind: 'pin'; pinned: boolean; ids: string[] }
  | { kind: 'move-project'; projectId: string | null; ids: string[] }
  | { kind: 'move-set'; setId: string | null; ids: string[] }
  | { kind: 'export'; ids: string[]; targets: SessionExportTarget[]; lang: 'zh' | 'en' }

type SessionBatchState = {
  phase: 'idle' | 'running' | 'success' | 'error'
  action: SessionBatchAction | null
  error: string
  warnings: string[]
  settledGeneration: number
  notifiedGeneration: number
  refreshedGeneration: number
  /** Bumped for each accepted flight. Late writes from an older flight no-op. */
  mutation: number
}

const EMPTY_SESSION_BATCH: SessionBatchState = {
  phase: 'idle',
  action: null,
  error: '',
  warnings: [],
  settledGeneration: 0,
  notifiedGeneration: 0,
  refreshedGeneration: 0,
  mutation: 0,
}

export const sessionBatchStore = createWindowStore(EMPTY_SESSION_BATCH)

let flightEpoch = 0
let activeFlight: Promise<void> | null = null

function flightKey() {
  return `sessions:batch:${flightEpoch}`
}

function dedupedIds(ids: string[]): string[] {
  return [...new Set(ids.filter((id) => id.trim() !== ''))]
}

function errorMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err)
}

function normalize(action: SessionBatchAction): SessionBatchAction | null {
  if (action.kind === 'archive-one' || action.kind === 'delete-one') {
    const ids = dedupedIds(action.ids).slice(0, 1)
    if (ids.length === 0) return null
    return action.kind === 'archive-one'
      ? { kind: 'archive-one', archived: action.archived, ids }
      : { kind: 'delete-one', ids }
  }
  const ids = dedupedIds(action.ids)
  if (ids.length === 0) return null
  switch (action.kind) {
    case 'archive':
      return { kind: 'archive', archived: action.archived, ids }
    case 'delete':
      return { kind: 'delete', ids }
    case 'pin':
      return { kind: 'pin', pinned: action.pinned, ids }
    case 'move-project':
      return { kind: 'move-project', projectId: action.projectId, ids }
    case 'move-set':
      return { kind: 'move-set', setId: action.setId, ids }
    case 'export': {
      const targets = ids.map((id) => {
        const title = action.targets.find((target) => target.id === id)?.title.trim() || id
        return { id, title }
      })
      return { kind: 'export', ids, targets, lang: action.lang }
    }
    default:
      return null
  }
}

function canWrite(epoch: number, token: number): boolean {
  return flightEpoch === epoch && sessionBatchStore.getSnapshot().mutation === token
}

function writeSettled(
  epoch: number,
  token: number,
  patch: { phase: 'success' | 'error'; action: SessionBatchAction; error: string; warnings: string[] },
) {
  if (!canWrite(epoch, token)) return
  sessionBatchStore.setState((state) => {
    if (state.mutation !== token) return state
    return {
      ...state,
      phase: patch.phase,
      action: patch.action,
      error: patch.error,
      warnings: patch.warnings,
      settledGeneration: state.settledGeneration + 1,
    }
  })
}

/** All dialogs cancelled: stop the spinner without pretending a library change happened. */
function writeIdle(epoch: number, token: number) {
  if (!canWrite(epoch, token)) return
  sessionBatchStore.setState((state) => {
    if (state.mutation !== token) return state
    return { ...state, phase: 'idle', action: null, error: '', warnings: [] }
  })
}

async function performExport(
  action: Extract<SessionBatchAction, { kind: 'export' }>,
  epoch: number,
  token: number,
) {
  const exported: string[] = []
  let started = false
  try {
    for (const target of action.targets) {
      if (!canWrite(epoch, token)) return
      const path = await save({
        defaultPath: conversationMarkdownFilename(target.title),
        filters: [{ name: 'Markdown', extensions: ['md'] }],
      })
      if (!path) continue
      started = true
      if (!canWrite(epoch, token)) return
      await chatApi.exportConversationMarkdown(target.id, path, action.lang)
      exported.push(path)
    }
  } catch (err) {
    writeSettled(epoch, token, {
      phase: 'error',
      action,
      error: errorMessage(err),
      warnings: exported,
    })
    return
  }
  if (!canWrite(epoch, token)) return
  if (!started) {
    writeIdle(epoch, token)
    return
  }
  writeSettled(epoch, token, { phase: 'success', action, error: '', warnings: [] })
}

async function perform(action: SessionBatchAction, epoch: number, token: number): Promise<void> {
  try {
    if (action.kind === 'export') {
      await performExport(action, epoch, token)
      return
    }
    if (action.kind === 'archive') {
      await chatApi.bulkUpdateConversations(action.ids, { archived: action.archived })
    } else if (action.kind === 'archive-one') {
      const id = action.ids[0]
      if (!id) return
      await chatApi.updateConversation(id, { archived: action.archived })
    } else if (action.kind === 'pin') {
      await chatApi.bulkUpdateConversations(action.ids, { pinned: action.pinned })
    } else if (action.kind === 'move-project') {
      await chatApi.bulkUpdateConversations(action.ids, { projectId: action.projectId })
    } else if (action.kind === 'move-set') {
      await chatApi.bulkUpdateConversations(action.ids, { setId: action.setId })
    } else if (action.kind === 'delete') {
      const result = await chatApi.bulkDeleteConversations(action.ids)
      if (!canWrite(epoch, token)) return
      writeSettled(epoch, token, {
        phase: 'success',
        action,
        error: '',
        warnings: result.warnings ?? [],
      })
      return
    } else if (action.kind === 'delete-one') {
      const id = action.ids[0]
      if (!id) return
      const warnings = await chatApi.deleteConversation(id)
      if (!canWrite(epoch, token)) return
      writeSettled(epoch, token, {
        phase: 'success',
        action,
        error: '',
        warnings: warnings ?? [],
      })
      return
    } else {
      return
    }
    if (!canWrite(epoch, token)) return
    writeSettled(epoch, token, { phase: 'success', action, error: '', warnings: [] })
  } catch (err) {
    writeSettled(epoch, token, {
      phase: 'error',
      action,
      error: errorMessage(err),
      warnings: [],
    })
  }
}

export function startSessionBatch(action: SessionBatchAction): Promise<void> {
  if (activeFlight) return activeFlight
  const next = normalize(action)
  if (!next) return Promise.resolve()
  const epoch = flightEpoch
  const token = sessionBatchStore.getSnapshot().mutation + 1
  sessionBatchStore.setState((state) => ({
    ...state,
    phase: 'running',
    action: next,
    error: '',
    warnings: [],
    mutation: token,
  }))
  const flight = sessionBatchStore.run(flightKey(), async () => {
    try {
      if (flightEpoch !== epoch || sessionBatchStore.getSnapshot().mutation !== token) return
      await perform(next, epoch, token)
    } finally {
      if (activeFlight === flight) activeFlight = null
    }
  })
  activeFlight = flight
  return flight
}

export function markSessionBatchNotified(generation: number) {
  sessionBatchStore.setState((state) => {
    if (state.phase === 'running' || state.settledGeneration !== generation) return state
    if (state.notifiedGeneration === generation) return state
    return { ...state, notifiedGeneration: generation }
  })
}

export function markSessionBatchRefreshed(generation: number) {
  sessionBatchStore.setState((state) => {
    if (state.phase === 'running' || state.settledGeneration !== generation) return state
    if (state.phase === 'success') {
      return {
        ...state,
        phase: 'idle',
        action: null,
        error: '',
        warnings: [],
        notifiedGeneration: Math.max(state.notifiedGeneration, generation),
        refreshedGeneration: generation,
      }
    }
    if (state.refreshedGeneration === generation) return state
    return { ...state, refreshedGeneration: generation }
  })
}

/** Drops a leaked flight so a later test can start a new batch. The backend call is not cancelled. */
export function resetSessionBatchStoreForTests() {
  flightEpoch += 1
  activeFlight = null
  sessionBatchStore.setState(EMPTY_SESSION_BATCH)
}
