import { createWindowStore } from '../../utils/windowStore'
import { loadStudyMaterial, type StudyReaderContext, type StudyRegion } from './studyMaterial'
import { requestStudyHelp, type StudyHelpInput } from './studyRequest'
import { readLegacyStudyAnswer } from './studyLegacyAnswer'
import {
  createEmptyStudyPage, deleteStudyDocument, importStudyDocument, loadStudyWorkspace,
  saveStudyDocument, setSelectedStudyDocument, validateStudyDocument, STUDY_LIMITS, type StudyDocument, type StudyPageState, type StudyTurn,
} from './studyStorage'

type StudyState = {
  documents: StudyDocument[]
  selectedDocumentId: string | null
  selectedTurnId: string | null
  loaded: boolean
  importing: boolean
  error: string
  notice: string
  dirtyIds: string[]
  saveError: string
  activeRequest: { documentId: string; page: number; turnId: string } | null
}
const initialState = (): StudyState => ({ documents: [], selectedDocumentId: null, selectedTurnId: null, loaded: false, importing: false, error: '', notice: '', dirtyIds: [], saveError: '', activeRequest: null })
/** The single owner of Study drafts, request identity and persistence across page navigation. */
export const studyWorkspace = createWindowStore(initialState())
const saving = new Map<string, Promise<void>>()
const removing = new Set<string>()
const documentSaveErrors = new Map<string, string>()
let selectionError = ''
function saveErrorMessage() { return [selectionError, ...documentSaveErrors.values()].filter(Boolean).join('\n') }
let requestController: AbortController | null = null
let importController: AbortController | null = null
let navigationRevision = 0
let localEditRevision = 0

function message(error: unknown): string { return error instanceof Error ? error.message : String(error) }
function documentById(id: string) { return studyWorkspace.getSnapshot().documents.find((doc) => doc.id === id) }
export function studyPage(doc: StudyDocument, page = doc.lastPage): StudyPageState { return doc.pages[String(page)] ?? createEmptyStudyPage() }

function flushDocument(id: string): Promise<void> {
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
  if (persist) void flushDocument(id).catch(() => {})
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
  studyWorkspace.getSnapshot().dirtyIds.forEach((id) => { void flushDocument(id).catch(() => {}) })
  if (selectionPending) void flushSelection().catch(() => {})
}
export function editStudyPage(documentId: string, page: number, patch: Partial<StudyPageState>) {
  const doc = documentById(documentId)
  if (!doc || !Number.isInteger(page) || page < 1 || page > doc.pageCount) return
  updateDocument(documentId, (doc) => ({ ...doc, pages: { ...doc.pages, [page]: { ...studyPage(doc, page), ...patch } } }))
}
export function openStudyPage(documentId: string, page: number, region?: StudyRegion | null, turnId: string | null = null) {
  const doc = documentById(documentId)
  if (!doc || removing.has(documentId) || !Number.isFinite(page)) return
  navigationRevision += 1
  const nextPage = Math.max(1, Math.min(doc.pageCount, Math.round(page)))
  studyWorkspace.setState((state) => ({ ...state, selectedDocumentId: documentId, selectedTurnId: turnId, error: '' }))
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
  const canReload = () => !studyWorkspace.getSnapshot().activeRequest && !importController && removing.size === 0
  if (!canReload()) return Promise.reject(new Error('Stop the active reply, import, or removal before reloading saved work.'))
  const revision = localEditRevision
  return studyWorkspace.run('reload', async () => {
    if (!canReload()) throw new Error('Stop the active reply, import, or removal before reloading saved work.')
    // Already-submitted saves must settle before reading the authoritative version.
    await Promise.all([...saving.values()].map((flight) => flight.catch(() => {})))
    await selecting?.catch(() => {})
    const loaded = await loadStudyWorkspace()
    if (!canReload() || revision !== localEditRevision) throw new Error('Your local work changed while reloading. Nothing was discarded. Confirm again when you are ready to reload.')
    if (loaded.warnings.length) throw new Error(`Saved work could not be fully recovered. Your local drafts remain unchanged. ${loaded.warnings.join(' ')}`)
    documentSaveErrors.clear()
    selectionError = ''
    selectionPending = false
    studyWorkspace.setState((state) => ({ ...state, ...loaded, loaded: true, selectedTurnId: null, dirtyIds: [], saveError: '', error: '', notice: loaded.warnings.join('\n') }))
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
        selectedTurnId: revision === navigationRevision && !controller.signal.aborted ? null : state.selectedTurnId,
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
  if (studyWorkspace.getSnapshot().activeRequest?.documentId === id) throw new Error('Stop the active reply before removing this material.')
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
      return { ...state, documents: state.documents.filter((doc) => doc.id !== id), selectedDocumentId: state.selectedDocumentId === id ? state.documents.find((doc) => doc.id !== id)?.id ?? null : state.selectedDocumentId, selectedTurnId: state.selectedDocumentId === id ? null : state.selectedTurnId, dirtyIds, saveError: saveErrorMessage() }
    })
    selectionPending = true
    await flushSelection()
  } finally { removing.delete(id) }
}
function withTurnPatch(doc: StudyDocument, page: number, turnId: string, patch: Partial<StudyTurn>): StudyDocument {
  return { ...doc, pages: { ...doc.pages, [page]: { ...studyPage(doc, page), history: studyPage(doc, page).history.map((turn) => turn.id === turnId ? { ...turn, ...patch } : turn) } } }
}
function patchTurn(documentId: string, page: number, turnId: string, patch: Partial<StudyTurn>, persist = true) {
  updateDocument(documentId, (doc) => withTurnPatch(doc, page, turnId, patch), persist)
}
export function sameStudyRegion(a?: StudyRegion | null, b?: StudyRegion | null) {
  return (!a && !b) || Boolean(a && b && a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height)
}
export async function sendStudyHelp(options: {
  documentId: string; page: number; mode: StudyTurn['mode']; providerId: string; model: string
  context: StudyReaderContext; visionCapable: boolean; retry?: StudyTurn
}) {
  const { documentId, page, context } = options
  const doc = documentById(documentId)
  if (!doc || removing.has(documentId) || studyWorkspace.getSnapshot().activeRequest) return
  if (!Number.isInteger(page) || page < 1 || page > doc.pageCount) throw new Error('Select a valid document page.')
  const draft = studyPage(doc, page)
  // Resolve the persisted snapshot, rather than trusting fields on a caller's old copy.
  const retry = options.retry && draft.history.find((turn) => turn.id === options.retry?.id)
  if (options.retry && (options.retry.page !== page || !retry)) throw new Error('This saved question belongs to another material or page.')
  if (draft.history.length >= STUDY_LIMITS.maxHistoryPerPage) throw new Error('This page has reached its 500-response history limit. Your question and notes are unchanged. Start on another page or material.')
  if (context.status !== 'ready' || context.page !== page || !sameStudyRegion(context.region, retry ? retry.region : draft.region)) throw new Error('Wait for the selected page or region to finish loading.')
  if (!context.imageDataUrl?.trim()) throw new Error('Wait for the original page image to finish loading.')
  if (options.visionCapable !== true) throw new Error('Choose a vision-capable model to study the original page image.')
  const mode = retry?.mode ?? options.mode
  const question = retry?.question ?? draft.question.trim()
  const attempt = retry?.attempt ?? draft.attempt.trim()
  if (!question) throw new Error('Write a question about this page first. / 请先写下问题。')
  if (mode === 'check' && !attempt.trim()) throw new Error('Add your attempt before checking it. / 请先写出自己的解答或思路。')
  const turnId = crypto.randomUUID()
  const input: StudyHelpInput = { requestId: turnId, documentId, documentName: doc.name, pageNumber: page, mode,
    question, attempt, imageDataUrl: context.imageDataUrl, visionCapable: options.visionCapable,
    providerId: options.providerId, model: options.model,
    history: draft.history.filter((turn) => turn.status === 'complete').slice(-6).flatMap((turn) => [{ role: 'user' as const, content: `${turn.question}\n${turn.attempt}` }, { role: 'assistant' as const, content: readLegacyStudyAnswer(turn.answer)?.visibleText ?? turn.answer }]),
  }
  const selectedRegion = retry ? retry.region : draft.region
  const turn: StudyTurn = { id: turnId, page, mode, question, attempt, sourceText: '', region: selectedRegion ? { ...selectedRegion } : selectedRegion,
    answer: '', status: 'streaming', createdAt: Date.now(), providerId: options.providerId, model: options.model,
    sourceImageUsed: true, sourceWarning: context.warning,
  }
  // Leave room for a bounded error/terminal status even if the provider fills the response budget.
  const terminalReserve = 4096
  validateStudyDocument({ ...doc, pages: { ...doc.pages, [page]: { ...draft, history: [...draft.history, turn] } } }, terminalReserve)
  const controller = new AbortController()
  requestController = controller
  studyWorkspace.setState((state) => ({ ...state, activeRequest: { documentId, page, turnId }, selectedTurnId: turnId, error: '' }))
  editStudyPage(documentId, page, { history: [...draft.history, turn] })
  let content = ''
  let responseLimitError = ''
  let responseBytesRemaining = 0
  let budgetSnapshot: StudyDocument | undefined
  const encoder = new TextEncoder()
  const refreshResponseBudget = () => {
    budgetSnapshot = documentById(documentId)
    if (budgetSnapshot) responseBytesRemaining = validateStudyDocument(budgetSnapshot, terminalReserve)
  }
  const stopAtResponseLimit = () => {
    responseLimitError = 'The response reached the local storage limit. Its partial answer is still in this window. Shorten notes or use another material before retrying.'
    controller.abort()
  }
  const validateAnswer = (answer: string) => {
    const latest = documentById(documentId)
    if (latest) validateStudyDocument(withTurnPatch(latest, page, turnId, { answer }), terminalReserve)
  }
  let checkpoint: ReturnType<typeof setTimeout> | undefined
  try {
    // Persist the placeholder before starting the remote operation. A failed save must be visible.
    await flushDocument(documentId)
    if (controller.signal.aborted) throw new DOMException('Study request cancelled.', 'AbortError')
    refreshResponseBudget()
    const result = await requestStudyHelp(input, (delta) => {
      if (controller.signal.aborted) return
      // Only edits/checkpoints outside this stream invalidate the full-document budget.
      // Encoding each fragment overestimates split surrogate pairs, so the byte bound stays safe.
      try {
        if (budgetSnapshot !== documentById(documentId)) refreshResponseBudget()
        const bytes = encoder.encode(JSON.stringify(delta)).byteLength - 2
        if (content.length + delta.length > STUDY_LIMITS.maxTextLength || bytes > responseBytesRemaining) {
          stopAtResponseLimit()
          return
        }
        responseBytesRemaining -= bytes
      } catch { stopAtResponseLimit(); return }
      content += delta
      patchTurn(documentId, page, turnId, { answer: content }, false)
      budgetSnapshot = documentById(documentId)
      if (!checkpoint && !studyWorkspace.getSnapshot().saveError) checkpoint = setTimeout(() => {
        checkpoint = undefined
        try { refreshResponseBudget() } catch { stopAtResponseLimit(); return }
        void flushDocument(documentId).catch(() => {})
      }, 250)
    }, controller.signal)
    if (responseLimitError) throw new Error(responseLimitError)
    const answer = result.content || content
    validateAnswer(answer)
    patchTurn(documentId, page, turnId, { answer, status: controller.signal.aborted ? 'cancelled' : 'complete' })
  } catch (error) {
    const cancelled = controller.signal.aborted && !responseLimitError
    patchTurn(documentId, page, turnId, { answer: content, status: cancelled ? 'cancelled' : 'error', error: cancelled ? undefined : (responseLimitError || message(error)).slice(0, 512) })
  } finally {
    if (checkpoint) clearTimeout(checkpoint)
    if (requestController === controller) requestController = null
    studyWorkspace.setState((state) => state.activeRequest?.turnId === turnId ? { ...state, activeRequest: null } : state)
  }
}
export function cancelStudyHelp() { requestController?.abort() }

/** Test cleanup releases resources, then restores the owner to its initial snapshot. */
export function resetStudyWorkspaceForTests() { requestController?.abort(); importController?.abort(); requestController = null; importController = null; navigationRevision += 1; localEditRevision += 1; saving.clear(); removing.clear(); documentSaveErrors.clear(); selectionError = ''; selectionPending = false; selecting = null; studyWorkspace.setState(initialState()) }
