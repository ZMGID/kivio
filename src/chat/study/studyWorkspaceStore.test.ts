import { IDBFactory } from 'fake-indexeddb'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { StudyDocument } from './studyStorage'

const mocks = vi.hoisted(() => ({ material: vi.fn() }))
vi.mock('./studyMaterial', async original => ({ ...await original<typeof import('./studyMaterial')>(), loadStudyMaterial: mocks.material }))
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
  storage = await import('./studyStorage')
  owner = await import('./studyWorkspaceStore')
})

afterEach(() => {
  owner.resetStudyWorkspaceForTests()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

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
    return save(document)
  })
  return { entered: entered.promise, release: () => release.resolve() }
}

describe('Study material and draft persistence', () => {
  it('does not write or advance a clean material merely when its conversation opens', async () => {
    const [document] = await seed()
    const save = vi.spyOn(storage, 'saveStudyDocument')
    const revision = current(document).revision
    await owner.flushStudyDocument(document.id)
    expect(save).not.toHaveBeenCalled()
    expect((await storage.loadStudyWorkspace()).documents[0].revision).toBe(revision)
    owner.editStudyPage(document.id, 1, { notes: 'A real edit still persists' })
    await saved()
    expect(save).toHaveBeenCalledTimes(1)
    expect((await storage.loadStudyWorkspace()).documents[0].pages['1'].notes).toBe('A real edit still persists')
  })

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

  it('locks out edits before waiting for a deletion and never resurrects the document', async () => {
    const [first, second] = await seed(2)
    await question(first)
    const save = holdNextSave()
    owner.editStudyPage(first.id, 1, { notes: 'Pending notes' })
    await save.entered
    const removing = owner.removeStudyDocument(first.id)
    await expect(owner.reloadStudyWorkspace()).rejects.toThrow('Finish the import or removal')
    owner.editStudyPage(first.id, 1, { notes: 'Must not start another save' })
    expect(current(first).pages['1'].notes).toBe('Pending notes')
    save.release()
    await removing
    await saved()
    expect(owner.studyWorkspace.getSnapshot()).toMatchObject({ selectedDocumentId: second.id, saveError: '' })
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

  it('keeps conflicting local drafts dirty while protecting the external saved version from navigation and retries', async () => {
    const [document] = await seed()
    const external = (await storage.loadStudyWorkspace()).documents[0]
    external.pages['1'] = { ...storage.createEmptyStudyPage(), notes: 'Notes from another window' }
    external.revision = await storage.saveStudyDocument(external)
    owner.openStudyPage(document.id, 2)
    await vi.waitFor(() => expect(owner.studyWorkspace.getSnapshot().saveError).toContain('another Kivio window'))
    owner.editStudyPage(document.id, 2, { notes: 'Keep my unsaved local work' })
    owner.retryStudySave()
    await vi.waitFor(() => expect(owner.studyWorkspace.getSnapshot().dirtyIds).toContain(document.id))
    expect(current(document)).toMatchObject({ revision: 0, lastPage: 2 })
    expect(current(document).pages['2'].notes).toBe('Keep my unsaved local work')
    expect((await storage.loadStudyWorkspace()).documents[0]).toEqual(external)
    await owner.reloadStudyWorkspace()
    expect(current(document)).toEqual(external)
    expect(owner.studyWorkspace.getSnapshot()).toMatchObject({ dirtyIds: [], saveError: '' })
  })

  it('does not discard a conflicting draft when reloading the saved version fails', async () => {
    const [document] = await seed()
    const external = (await storage.loadStudyWorkspace()).documents[0]
    external.revision = await storage.saveStudyDocument(external)
    owner.editStudyPage(document.id, 1, { notes: 'A local draft to preserve' })
    await vi.waitFor(() => expect(owner.studyWorkspace.getSnapshot().saveError).toContain('another Kivio window'))
    const draft = current(document)
    vi.spyOn(storage, 'loadStudyWorkspace').mockRejectedValueOnce(new Error('Read temporarily failed'))
    await expect(owner.reloadStudyWorkspace()).rejects.toThrow('Read temporarily failed')
    expect(current(document)).toBe(draft)
    expect(owner.studyWorkspace.getSnapshot().dirtyIds).toContain(document.id)
    expect(owner.studyWorkspace.getSnapshot().saveError).toContain('another Kivio window')
  })

  it('keeps local work when a reload returns damaged or incomplete saved records', async () => {
    const [document] = await seed()
    await question(document, 'Preserve the local question')
    const draft = current(document)
    vi.spyOn(storage, 'loadStudyWorkspace').mockResolvedValueOnce({ documents: [], selectedDocumentId: null, warnings: ['The saved material is damaged.'] })
    await expect(owner.reloadStudyWorkspace()).rejects.toThrow('Your local drafts remain unchanged')
    expect(current(document)).toBe(draft)
  })

  it('does not discard edits made after the user confirmed a pending reload', async () => {
    const [document] = await seed()
    const loaded = deferred()
    const entered = deferred()
    const load = storage.loadStudyWorkspace
    vi.spyOn(storage, 'loadStudyWorkspace').mockImplementationOnce(async () => { entered.resolve(); await loaded.promise; return load() })
    const reloading = owner.reloadStudyWorkspace()
    await entered.promise
    owner.editStudyPage(document.id, 1, { notes: 'Typed after reload confirmation' })
    loaded.resolve()
    await expect(reloading).rejects.toThrow('Nothing was discarded')
    await saved()
    expect(current(document).pages['1'].notes).toBe('Typed after reload confirmation')
  })

  it('refuses reload while an import is active', async () => {
    await owner.initializeStudy()
    const parse = deferred<{ kind: 'pdf'; pageCount: number }>()
    mocks.material.mockReturnValueOnce(parse.promise)
    const importing = owner.importStudyFile(new File(['pending'], 'pending.pdf'))
    await vi.waitFor(() => expect(mocks.material).toHaveBeenCalled())
    await expect(owner.reloadStudyWorkspace()).rejects.toThrow('Finish the import or removal')
    owner.cancelStudyImport()
    parse.resolve({ kind: 'pdf', pageCount: 3 })
    await importing
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
