import type { Automation } from '../../api/automationContracts'
import { isTauriRuntime } from '../../api/tauri'
import { createWindowStore } from '../../utils/windowStore'
import { automationApi } from './api'

export const AUTOMATION_SAVE_DEBOUNCE_MS = 400

export type AutomationSaveStatus = 'saved' | 'pending' | 'saving' | 'error'

/** Open automation draft and its ordered save. Run/export state stays elsewhere. */
export interface AutomationDraftState {
  draft: Automation | null
  dirty: boolean
  status: AutomationSaveStatus
  error: string
  session: number
  /** Echo of our own successful write, so a remote event can be ignored. */
  lastSelfUpdatedAt: string
  /** Bumped after a successful write so a mounted page can refresh its list. */
  savedEpoch: number
}

const EMPTY: AutomationDraftState = {
  draft: null,
  dirty: false,
  status: 'saved',
  error: '',
  session: 0,
  lastSelfUpdatedAt: '',
  savedEpoch: 0,
}

export const automationDraftStore = createWindowStore<AutomationDraftState>(EMPTY)

let saveTimer: ReturnType<typeof setTimeout> | null = null
const retiredIds = new Set<string>()

function clearSaveTimer() {
  if (saveTimer == null) return
  clearTimeout(saveTimer)
  saveTimer = null
}

export function resetAutomationDraftStore() {
  clearSaveTimer()
  retiredIds.clear()
  automationDraftStore.setState((prev) => ({ ...EMPTY, session: prev.session + 1, savedEpoch: prev.savedEpoch }))
}

function live(id: string, session: number) {
  const state = automationDraftStore.getSnapshot()
  return state.session === session && state.draft?.id === id && !retiredIds.has(id)
}

export function showAutomation(automation: Automation) {
  clearSaveTimer()
  retiredIds.delete(automation.id)
  automationDraftStore.setState((prev) => ({
    draft: automation,
    dirty: false,
    status: 'saved',
    error: '',
    session: prev.session + 1,
    lastSelfUpdatedAt: automation.updatedAt,
    savedEpoch: prev.savedEpoch,
  }))
}

export function discardAutomationDraft() {
  clearSaveTimer()
  automationDraftStore.setState((prev) => ({
    ...EMPTY,
    session: prev.session + 1,
    savedEpoch: prev.savedEpoch,
  }))
}

/** Server already removed this automation. Hide its draft and drop late writes. */
export function forgetAutomationDraft(id: string) {
  retiredIds.add(id)
  if (automationDraftStore.getSnapshot().draft?.id === id) discardAutomationDraft()
}

/** Explicit delete. A failed remove keeps the draft editable. */
export async function deleteAutomationDraft(id: string, remove: () => Promise<void>) {
  retiredIds.add(id)
  try {
    await automationDraftStore.run(`automation:${id}`, async () => {})
    await remove()
    retiredIds.delete(id)
    if (automationDraftStore.getSnapshot().draft?.id === id) discardAutomationDraft()
  } catch (error) {
    retiredIds.delete(id)
    throw error
  }
}

export function stageAutomationDraft(next: Automation) {
  const current = automationDraftStore.getSnapshot()
  if (retiredIds.has(next.id)) return
  if (current.draft && current.draft.id !== next.id) return
  const session = current.draft?.id === next.id ? current.session : current.session
  automationDraftStore.setState((prev) => {
    if (prev.session !== session) return prev
    if (prev.draft && prev.draft.id !== next.id) return prev
    return {
      ...prev,
      draft: next,
      dirty: isTauriRuntime() ? true : prev.dirty,
      status: !isTauriRuntime() ? prev.status : prev.status === 'saving' ? 'saving' : 'pending',
      session: prev.draft ? prev.session : prev.session,
    }
  })
  if (!isTauriRuntime()) return
  clearSaveTimer()
  saveTimer = setTimeout(() => {
    saveTimer = null
    void flushAutomationDraft().catch(() => {})
  }, AUTOMATION_SAVE_DEBOUNCE_MS)
}

async function writeAutomationSession(id: string, session: number) {
  while (live(id, session) && isTauriRuntime()) {
    const snap = automationDraftStore.getSnapshot()
    if (!snap.dirty || !snap.draft) {
      automationDraftStore.setState((prev) => {
        if (!live(id, session) || prev.dirty) return prev
        if (prev.status === 'saved' && prev.error === '') return prev
        return { ...prev, status: 'saved', error: '' }
      })
      if (!live(id, session) || automationDraftStore.getSnapshot().dirty) continue
      return
    }
    const sent = snap.draft
    automationDraftStore.setState((prev) => (
      live(id, session) ? { ...prev, status: 'saving', error: '' } : prev
    ))
    if (!live(id, session)) return
    try {
      const saved = await automationApi.save(sent)
      if (!live(id, session)) return
      automationDraftStore.setState((prev) => {
        if (!live(id, session) || !prev.draft) return prev
        if (prev.draft !== sent) {
          return { ...prev, lastSelfUpdatedAt: saved.updatedAt, status: 'saving' }
        }
        return {
          ...prev,
          draft: saved,
          dirty: false,
          status: 'saved',
          error: '',
          lastSelfUpdatedAt: saved.updatedAt,
          savedEpoch: prev.savedEpoch + 1,
        }
      })
      if (automationDraftStore.getSnapshot().draft === saved && !automationDraftStore.getSnapshot().dirty) return
    } catch (err) {
      if (!live(id, session)) return
      const message = err instanceof Error ? err.message : String(err)
      automationDraftStore.setState((prev) => (
        live(id, session) ? { ...prev, status: 'error', error: message } : prev
      ))
      throw err
    }
  }
}

async function drainAutomation(id: string, session: number) {
  if (!isTauriRuntime()) return
  for (;;) {
    try {
      await automationDraftStore.run(`automation:${id}`, () => writeAutomationSession(id, session))
    } catch (error) {
      const after = automationDraftStore.getSnapshot()
      if (after.session !== session || after.draft?.id !== id || retiredIds.has(id)) return
      throw error
    }
    const after = automationDraftStore.getSnapshot()
    if (after.session !== session || after.draft?.id !== id || retiredIds.has(id)) return
    if (after.status === 'error') throw new Error(after.error || 'save failed')
    if (!after.dirty) return
  }
}

/** Autosave, run, export, back, and page-leave all wait on this ordered writer. */
export function flushAutomationDraft(): Promise<void> {
  clearSaveTimer()
  const start = automationDraftStore.getSnapshot()
  const id = start.draft?.id
  if (!id) return Promise.resolve()
  if (retiredIds.has(id)) return Promise.resolve()
  if (!start.dirty && start.status !== 'saving') return Promise.resolve()
  return drainAutomation(id, start.session)
}
