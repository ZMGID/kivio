import { IDBFactory } from 'fake-indexeddb'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { StudyDocument, StudyTurn } from './studyStorage'
import type { StudyHelpInput, StudyHelpResponse } from './studyRequest'
import type { StudyReaderContext } from './studyMaterial'

const mocks = vi.hoisted(() => ({ material: vi.fn(), request: vi.fn() }))
vi.mock('./studyMaterial', () => ({ loadStudyMaterial: mocks.material }))
vi.mock('./studyRequest', () => ({ requestStudyHelp: mocks.request }))
let owner: typeof import('./studyWorkspaceStore')
let storage: typeof import('./studyStorage')

function deferred<T = void>() {
  let resolve!: (value: T | PromiseLike<T>) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}

beforeEach(async () => {
  vi.resetModules()
  vi.stubGlobal('indexedDB', new IDBFactory())
  mocks.material.mockReset().mockResolvedValue({ kind: 'pdf', pageCount: 3, mimeType: 'application/pdf' })
  mocks.request.mockReset().mockImplementation(async (input: StudyHelpInput) => response(input))
  storage = await import('./studyStorage')
  owner = await import('./studyWorkspaceStore')
})

afterEach(() => {
  owner.resetStudyWorkspaceForTests()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

function response(input: StudyHelpInput, content = 'A helpful answer'): StudyHelpResponse {
  return { requestId: input.requestId, documentId: input.documentId, pageNumber: input.pageNumber, mode: input.mode ?? 'hint', providerId: input.providerId, model: input.model, content }
}
function context(page = 1, region: StudyReaderContext['region'] = null): StudyReaderContext {
  return { page, pageCount: 3, text: 'Original page text', region, status: 'ready', imageDataUrl: 'data:image/png;base64,newImage' }
}
function options(document: StudyDocument, page = 1) {
  return { documentId: document.id, page, mode: 'hint' as const, providerId: 'provider', model: 'model', includeImage: false, context: context(page) }
}
async function seed(count = 1) {
  const documents: StudyDocument[] = []
  for (let index = 0; index < count; index++) {
    const imported = await storage.importStudyDocument({ name: `${index}.pdf`, kind: 'pdf', pageCount: 3, blob: new Blob([`document ${index}`]) })
    documents.push(imported.document)
  }
  await owner.initializeStudy()
  return documents
}
async function saved() {
  await vi.waitFor(() => expect(owner.studyWorkspace.getSnapshot().dirtyIds).toEqual([]))
}
async function question(document: StudyDocument, value = 'Why this equation?') {
  owner.editStudyPage(document.id, 1, { question: value })
  await saved()
}
function current(document: StudyDocument) { return owner.studyWorkspace.getSnapshot().documents.find((item) => item.id === document.id)! }
function holdNextSave() {
  const release = deferred()
  const entered = deferred()
  const save = storage.saveStudyDocument
  vi.spyOn(storage, 'saveStudyDocument').mockImplementationOnce(async (document) => {
    entered.resolve()
    await release.promise
    await save(document)
  })
  return { entered: entered.promise, release: () => release.resolve() }
}

describe('Study workspace lifecycle', () => {
  it('can retry initialization after a transient storage failure', async () => {
    const loading = vi.spyOn(storage, 'loadStudyWorkspace').mockRejectedValueOnce(new Error('Storage temporarily locked'))
    await owner.initializeStudy()
    expect(owner.studyWorkspace.getSnapshot()).toMatchObject({ loaded: false, error: 'Storage temporarily locked' })
    await owner.initializeStudy()
    expect(owner.studyWorkspace.getSnapshot()).toMatchObject({ loaded: true, error: '' })
    expect(loading).toHaveBeenCalledTimes(2)
  })

  it('restores loading before an import, so startup cannot overwrite a new material', async () => {
    const release = deferred()
    const load = storage.loadStudyWorkspace
    vi.spyOn(storage, 'loadStudyWorkspace').mockImplementationOnce(async () => { await release.promise; return load() })
    const initial = owner.initializeStudy()
    const importing = owner.importStudyFile(new File(['new material'], 'new.pdf'))
    await Promise.resolve()
    expect(mocks.material).not.toHaveBeenCalled()
    release.resolve()
    await Promise.all([initial, importing])
    expect(owner.studyWorkspace.getSnapshot().documents).toHaveLength(1)
    expect((await storage.loadStudyWorkspace()).documents).toHaveLength(1)
  })

  it('serializes placeholder persistence with queued edits and sends only after it is saved', async () => {
    const [document] = await seed()
    await question(document, 'Original question')
    const save = holdNextSave()
    owner.editStudyPage(document.id, 1, { notes: 'Edit before sending' })
    await save.entered
    const sending = owner.sendStudyHelp(options(document))
    owner.editStudyPage(document.id, 1, { question: 'Draft for my next question', notes: 'Most recent notes' })
    await Promise.resolve()
    expect(mocks.request).not.toHaveBeenCalled()
    let placeholderSaved = false
    mocks.request.mockImplementationOnce(async (input: StudyHelpInput) => {
      const persisted = (await storage.loadStudyWorkspace()).documents[0]
      placeholderSaved = persisted.pages['1'].history.some((turn) => turn.id === input.requestId)
      return response(input)
    })
    save.release()
    await sending
    await saved()
    expect(placeholderSaved).toBe(true)
    const persisted = (await storage.loadStudyWorkspace()).documents[0]
    expect(persisted.pages['1']).toMatchObject({ question: 'Draft for my next question', notes: 'Most recent notes' })
    expect(persisted.pages['1'].history[0]).toMatchObject({ question: 'Original question', status: 'complete', answer: 'A helpful answer' })
  })

  it('does not call the provider after Stop during placeholder persistence', async () => {
    const [document] = await seed()
    await question(document)
    const save = holdNextSave()
    const sending = owner.sendStudyHelp(options(document))
    await save.entered
    owner.cancelStudyHelp()
    save.release()
    await sending
    await saved()
    expect(mocks.request).not.toHaveBeenCalled()
    expect(current(document).pages['1'].history[0].status).toBe('cancelled')
    expect((await storage.loadStudyWorkspace()).documents[0].pages['1'].history[0].status).toBe('cancelled')
  })

  it('does not spend a provider request if the placeholder cannot be persisted', async () => {
    const [document] = await seed()
    await question(document)
    vi.spyOn(storage, 'saveStudyDocument').mockRejectedValueOnce(new storage.StudyStorageError('quota', 'Storage is full'))
    await owner.sendStudyHelp(options(document))
    await saved()
    expect(mocks.request).not.toHaveBeenCalled()
    expect(current(document).pages['1'].history[0]).toMatchObject({ status: 'error', error: 'Storage is full' })
  })

  it('keeps unsaved draft errors visible and retries the actual unsaved document', async () => {
    const [document] = await seed()
    vi.spyOn(storage, 'saveStudyDocument').mockRejectedValueOnce(new Error('Disk full'))
    owner.editStudyPage(document.id, 2, { notes: 'Do not lose this note' })
    await vi.waitFor(() => expect(owner.studyWorkspace.getSnapshot().saveError).toBe('Disk full'))
    expect(owner.studyWorkspace.getSnapshot().dirtyIds).toContain(document.id)
    expect((await storage.loadStudyWorkspace()).documents[0].pages).toEqual({})
    owner.retryStudySave()
    await saved()
    expect(owner.studyWorkspace.getSnapshot().saveError).toBe('')
    expect((await storage.loadStudyWorkspace()).documents[0].pages['2'].notes).toBe('Do not lose this note')
  })

  it('keeps selection failures visible even when document saves succeed and retries the selection', async () => {
    const [first, second] = await seed(2)
    vi.spyOn(storage, 'setSelectedStudyDocument').mockRejectedValueOnce(new Error('Could not save reading position'))
    owner.openStudyPage(first.id, 2)
    await saved()
    await vi.waitFor(() => expect(owner.studyWorkspace.getSnapshot().saveError).toBe('Could not save reading position'))
    expect((await storage.loadStudyWorkspace()).selectedDocumentId).toBe(second.id)
    owner.retryStudySave()
    await vi.waitFor(() => expect(owner.studyWorkspace.getSnapshot().saveError).toBe(''))
    const persisted = await storage.loadStudyWorkspace()
    expect(persisted.selectedDocumentId).toBe(first.id)
    expect(persisted.documents.find((doc) => doc.id === first.id)?.lastPage).toBe(2)
  })

  it('binds a late response to its original document and page after navigation', async () => {
    const [first, second] = await seed(2)
    await question(first)
    const started = deferred()
    const finish = deferred<StudyHelpResponse>()
    let input!: StudyHelpInput
    let delta!: (value: string) => void
    mocks.request.mockImplementationOnce((value: StudyHelpInput, onDelta: (text: string) => void) => { input = value; delta = onDelta; started.resolve(); return finish.promise })
    const sending = owner.sendStudyHelp(options(first))
    await started.promise
    delta('Partial answer')
    owner.openStudyPage(second.id, 3)
    owner.editStudyPage(second.id, 3, { notes: 'Work on the other document' })
    finish.resolve(response(input, 'Completed original answer'))
    await sending
    await saved()
    expect(owner.studyWorkspace.getSnapshot()).toMatchObject({ selectedDocumentId: second.id, selectedTurnId: null, activeRequest: null })
    expect(current(first).pages['1'].history[0].answer).toBe('Completed original answer')
    expect(current(second).pages['3']).toMatchObject({ notes: 'Work on the other document', history: [] })
    expect(current(second).lastPage).toBe(3)
  })

  it('checkpoints partial output and ignores late deltas after cancellation', async () => {
    const [document] = await seed()
    await question(document)
    const started = deferred()
    let delta!: (value: string) => void
    mocks.request.mockImplementationOnce((_input: StudyHelpInput, onDelta: (text: string) => void, signal: AbortSignal) => {
      delta = onDelta
      started.resolve()
      return new Promise((_, reject) => signal.addEventListener('abort', () => reject(new DOMException('Stopped', 'AbortError')), { once: true }))
    })
    const sending = owner.sendStudyHelp(options(document))
    await started.promise
    delta('Keep this partial answer')
    expect(owner.studyWorkspace.getSnapshot().dirtyIds).toContain(document.id)
    await vi.waitFor(async () => expect((await storage.loadStudyWorkspace()).documents[0].pages['1'].history[0].answer).toBe('Keep this partial answer'))
    owner.cancelStudyHelp()
    delta('Late text must not leak')
    await sending
    await saved()
    expect(current(document).pages['1'].history[0]).toMatchObject({ status: 'cancelled', answer: 'Keep this partial answer' })
  })

  it('retries the original request snapshot and a null original region rather than a newer draft', async () => {
    const [document] = await seed()
    const original: StudyTurn = { id: 'original-failed-turn', page: 1, mode: 'check', question: 'Original question', attempt: 'Original attempt', sourceText: 'Original corrected text', region: null, answer: '', status: 'error', error: 'Temporary failure', createdAt: 123, providerId: 'old-provider', model: 'old-model', sourceImageUsed: true, sourceWarning: 'Original OCR warning' }
    owner.editStudyPage(document.id, 1, { question: 'Changed question', attempt: 'Changed attempt', correctedText: 'Changed source', region: { x: 0, y: 0, width: 0.5, height: 0.5 }, history: [original] })
    await saved()
    owner.openStudyPage(document.id, 1, original.region, original.id)
    await owner.sendStudyHelp({ ...options(document), mode: 'check', retry: original, context: { ...context(), warning: 'New extraction warning' } })
    await saved()
    expect(mocks.request.mock.calls[0][0]).toMatchObject({ question: 'Original question', attempt: 'Original attempt', pageText: 'Original corrected text', imageDataUrl: 'data:image/png;base64,newImage', providerId: 'provider', model: 'model' })
    expect(current(document).pages['1'].history[1]).toMatchObject({ region: null, sourceWarning: 'Original OCR warning', sourceImageUsed: true })
  })

  it('rejects a retry from another page or document and waits for its exact region', async () => {
    const [document] = await seed()
    await question(document)
    const original: StudyTurn = { id: 'owned', page: 1, mode: 'hint', question: 'Original', attempt: '', sourceText: 'text', region: { x: 0, y: 0, width: 0.5, height: 0.5 }, answer: '', status: 'error', createdAt: 1, providerId: 'provider', model: 'model' }
    owner.editStudyPage(document.id, 1, { history: [original] })
    await saved()
    await expect(owner.sendStudyHelp({ ...options(document), retry: { ...original, id: 'from-another-document' } })).rejects.toThrow('another material or page')
    await expect(owner.sendStudyHelp({ ...options(document, 2), retry: original })).rejects.toThrow('another material or page')
    await expect(owner.sendStudyHelp({ ...options(document), retry: original })).rejects.toThrow('region to finish loading')
    expect(mocks.request).not.toHaveBeenCalled()
  })

  it('locks out edits and new requests before waiting for a deletion and never resurrects the document', async () => {
    const [first, second] = await seed(2)
    await question(first)
    const save = holdNextSave()
    owner.editStudyPage(first.id, 1, { notes: 'Pending notes' })
    await save.entered
    const removing = owner.removeStudyDocument(first.id)
    owner.editStudyPage(first.id, 1, { notes: 'Must not start another save' })
    await owner.sendStudyHelp(options(first))
    expect(mocks.request).not.toHaveBeenCalled()
    expect(current(first).pages['1'].notes).toBe('Pending notes')
    save.release()
    await removing
    await saved()
    expect(owner.studyWorkspace.getSnapshot()).toMatchObject({ selectedDocumentId: second.id, activeRequest: null, saveError: '' })
    expect(owner.studyWorkspace.getSnapshot().documents.map((doc) => doc.id)).toEqual([second.id])
    expect((await storage.loadStudyWorkspace()).documents.map((doc) => doc.id)).toEqual([second.id])
    await expect(storage.readStudyDocumentBlob(first.id)).rejects.toMatchObject({ code: 'not-found' })
  })

  it('keeps a material usable when deleting it fails', async () => {
    const [document] = await seed()
    vi.spyOn(storage, 'deleteStudyDocument').mockRejectedValueOnce(new Error('Could not commit deletion'))
    await expect(owner.removeStudyDocument(document.id)).rejects.toThrow('Could not commit deletion')
    owner.editStudyPage(document.id, 1, { notes: 'Still editable' })
    await saved()
    expect((await storage.loadStudyWorkspace()).documents[0].pages['1'].notes).toBe('Still editable')
    await owner.removeStudyDocument(document.id)
    expect(owner.studyWorkspace.getSnapshot().documents).toEqual([])
  })

  it('refuses to remove the document while its provider request is active', async () => {
    const [document] = await seed()
    await question(document)
    const started = deferred()
    mocks.request.mockImplementationOnce((_input: StudyHelpInput, _delta: unknown, signal: AbortSignal) => {
      started.resolve()
      return new Promise((_, reject) => signal.addEventListener('abort', () => reject(new DOMException('Stopped', 'AbortError')), { once: true }))
    })
    const sending = owner.sendStudyHelp(options(document))
    await started.promise
    await expect(owner.removeStudyDocument(document.id)).rejects.toThrow('Stop the active reply')
    owner.cancelStudyHelp()
    await sending
    await saved()
  })

  it('does not let a late import replace newer navigation or its persisted selection', async () => {
    const [first, second] = await seed(2)
    const parse = deferred<{ kind: 'pdf'; pageCount: number }>()
    mocks.material.mockReturnValueOnce(parse.promise)
    const importing = owner.importStudyFile(new File(['third'], 'third.pdf'))
    await vi.waitFor(() => expect(mocks.material).toHaveBeenCalled())
    owner.openStudyPage(first.id, 2)
    parse.resolve({ kind: 'pdf', pageCount: 3 })
    await importing
    await saved()
    expect(owner.studyWorkspace.getSnapshot().selectedDocumentId).toBe(first.id)
    const persisted = await storage.loadStudyWorkspace()
    expect(persisted.documents).toHaveLength(3)
    expect(persisted.selectedDocumentId).toBe(first.id)
    expect(persisted.documents.some((doc) => doc.id === second.id)).toBe(true)
  })

  it('cancels a newly queued import before parsing or committing any material', async () => {
    await owner.initializeStudy()
    const importing = owner.importStudyFile(new File(['cancel me'], 'cancel.pdf'))
    owner.cancelStudyImport()
    await importing
    expect(mocks.material).not.toHaveBeenCalled()
    expect(owner.studyWorkspace.getSnapshot()).toMatchObject({ importing: false, documents: [], error: '' })
    expect((await storage.loadStudyWorkspace()).documents).toEqual([])
  })

  it('cancels after parsing but before the import hash completes without committing a material', async () => {
    await owner.initializeStudy()
    const hash = deferred<ArrayBuffer>()
    const entered = deferred()
    const file = new File(['cancel during hash'], 'cancel.pdf')
    vi.spyOn(file, 'arrayBuffer').mockImplementationOnce(() => { entered.resolve(); return hash.promise })
    const importing = owner.importStudyFile(file)
    await entered.promise
    owner.cancelStudyImport()
    hash.resolve(new TextEncoder().encode('cancel during hash').buffer)
    await importing
    expect(owner.studyWorkspace.getSnapshot()).toMatchObject({ importing: false, documents: [], error: '' })
    expect((await storage.loadStudyWorkspace()).documents).toEqual([])
  })

  it('ignores invalid page navigation instead of persisting an unusable reading position', async () => {
    const [document] = await seed()
    owner.openStudyPage(document.id, Number.NaN)
    owner.editStudyPage(document.id, 4, { notes: 'Outside document' })
    expect(current(document).lastPage).toBe(1)
    expect(current(document).pages).toEqual({})
    expect(owner.studyWorkspace.getSnapshot().dirtyIds).toEqual([])
  })
})
