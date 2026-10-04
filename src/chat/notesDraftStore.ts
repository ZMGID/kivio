import { api, type Note } from '../api/tauri'
import { createWindowStore } from '../utils/windowStore'

/** Debounce for typing. Leaving the page flushes immediately and does not wait this out. */
export const NOTE_SAVE_DEBOUNCE_MS = 800

export type NoteSaveStatus = 'idle' | 'pending' | 'saving' | 'saved' | 'error'

/** Editable note plus the save flight. Survives leaving the notes page. */
export interface NoteDraftState {
  /** Last acknowledged server copy. Null when no note is open. */
  note: Note | null
  title: string
  content: string
  folder: string
  status: NoteSaveStatus
  error: string
  /** Bumped when the open note changes, is discarded, or is deleted. */
  session: number
}

const EMPTY: NoteDraftState = {
  note: null,
  title: '',
  content: '',
  folder: '',
  status: 'idle',
  error: '',
  session: 0,
}

export const noteDraftStore = createWindowStore<NoteDraftState>(EMPTY)

let saveTimer: ReturnType<typeof setTimeout> | null = null
const retiredNoteIds = new Set<string>()

function clearSaveTimer() {
  if (saveTimer == null) return
  clearTimeout(saveTimer)
  saveTimer = null
}

export function resetNoteDraftStore() {
  clearSaveTimer()
  retiredNoteIds.clear()
  noteDraftStore.setState((prev) => ({ ...EMPTY, session: prev.session + 1 }))
}

function isDirty(state: NoteDraftState) {
  const note = state.note
  if (!note) return false
  return state.title !== note.title || state.content !== note.content || state.folder !== note.folder
}

function live(noteId: string, session: number) {
  const state = noteDraftStore.getSnapshot()
  return state.session === session && state.note?.id === noteId && !retiredNoteIds.has(noteId)
}

export function showNote(note: Note) {
  clearSaveTimer()
  noteDraftStore.setState((prev) => ({
    note,
    title: note.title,
    content: note.content,
    folder: note.folder,
    status: 'saved',
    error: '',
    session: prev.session + 1,
  }))
}

/** Explicit back. A later save completion must not reopen this draft. */
export function discardNoteDraft() {
  clearSaveTimer()
  noteDraftStore.setState((prev) => ({ ...EMPTY, session: prev.session + 1 }))
}

export function editNoteDraft(patch: { title?: string; content?: string; folder?: string }) {
  const current = noteDraftStore.getSnapshot()
  if (!current.note || retiredNoteIds.has(current.note.id)) return
  const title = patch.title ?? current.title
  const content = patch.content ?? current.content
  const folder = patch.folder ?? current.folder
  if (title === current.title && content === current.content && folder === current.folder) return
  const session = current.session
  noteDraftStore.setState((prev) => {
    if (prev.session !== session || !prev.note) return prev
    return {
      ...prev,
      title,
      content,
      folder,
      status: prev.status === 'saving' ? 'saving' : 'pending',
    }
  })
  clearSaveTimer()
  saveTimer = setTimeout(() => {
    saveTimer = null
    void flushNoteDraft()
  }, NOTE_SAVE_DEBOUNCE_MS)
}

function acknowledge(noteId: string, session: number, submitted: { title: string; content: string; folder: string }, updated: Note) {
  noteDraftStore.setState((prev) => {
    if (!live(noteId, session) || !prev.note) return prev
    return {
      ...prev,
      note: updated,
      title: prev.title === submitted.title ? updated.title : prev.title,
      content: prev.content === submitted.content ? updated.content : prev.content,
      folder: prev.folder === submitted.folder ? updated.folder : prev.folder,
      status: 'saving',
    }
  })
}

function markSavedIfClean(noteId: string, session: number) {
  noteDraftStore.setState((prev) => {
    if (!live(noteId, session) || isDirty(prev)) return prev
    if (prev.status === 'saved' && prev.error === '') return prev
    return { ...prev, status: 'saved', error: '' }
  })
}

async function writeNoteSession(noteId: string, session: number) {
  if (!live(noteId, session)) return
  if (!isDirty(noteDraftStore.getSnapshot())) {
    markSavedIfClean(noteId, session)
    return
  }
  noteDraftStore.setState((prev) => (
    live(noteId, session) ? { ...prev, status: 'saving', error: '' } : prev
  ))
  while (live(noteId, session)) {
    const snap = noteDraftStore.getSnapshot()
    if (!isDirty(snap)) {
      markSavedIfClean(noteId, session)
      if (!live(noteId, session) || isDirty(noteDraftStore.getSnapshot())) continue
      return
    }
    const submitted = { title: snap.title, content: snap.content, folder: snap.folder }
    if (!live(noteId, session)) return
    try {
      const updated = await api.notesUpdate(noteId, submitted.title, submitted.content, submitted.folder)
      if (!live(noteId, session)) return
      acknowledge(noteId, session, submitted, updated)
    } catch (err) {
      if (!live(noteId, session)) return
      const message = err instanceof Error ? err.message : String(err)
      noteDraftStore.setState((prev) => (
        live(noteId, session) ? { ...prev, status: 'error', error: message } : prev
      ))
      return
    }
  }
}

async function drainNote(noteId: string, session: number): Promise<boolean> {
  for (;;) {
    await noteDraftStore.run(`note:${noteId}`, () => writeNoteSession(noteId, session))
    const after = noteDraftStore.getSnapshot()
    if (after.session !== session || after.note?.id !== noteId) return false
    if (retiredNoteIds.has(noteId) || after.status === 'error') return false
    if (!isDirty(after)) return true
  }
}

/** Persist the open note until the latest draft is acknowledged, or the save fails. */
export function flushNoteDraft(): Promise<boolean> {
  clearSaveTimer()
  const start = noteDraftStore.getSnapshot()
  const noteId = start.note?.id
  if (!noteId) return Promise.resolve(true)
  const session = start.session
  if (!isDirty(start) && start.status !== 'saving') {
    if (start.status !== 'saved') markSavedIfClean(noteId, session)
    return Promise.resolve(true)
  }
  return drainNote(noteId, session)
}

/**
 * Delete is definitive: an in-flight save may finish its current request, but it
 * cannot reopen the draft or start another write. A failed delete keeps the draft.
 */
export async function settleNoteDelete(noteId: string, remove: () => Promise<void>) {
  retiredNoteIds.add(noteId)
  try {
    await noteDraftStore.run(`note:${noteId}`, async () => {})
    await remove()
    retiredNoteIds.delete(noteId)
    if (noteDraftStore.getSnapshot().note?.id === noteId) discardNoteDraft()
  } catch (error) {
    retiredNoteIds.delete(noteId)
    const snap = noteDraftStore.getSnapshot()
    if (snap.note?.id === noteId && snap.status === 'saving') {
      noteDraftStore.setState((prev) => (
        prev.note?.id === noteId
          ? { ...prev, status: isDirty(prev) ? 'pending' : 'saved' }
          : prev
      ))
    }
    throw error
  }
}
