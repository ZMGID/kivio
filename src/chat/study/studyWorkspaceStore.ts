import { createWindowStore } from '../../utils/windowStore'
import { loadStudyMaterial, type StudyRegion } from './studyMaterial'
import {
  createEmptyStudyPage, deleteStudyDocument, importStudyDocument, loadStudyWorkspace,
  saveStudyDocument, setSelectedStudyDocument, type StudyDocument, type StudyPageState,
} from './studyStorage'

type StudyState = {
  documents: StudyDocument[]
  selectedDocumentId: string | null
  loaded: boolean
  importing: boolean
  error: string
  notice: string
  dirtyIds: string[]
  saveError: string
}
const initialState = (): StudyState => ({ documents: [], selectedDocumentId: null, loaded: false, importing: false, error: '', notice: '', dirtyIds: [], saveError: '' })
/** The single owner of materials, reader position, notes and restart-persistent source drafts. Chat owns conversations and requests. */
export const studyWorkspace = createWindowStore(initialState())
const saving = new Map<string, Promise<void>>()
const removing = new Set<string>()
const documentSaveErrors = new Map<string, string>()
let selectionError = ''
function saveErrorMessage() { return [selectionError, ...documentSaveErrors.values()].filter(Boolean).join('\n') }
let importController: AbortController | null = null
let navigationRevision = 0
let localEditRevision = 0

function message(error: unknown): string { return error instanceof Error ? error.message : String(error) }
function documentById(id: string) { return studyWorkspace.getSnapshot().documents.find((doc) => doc.id === id) }
export function studyPage(doc: StudyDocument, page = doc.lastPage): StudyPageState { return doc.pages[String(page)] ?? createEmptyStudyPage() }

export function flushStudyDocument(id: string): Promise<void> {
  const existing = saving.get(id)
  if (existing) return existing
  const flight = Promise.resolve().then(async () => {
    try {
      while (documentById(id) && !removing.has(id)) {
        const snapshot = documentById(id)
        if (!snapshot) break
        const revision = await saveStudyDocument(snapshot)
        const unchanged = snapshot === documentById(id)
        // This window may have newer local edits while its previous snapshot is committing.
        // Advance their CAS baseline without replacing any of those edits with the saved snapshot.
        studyWorkspace.setState((state) => ({ ...state, documents: state.documents.map((doc) => doc.id === id ? { ...doc, revision } : doc) }))
        if (unchanged) {
          documentSaveErrors.delete(id)
          studyWorkspace.setState((state) => ({ ...state, dirtyIds: state.dirtyIds.filter((key) => key !== id), saveError: saveErrorMessage() }))
          break
        }
      }
    } catch (error) {
      documentSaveErrors.set(id, message(error))
      studyWorkspace.setState((state) => ({ ...state, saveError: saveErrorMessage() }))
      throw error
    }
  }).finally(() => { if (saving.get(id) === flight) saving.delete(id) })
  saving.set(id, flight)
  return flight
}
function updateDocument(id: string, update: (doc: StudyDocument) => StudyDocument, persist = true) {
  if (!documentById(id) || removing.has(id)) return
  localEditRevision += 1
  studyWorkspace.setState((state) => ({
    ...state,
    documents: state.documents.map((doc) => doc.id === id ? { ...update(doc), updatedAt: Date.now() } : doc),
    dirtyIds: state.dirtyIds.includes(id) ? state.dirtyIds : [...state.dirtyIds, id],
  }))
  if (persist) void flushStudyDocument(id).catch(() => {})
}
let selectionPending = false
let selecting: Promise<void> | null = null
function flushSelection(): Promise<void> {
  if (selecting) return selecting
  const flight = Promise.resolve().then(async () => {
    while (selectionPending) {
      const id = studyWorkspace.getSnapshot().selectedDocumentId
      await setSelectedStudyDocument(id)
      if (studyWorkspace.getSnapshot().selectedDocumentId === id) selectionPending = false
    }
    selectionError = ''
    studyWorkspace.setState((state) => ({ ...state, saveError: saveErrorMessage() }))
  }).catch((error) => {
    selectionError = message(error)
    studyWorkspace.setState((state) => ({ ...state, saveError: saveErrorMessage() }))
    throw error
  }).finally(() => { if (selecting === flight) selecting = null })
  selecting = flight
  return flight
}
function persistSelection() { selectionPending = true; void flushSelection().catch(() => {}) }
export function retryStudySave() {
  studyWorkspace.getSnapshot().dirtyIds.forEach((id) => { void flushStudyDocument(id).catch(() => {}) })
  if (selectionPending) void flushSelection().catch(() => {})
}
export function editStudyPage(documentId: string, page: number, patch: Partial<StudyPageState>) {
  const doc = documentById(documentId)
  if (!doc || !Number.isInteger(page) || page < 1 || page > doc.pageCount) return
  updateDocument(documentId, (doc) => ({ ...doc, pages: { ...doc.pages, [page]: { ...studyPage(doc, page), ...patch } } }))
}
export function openStudyPage(documentId: string, page: number, region?: StudyRegion | null) {
  const doc = documentById(documentId)
  if (!doc || removing.has(documentId) || !Number.isFinite(page)) return
  navigationRevision += 1
  const nextPage = Math.max(1, Math.min(doc.pageCount, Math.round(page)))
  studyWorkspace.setState((state) => ({ ...state, selectedDocumentId: documentId, error: '' }))
  updateDocument(documentId, (item) => ({ ...item, lastPage: nextPage, ...(region === undefined ? {} : { pages: { ...item.pages, [nextPage]: { ...studyPage(item, nextPage), region } } }) }))
  persistSelection()
}
export function initializeStudy() {
  return studyWorkspace.run('initialize', async () => {
    if (studyWorkspace.getSnapshot().loaded) return
    try {
      const loaded = await loadStudyWorkspace()
      studyWorkspace.setState((state) => ({ ...state, ...loaded, loaded: true, notice: loaded.warnings.join('\n'), error: '' }))
    } catch (error) { studyWorkspace.setState((state) => ({ ...state, loaded: false, error: message(error) })) }
  })
}
/** The caller must confirm discarding local edits before invoking this recovery action. */
export function reloadStudyWorkspace(): Promise<void> {
  const canReload = () => !importController && removing.size === 0
  if (!canReload()) return Promise.reject(new Error('Finish the import or removal before reloading saved work.'))
  const revision = localEditRevision
  return studyWorkspace.run('reload', async () => {
    if (!canReload()) throw new Error('Finish the import or removal before reloading saved work.')
    // Already-submitted saves must settle before reading the authoritative version.
    await Promise.all([...saving.values()].map((flight) => flight.catch(() => {})))
    await selecting?.catch(() => {})
    const loaded = await loadStudyWorkspace()
    if (!canReload() || revision !== localEditRevision) throw new Error('Your local work changed while reloading. Nothing was discarded. Confirm again when you are ready to reload.')
    if (loaded.warnings.length) throw new Error(`Saved work could not be fully recovered. Your local drafts remain unchanged. ${loaded.warnings.join(' ')}`)
    documentSaveErrors.clear()
    selectionError = ''
    selectionPending = false
    studyWorkspace.setState((state) => ({ ...state, ...loaded, loaded: true, dirtyIds: [], saveError: '', error: '', notice: loaded.warnings.join('\n') }))
  })
}

export function importStudyFile(file: File) {
  if (importController) return studyWorkspace.run('import', async () => {})
  const revision = navigationRevision
  localEditRevision += 1
  const controller = new AbortController()
  importController = controller
  studyWorkspace.setState((state) => ({ ...state, importing: true, error: '', notice: '' }))
  return studyWorkspace.run('import', async () => {
    try {
      await initializeStudy()
      if (controller.signal.aborted) return
      if (!studyWorkspace.getSnapshot().loaded) throw new Error(studyWorkspace.getSnapshot().error || 'Could not restore the Study library. Retry before importing.')
      const material = await loadStudyMaterial(file, controller.signal)
      if (controller.signal.aborted) return
      const result = await importStudyDocument({ name: file.name, kind: material.kind, pageCount: material.pageCount, blob: file, signal: controller.signal })
      studyWorkspace.setState((state) => ({ ...state,
        documents: state.documents.some((doc) => doc.id === result.document.id) ? state.documents : [result.document, ...state.documents],
        selectedDocumentId: revision === navigationRevision && !controller.signal.aborted ? result.document.id : state.selectedDocumentId,
        notice: result.duplicate ? 'This material is already in your library. Your saved work is unchanged. / 材料已存在，已保留原有学习记录。' : '',
      }))
      if (revision !== navigationRevision || controller.signal.aborted) { selectionPending = true; await flushSelection() }
    } catch (error) {
      if (!controller.signal.aborted) studyWorkspace.setState((state) => ({ ...state, error: message(error) }))
    } finally {
      if (importController === controller) importController = null
      studyWorkspace.setState((state) => ({ ...state, importing: false }))
    }
  })
}
export function cancelStudyImport() { importController?.abort() }
export async function removeStudyDocument(id: string) {
  if (removing.has(id)) return
  removing.add(id)
  localEditRevision += 1
  navigationRevision += 1
  try {
    // Lock edits and new requests before waiting, then let any in-flight write finish.
    await saving.get(id)?.catch(() => {})
    await deleteStudyDocument(id)
    documentSaveErrors.delete(id)
    studyWorkspace.setState((state) => {
      const dirtyIds = state.dirtyIds.filter((key) => key !== id)
      return { ...state, documents: state.documents.filter((doc) => doc.id !== id), selectedDocumentId: state.selectedDocumentId === id ? state.documents.find((doc) => doc.id !== id)?.id ?? null : state.selectedDocumentId, dirtyIds, saveError: saveErrorMessage() }
    })
    selectionPending = true
    await flushSelection()
  } finally { removing.delete(id) }
}
export function sameStudyRegion(a?: StudyRegion | null, b?: StudyRegion | null) {
  return (!a && !b) || Boolean(a && b && a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height)
}

/** Test cleanup releases material operations only; Chat owns its own request lifecycle. */
export function resetStudyWorkspaceForTests() { importController?.abort(); importController = null; navigationRevision += 1; localEditRevision += 1; saving.clear(); removing.clear(); documentSaveErrors.clear(); selectionError = ''; selectionPending = false; selecting = null; studyWorkspace.setState(initialState()) }
