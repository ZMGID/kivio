import { IDBFactory, IDBObjectStore } from 'fake-indexeddb'
import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest'
import type { StudyDocument, StudyTurn } from './studyStorage'

let storage: typeof import('./studyStorage')

beforeEach(async () => {
  vi.resetModules()
  vi.stubGlobal('indexedDB', new IDBFactory())
  storage = await import('./studyStorage')
})

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const pdf = (bytes = 'example PDF contents') => new Blob([bytes], { type: 'application/pdf' })

function importPdf(name = 'lecture.pdf', bytes = 'example PDF contents') {
  return storage.importStudyDocument({ name, blob: pdf(bytes), kind: 'pdf', pageCount: 3 })
}

function turn(overrides: Partial<StudyTurn> = {}): StudyTurn {
  return {
    id: 'turn-1', mode: 'hint', question: 'Why?', attempt: 'My attempt',
    sourceText: 'The corrected source actually sent', sourceImageUsed: true,
    sourceWarning: 'This page is scanned', answer: 'First, look at the equation.',
    status: 'complete', createdAt: 123, providerId: 'provider-1', model: 'vision-model', page: 2,
    region: { x: 0.1, y: 0.2, width: 0.3, height: 0.4 }, ...overrides,
  }
}

function addWork(document: StudyDocument): StudyDocument {
  document.lastPage = 2
  document.pages['2'] = {
    question: 'Explain the equation', attempt: 'My initial answer', correctedText: 'x = 2',
    notes: 'Review this before the exam', region: { x: 0.1, y: 0.2, width: 0.3, height: 0.4 },
    history: [turn()],
  }
  return document
}

async function rawDatabase(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const opening = indexedDB.open(storage.STUDY_DATABASE_NAME, 1)
    opening.onsuccess = () => resolve(opening.result)
    opening.onerror = () => reject(opening.error)
  })
}

async function rawRead(store: string, id: string): Promise<unknown> {
  const db = await rawDatabase()
  try {
    return await new Promise((resolve, reject) => {
      const request = db.transaction(store).objectStore(store).get(id)
      request.onsuccess = () => resolve(request.result)
      request.onerror = () => reject(request.error)
    })
  } finally { db.close() }
}

async function rawWrite(store: string, value: unknown, removeId?: string): Promise<void> {
  const db = await rawDatabase()
  try {
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(store, 'readwrite')
      if (removeId) tx.objectStore(store).delete(removeId)
      else tx.objectStore(store).put(value)
      tx.oncomplete = () => resolve()
      tx.onabort = () => reject(tx.error)
    })
  } finally { db.close() }
}

function failNextMetaWrite() {
  const put = IDBObjectStore.prototype.put
  return vi.spyOn(IDBObjectStore.prototype, 'put').mockImplementation(function (this: IDBObjectStore, value, key) {
    if (this.name === 'meta') throw new DOMException('Quota exhausted', 'QuotaExceededError')
    return put.call(this, value, key)
  })
}

describe('Study local persistence', () => {
  it('starts with an empty library and independent empty page drafts', async () => {
    expect(await storage.loadStudyWorkspace()).toEqual({ documents: [], selectedDocumentId: null, warnings: [] })
    const page = storage.createEmptyStudyPage()
    page.notes = 'Changed'
    page.history.push(turn())
    expect(storage.createEmptyStudyPage()).toEqual({ question: '', attempt: '', correctedText: '', notes: '', history: [] })
  })

  it('imports original binary data, selects it, and identifies it with the SHA-256 content hash', async () => {
    const { document, duplicate } = await importPdf()
    const expectedHash = await crypto.subtle.digest('SHA-256', await pdf().arrayBuffer())
    expect(document.id).toBe(Array.from(new Uint8Array(expectedHash), (byte) => byte.toString(16).padStart(2, '0')).join(''))
    expect(duplicate).toBe(false)
    expect(await (await storage.readStudyDocumentBlob(document.id)).text()).toBe('example PDF contents')
    expect(await storage.loadStudyWorkspace()).toEqual({ documents: [document], selectedDocumentId: document.id, warnings: [] })
  })

  it('owns the import input before asynchronous hashing begins', async () => {
    const input = { name: 'original.pdf', kind: 'pdf' as const, pageCount: 3, blob: pdf('original') }
    const importing = storage.importStudyDocument(input)
    input.name = 'changed.pdf'
    input.blob = pdf('different bytes')
    const { document } = await importing
    expect(document.name).toBe('original.pdf')
    expect(await (await storage.readStudyDocumentBlob(document.id)).text()).toBe('original')
  })

  it('deduplicates identical bytes even under a new filename without replacing existing work', async () => {
    const { document } = await importPdf()
    addWork(document)
    document.revision = await storage.saveStudyDocument(document)
    const imported = await importPdf('renamed.pdf')
    expect(imported).toEqual({ document, duplicate: true })
    expect((await storage.loadStudyWorkspace()).documents).toEqual([document])
  })

  it('keeps files with the same filename and different contents separate', async () => {
    const first = await importPdf('same.pdf', 'version one')
    const second = await importPdf('same.pdf', 'version two')
    expect(second.document.id).not.toBe(first.document.id)
    expect((await storage.loadStudyWorkspace()).documents).toHaveLength(2)
  })

  it('atomically deduplicates simultaneous imports', async () => {
    const imported = await Promise.all([importPdf(), importPdf()])
    expect(imported.map((item) => item.duplicate).sort()).toEqual([false, true])
    expect((await storage.loadStudyWorkspace()).documents).toHaveLength(1)
    expect((await storage.loadStudyWorkspace()).warnings).toEqual([])
  })

  it('restores per-page drafts, region, notes, request snapshot, and selected material after reopening', async () => {
    const first = await importPdf()
    const second = await importPdf('second.pdf', 'second file')
    addWork(first.document)
    first.document.pages['1'] = { ...storage.createEmptyStudyPage(), notes: 'Different page notes' }
    first.document.revision = await storage.saveStudyDocument(first.document)
    await storage.setSelectedStudyDocument(first.document.id)
    vi.resetModules()
    storage = await import('./studyStorage')
    const restored = await storage.loadStudyWorkspace()
    expect(restored.selectedDocumentId).toBe(first.document.id)
    expect(restored.documents.find((item) => item.id === first.document.id)).toEqual(first.document)
    expect(restored.documents.find((item) => item.id === second.document.id)?.pages).toEqual({})
    expect(restored.warnings).toEqual([])
  })

  it('persists an explicitly cleared region from the reader', async () => {
    const { document } = await importPdf()
    addWork(document)
    document.pages['2'].region = null
    document.pages['2'].history[0].region = null
    document.revision = await storage.saveStudyDocument(document)
    expect((await storage.loadStudyWorkspace()).documents[0].pages['2'].region).toBeNull()
    expect((await storage.loadStudyWorkspace()).documents[0].pages['2'].history[0].region).toBeNull()
  })

  it('saves and reloads reading turns alongside every legacy math mode without changing their schema', async () => {
    const { document } = await importPdf('journal.pdf')
    addWork(document)
    document.pages['2'].history = (['read', 'hint', 'explain', 'check', 'solution'] as const).map((mode, index) => turn({
      id: `mode-${index}`, mode, question: mode === 'read' ? 'Translate this paragraph.' : 'Why?',
      attempt: mode === 'read' ? '' : 'My attempt', sourceText: mode === 'read' ? '' : 'Legacy saved source',
    }))
    document.revision = await storage.saveStudyDocument(document)
    vi.resetModules()
    storage = await import('./studyStorage')
    const restored = await storage.loadStudyWorkspace()
    expect(restored.warnings).toEqual([])
    expect(restored.documents[0]).toEqual(document)
    expect(restored.documents[0].pages['2'].history.map(turn => turn.mode)).toEqual(['read', 'hint', 'explain', 'check', 'solution'])
  })

  it('preserves unfinished legacy history on load, duplicate import and unrelated draft saves', async () => {
    const { document } = await importPdf()
    addWork(document)
    document.pages['2'].history[0].status = 'streaming'
    document.revision = await storage.saveStudyDocument(document)
    const original = await rawRead('documents', document.id) as { document: StudyDocument }
    const history = JSON.stringify(original.document.pages['2'].history)
    const restored = (await storage.loadStudyWorkspace()).documents[0]
    expect(restored.pages['2'].history[0]).toMatchObject({ status: 'streaming', answer: 'First, look at the equation.' })
    expect(restored.pages['2'].history[0].error).toBeUndefined()
    const duplicate = await importPdf()
    expect(duplicate.duplicate).toBe(true)
    expect(JSON.stringify(duplicate.document.pages['2'].history)).toBe(history)
    restored.pages['2'].notes = 'A new note beside the unchanged archive'
    restored.revision = await storage.saveStudyDocument(restored)
    expect((await storage.loadStudyWorkspace()).warnings).toEqual([])
    expect(JSON.stringify((await storage.loadStudyWorkspace()).documents[0].pages['2'].history)).toBe(history)
    const saved = await rawRead('documents', document.id) as { document: StudyDocument }
    expect(JSON.stringify(saved.document.pages['2'].history)).toBe(history)
  })

  it.each(['complete', 'error', 'cancelled', 'interrupted'] as const)('retains a %s terminal response when restored', async (status) => {
    const { document } = await importPdf()
    addWork(document)
    document.pages['2'].history[0].status = status
    document.revision = await storage.saveStudyDocument(document)
    expect((await storage.loadStudyWorkspace()).documents[0].pages['2'].history[0].status).toBe(status)
  })

  it('snapshots saves before awaiting storage and rejects concurrent stale snapshots', async () => {
    const { document } = await importPdf()
    addWork(document)
    const saves = []
    for (const notes of ['first edit', 'second edit', 'latest edit']) {
      document.pages['2'].notes = notes
      saves.push(storage.saveStudyDocument(document))
    }
    document.pages['2'].notes = 'not submitted'
    const results = await Promise.allSettled(saves)
    expect(results[0]).toMatchObject({ status: 'fulfilled', value: 1 })
    expect(results.slice(1)).toEqual([
      { status: 'rejected', reason: expect.objectContaining({ code: 'conflict' }) },
      { status: 'rejected', reason: expect.objectContaining({ code: 'conflict' }) },
    ])
    expect((await storage.loadStudyWorkspace()).documents[0].pages['2'].notes).toBe('first edit')
  })

  it('uses persisted CAS across independently loaded modules so stale navigation cannot erase another window’s notes', async () => {
    await importPdf()
    const first = (await storage.loadStudyWorkspace()).documents[0]
    vi.resetModules()
    const otherStorage = await import('./studyStorage')
    const stale = (await otherStorage.loadStudyWorkspace()).documents[0]
    first.pages['1'] = { ...storage.createEmptyStudyPage(), notes: 'Saved in the first window' }
    expect(await storage.saveStudyDocument(first)).toBe(1)
    stale.lastPage = 2
    await expect(otherStorage.saveStudyDocument(stale)).rejects.toMatchObject({ code: 'conflict' })
    const durable = (await otherStorage.loadStudyWorkspace()).documents[0]
    expect(durable.revision).toBe(1)
    expect(durable.lastPage).toBe(1)
    expect(durable.pages['1'].notes).toBe('Saved in the first window')
    expect(stale.revision).toBe(0)
    expect(stale.lastPage).toBe(2)
    expect(stale.pages).toEqual({})
  })

  it('keeps both the durable version and a conflicting caller’s draft unchanged on retries', async () => {
    await importPdf()
    const first = (await storage.loadStudyWorkspace()).documents[0]
    const second = (await storage.loadStudyWorkspace()).documents[0]
    first.pages['1'] = { ...storage.createEmptyStudyPage(), notes: 'External saved notes' }
    second.pages['2'] = { ...storage.createEmptyStudyPage(), notes: 'Unsaved local notes' }
    first.revision = await storage.saveStudyDocument(first)
    for (let attempt = 0; attempt < 2; attempt++) await expect(storage.saveStudyDocument(second)).rejects.toMatchObject({ code: 'conflict' })
    expect((await storage.loadStudyWorkspace()).documents[0]).toEqual(first)
    expect(second.pages['2'].notes).toBe('Unsaved local notes')
    expect(second.revision).toBe(0)
  })

  it('loads legacy records at revision zero and upgrades their metadata accounting on the first successful save', async () => {
    const { document } = await importPdf()
    const record = await rawRead('documents', document.id) as { document: Partial<StudyDocument>; metadataBytes: number; id: string }
    const originalBytes = record.metadataBytes
    delete record.document.revision
    record.metadataBytes = new TextEncoder().encode(JSON.stringify(record.document)).byteLength
    await rawWrite('documents', record)
    const workspace = await rawRead('meta', 'workspace') as { totalBytes: number }
    await rawWrite('meta', { ...workspace, totalBytes: workspace.totalBytes - originalBytes + record.metadataBytes })
    const restored = await storage.loadStudyWorkspace()
    expect(restored.warnings).toEqual([])
    expect(restored.documents[0].revision).toBe(0)
    restored.documents[0].pages['1'] = { ...storage.createEmptyStudyPage(), notes: 'Legacy work is retained' }
    expect(await storage.saveStudyDocument(restored.documents[0])).toBe(1)
    const upgraded = await storage.loadStudyWorkspace()
    expect(upgraded.warnings).toEqual([])
    expect(upgraded.documents[0].revision).toBe(1)
    expect(upgraded.documents[0].pages['1'].notes).toBe('Legacy work is retained')
  })

  it('updates only the affected document and never rewrites binary blobs on a note save', async () => {
    const first = await importPdf()
    await importPdf('other.pdf', 'other contents')
    const original = IDBObjectStore.prototype.put
    const writes: string[] = []
    vi.spyOn(IDBObjectStore.prototype, 'put').mockImplementation(function (this: IDBObjectStore, value, key) {
      writes.push(this.name)
      return original.call(this, value, key)
    })
    await storage.saveStudyDocument(addWork(first.document))
    expect(writes).toEqual(['documents', 'meta'])
  })

  it('rolls back both document and blob if quota prevents import completion', async () => {
    await storage.loadStudyWorkspace()
    const quota = failNextMetaWrite()
    await expect(importPdf()).rejects.toMatchObject({ code: 'quota' })
    quota.mockRestore()
    expect(await storage.loadStudyWorkspace()).toEqual({ documents: [], selectedDocumentId: null, warnings: [] })
    const db = await rawDatabase()
    const count = await new Promise<number>((resolve) => {
      const read = db.transaction('materials').objectStore('materials').count()
      read.onsuccess = () => resolve(read.result)
    })
    db.close()
    expect(count).toBe(0)
  })

  it('rolls back a failed save and permits a successful retry without pretending the edits were saved', async () => {
    const { document } = await importPdf()
    const quota = failNextMetaWrite()
    await expect(storage.saveStudyDocument(addWork(document))).rejects.toMatchObject({ code: 'quota' })
    quota.mockRestore()
    expect((await storage.loadStudyWorkspace()).documents[0].pages).toEqual({})
    document.revision = await storage.saveStudyDocument(document)
    expect((await storage.loadStudyWorkspace()).documents[0]).toEqual(document)
  })

  it('rolls back a failed delete, including its original file and selection', async () => {
    const { document } = await importPdf()
    const quota = failNextMetaWrite()
    await expect(storage.deleteStudyDocument(document.id)).rejects.toMatchObject({ code: 'quota' })
    quota.mockRestore()
    expect((await storage.loadStudyWorkspace()).selectedDocumentId).toBe(document.id)
    expect(await (await storage.readStudyDocumentBlob(document.id)).text()).toBe('example PDF contents')
  })

  it('removes only the confirmed material and its work, and rejects delayed writes instead of resurrecting it', async () => {
    const first = await importPdf()
    const second = await importPdf('keep.pdf', 'keep contents')
    await storage.setSelectedStudyDocument(first.document.id)
    await storage.deleteStudyDocument(first.document.id)
    const workspace = await storage.loadStudyWorkspace()
    expect(workspace).toEqual({ documents: [second.document], selectedDocumentId: null, warnings: [] })
    await expect(storage.readStudyDocumentBlob(first.document.id)).rejects.toMatchObject({ code: 'not-found' })
    await expect(storage.saveStudyDocument(addWork(first.document))).rejects.toMatchObject({ code: 'not-found' })
    expect(await rawRead('materials', first.document.id)).toBeUndefined()
  })

  it('rejects a stale material selection without changing the saved selection', async () => {
    const { document } = await importPdf()
    await expect(storage.setSelectedStudyDocument('missing')).rejects.toMatchObject({ code: 'not-found' })
    expect((await storage.loadStudyWorkspace()).selectedDocumentId).toBe(document.id)
  })

  it('rejects a pre-cancelled import without creating a saved material', async () => {
    const controller = new AbortController()
    controller.abort()
    await expect(storage.importStudyDocument({ name: 'cancel.pdf', kind: 'pdf', pageCount: 1, blob: pdf(), signal: controller.signal })).rejects.toMatchObject({ code: 'cancelled' })
    expect((await storage.loadStudyWorkspace()).documents).toEqual([])
  })

  it('rolls back original bytes, metadata and selection when import is cancelled during its write transaction', async () => {
    const first = await importPdf()
    const controller = new AbortController()
    const add = IDBObjectStore.prototype.add
    const intercept = vi.spyOn(IDBObjectStore.prototype, 'add').mockImplementation(function (this: IDBObjectStore, value, key) {
      const request = add.call(this, value, key)
      if (this.name === 'materials') controller.abort()
      return request
    })
    await expect(storage.importStudyDocument({ name: 'cancel.pdf', kind: 'pdf', pageCount: 1, blob: pdf('cancelled contents'), signal: controller.signal })).rejects.toMatchObject({ code: 'cancelled' })
    intercept.mockRestore()
    expect(await storage.loadStudyWorkspace()).toEqual({ documents: [first.document], selectedDocumentId: first.document.id, warnings: [] })
  })

  it('rejects empty and oversized files before hashing or saving them', async () => {
    await expect(storage.importStudyDocument({ name: 'empty.pdf', kind: 'pdf', pageCount: 1, blob: new Blob() })).rejects.toMatchObject({ code: 'invalid' })
    const oversized = new Blob([new Uint8Array(storage.STUDY_LIMITS.maxFileBytes + 1)])
    await expect(storage.importStudyDocument({ name: 'large.pdf', kind: 'pdf', pageCount: 1, blob: oversized })).rejects.toMatchObject({ code: 'limit' })
    expect((await storage.loadStudyWorkspace()).documents).toEqual([])
  })

  it('enforces the document limit while allowing duplicate reimports and freeing capacity after deletion', async () => {
    for (let index = 0; index < storage.STUDY_LIMITS.maxDocuments; index++) await importPdf(`${index}.pdf`, `file ${index}`)
    await expect(importPdf('101.pdf', 'new contents')).rejects.toMatchObject({ code: 'limit' })
    const duplicate = await importPdf('renamed.pdf', 'file 0')
    expect(duplicate.duplicate).toBe(true)
    await storage.deleteStudyDocument(duplicate.document.id)
    await expect(importPdf('replacement.pdf', 'new contents')).resolves.toMatchObject({ duplicate: false })
    expect((await storage.loadStudyWorkspace()).warnings).toEqual([])
  })

  it('checks total capacity for both imports and note growth without silently truncating drafts', async () => {
    const { document } = await importPdf()
    const workspace = await rawRead('meta', 'workspace') as Record<string, unknown>
    // Stand in for capacity occupied by other documents; the boundary uses the atomic library counter.
    await rawWrite('meta', { ...workspace, totalBytes: storage.STUDY_LIMITS.maxTotalBytes - 1 })
    await expect(importPdf('extra.pdf', 'new contents')).rejects.toMatchObject({ code: 'limit' })
    addWork(document)
    await expect(storage.saveStudyDocument(document)).rejects.toMatchObject({ code: 'limit' })
    const stored = await rawRead('documents', document.id) as { document: StudyDocument }
    expect(stored.document.pages).toEqual({})
  })

  it('bounds per-material metadata without truncating notes or request history', async () => {
    const { document } = await importPdf()
    for (let page = 1; page <= 3; page++) {
      document.pages[String(page)] = {
        ...storage.createEmptyStudyPage(),
        question: 'q'.repeat(storage.STUDY_LIMITS.maxTextLength),
        attempt: 'a'.repeat(storage.STUDY_LIMITS.maxTextLength),
        notes: 'n'.repeat(storage.STUDY_LIMITS.maxTextLength),
      }
    }
    await expect(storage.saveStudyDocument(document)).rejects.toMatchObject({ code: 'limit' })
    expect((await storage.loadStudyWorkspace()).documents[0].pages).toEqual({})
  })

  it.each([
    (document: StudyDocument) => { document.lastPage = 0 },
    (document: StudyDocument) => { document.pages['4'] = storage.createEmptyStudyPage() },
    (document: StudyDocument) => { document.pages['2'].notes = 'x'.repeat(storage.STUDY_LIMITS.maxTextLength + 1) },
    (document: StudyDocument) => { document.pages['2'].region = { x: 0.9, y: 0, width: 0.5, height: 1 } },
    (document: StudyDocument) => { document.pages['2'].history[0].page = 1 },
    (document: StudyDocument) => { document.pages['2'].history[0].mode = 'unknown' as StudyTurn['mode'] },
    (document: StudyDocument) => { document.pages['2'].history.push(turn()) },
    (document: StudyDocument) => { document.pageCount = 4 },
  ])('rejects invalid or mismatched work without overwriting the valid stored document', async (corrupt) => {
    const { document } = await importPdf()
    const baseline = structuredClone(document)
    addWork(document)
    corrupt(document)
    await expect(storage.saveStudyDocument(document)).rejects.toMatchObject({ code: 'invalid' })
    expect((await storage.loadStudyWorkspace()).documents).toEqual([baseline])
  })

  it('reports malformed stored documents but retains their raw data and keeps other documents usable', async () => {
    const first = await importPdf()
    const second = await importPdf('valid.pdf', 'valid document')
    const damaged = { id: first.document.id, document: { name: 'broken' }, metadataBytes: 10 }
    await rawWrite('documents', damaged)
    const restored = await storage.loadStudyWorkspace()
    expect(restored.documents).toEqual([second.document])
    expect(restored.warnings.join(' ')).toContain('damaged')
    expect(await rawRead('documents', first.document.id)).toEqual(damaged)
    await expect(storage.saveStudyDocument(first.document)).rejects.toMatchObject({ code: 'corrupt' })
    await expect(storage.setSelectedStudyDocument(first.document.id)).rejects.toMatchObject({ code: 'corrupt' })
  })

  it('reports missing original files and never marks their import or reload successful', async () => {
    const { document } = await importPdf()
    await rawWrite('materials', null, document.id)
    const restored = await storage.loadStudyWorkspace()
    expect(restored.documents).toEqual([])
    expect(restored.warnings.join(' ')).toContain('missing')
    await expect(storage.readStudyDocumentBlob(document.id)).rejects.toMatchObject({ code: 'corrupt' })
    await expect(importPdf()).rejects.toMatchObject({ code: 'corrupt' })
    expect(await rawRead('documents', document.id)).toBeDefined()
  })

  it('detects same-size binary corruption by checking the file content hash', async () => {
    const { document } = await importPdf('same.pdf', 'original')
    await rawWrite('materials', { id: document.id, blob: pdf('tampered') })
    await expect(storage.readStudyDocumentBlob(document.id)).rejects.toMatchObject({ code: 'corrupt' })
    await expect(importPdf('same.pdf', 'original')).rejects.toMatchObject({ code: 'corrupt' })
    expect(await rawRead('documents', document.id)).toBeDefined()
  })

  it('surfaces a damaged index and refuses writes instead of resetting stored work', async () => {
    const { document } = await importPdf()
    await rawWrite('meta', { key: 'workspace', totalBytes: -1, documentCount: 1, selectedDocumentId: document.id })
    expect((await storage.loadStudyWorkspace()).warnings.join(' ')).toContain('index is damaged')
    await expect(storage.saveStudyDocument(addWork(document))).rejects.toMatchObject({ code: 'corrupt' })
    await expect(importPdf('new.pdf', 'another file')).rejects.toMatchObject({ code: 'corrupt' })
    expect((await storage.loadStudyWorkspace()).documents[0].pages).toEqual({})
  })

  it('rejects inconsistent counts and impossible accounting before mutating saved work', async () => {
    const { document } = await importPdf()
    const original = await rawRead('meta', 'workspace') as Record<string, unknown>
    await rawWrite('meta', { ...original, documentCount: 0 })
    await expect(importPdf('new.pdf', 'new contents')).rejects.toMatchObject({ code: 'corrupt' })
    await rawWrite('meta', { ...original, totalBytes: 1 })
    await expect(storage.saveStudyDocument(addWork(document))).rejects.toMatchObject({ code: 'corrupt' })
    await expect(storage.deleteStudyDocument(document.id)).rejects.toMatchObject({ code: 'corrupt' })
    expect(await rawRead('documents', document.id)).toBeDefined()
    expect(await rawRead('materials', document.id)).toBeDefined()
  })

  it('does not silently fall back to volatile memory when IndexedDB is unavailable', async () => {
    vi.stubGlobal('indexedDB', undefined)
    await expect(storage.loadStudyWorkspace()).rejects.toMatchObject({ code: 'unavailable' })
    await expect(importPdf()).rejects.toMatchObject({ code: 'unavailable' })
  })
})
