import type { StudyRegion } from './studyMaterial'

export const STUDY_DATABASE_NAME = 'kivio-study'
export const STUDY_LIMITS = {
  maxFileBytes: 25 * 1024 * 1024,
  maxTotalBytes: 150 * 1024 * 1024,
  maxDocuments: 100,
  maxDocumentMetadataBytes: 4 * 1024 * 1024,
  maxTextLength: 500_000,
  maxHistoryPerPage: 500,
  maxPages: 10_000,
} as const

const STUDY_MODES = ['read', 'hint', 'explain', 'check', 'solution'] as const

export interface StudyTurn {
  id: string
  mode: typeof STUDY_MODES[number]
  question: string
  attempt: string
  sourceText: string
  sourceImageUsed?: boolean
  sourceWarning?: string
  region?: StudyRegion | null
  answer: string
  status: 'streaming' | 'complete' | 'error' | 'cancelled' | 'interrupted'
  error?: string
  createdAt: number
  providerId: string
  model: string
  page: number
}

export interface StudyPageState {
  question: string
  attempt: string
  correctedText: string
  notes: string
  region?: StudyRegion | null
  history: StudyTurn[]
}

export interface StudyDocument {
  /** SHA-256 of the imported bytes; filenames are not identities. */
  id: string
  /** Optimistic concurrency token. Only a successful save advances this value. */
  revision: number
  name: string
  kind: 'pdf' | 'image'
  pageCount: number
  createdAt: number
  updatedAt: number
  size: number
  lastPage: number
  pages: Record<string, StudyPageState>
}

export interface StudyWorkspace {
  documents: StudyDocument[]
  selectedDocumentId: string | null
  /** Invalid records are retained in storage, never silently discarded or replaced. */
  warnings: string[]
}

type StorageErrorCode = 'unavailable' | 'blocked' | 'invalid' | 'corrupt' | 'quota' | 'limit' | 'not-found' | 'storage' | 'cancelled' | 'conflict'

export class StudyStorageError extends Error {
  constructor(readonly code: StorageErrorCode, message: string, readonly cause?: unknown) {
    super(message)
    this.name = 'StudyStorageError'
  }
}

interface DocumentRecord {
  id: string
  document: StudyDocument
  metadataBytes: number
}

interface WorkspaceRecord {
  key: 'workspace'
  selectedDocumentId: string | null
  totalBytes: number
  documentCount: number
}

const DOCUMENTS = 'documents'
const MATERIALS = 'materials'
const META = 'meta'
const HASH = /^[a-f0-9]{64}$/
let databasePromise: Promise<IDBDatabase> | undefined

export function createEmptyStudyPage(): StudyPageState {
  return { question: '', attempt: '', correctedText: '', notes: '', history: [] }
}

function failure(error: unknown): StudyStorageError {
  if (error instanceof StudyStorageError) return error
  if (object(error)?.name === 'AbortError') return new StudyStorageError('cancelled', 'The Study operation was cancelled.', error)
  if (object(error)?.name === 'QuotaExceededError') {
    return new StudyStorageError('quota', 'Local storage is full. Your latest changes were not saved. Free some space and retry.', error)
  }
  return new StudyStorageError('storage', 'Could not access Study storage. Your latest changes may not be saved. Retry before leaving.', error)
}

function invalid(message: string): never {
  throw new StudyStorageError('invalid', message)
}

function object(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : null
}

function string(value: unknown, label: string, max: number = STUDY_LIMITS.maxTextLength): string {
  if (typeof value !== 'string' || value.length > max) invalid(`${label} is invalid or too long. Nothing was saved.`)
  return value
}

function integer(value: unknown, label: string, min: number, max = Number.MAX_SAFE_INTEGER): number {
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < min || value > max) invalid(`${label} is invalid. Nothing was saved.`)
  return value
}

function region(value: unknown): StudyRegion | null | undefined {
  if (value === undefined || value === null) return value
  const candidate = object(value)
  if (!candidate) invalid('The selected region is invalid. Nothing was saved.')
  const { x, y, width, height } = candidate
  if ([x, y, width, height].some((coordinate) => typeof coordinate !== 'number' || !Number.isFinite(coordinate))) invalid('The selected region is invalid. Nothing was saved.')
  const result = { x, y, width, height } as StudyRegion
  if (result.x < 0 || result.y < 0 || result.width <= 0 || result.height <= 0 || result.x + result.width > 1.000001 || result.y + result.height > 1.000001) invalid('The selected region is outside this page. Nothing was saved.')
  return result
}

/** Makes an owned, schema-checked snapshot before any asynchronous save work. */
function snapshotDocument(value: unknown): StudyDocument {
  const source = object(value)
  if (!source || typeof source.id !== 'string' || !HASH.test(source.id)) invalid('This material has an invalid identity. Nothing was saved.')
  if (source.kind !== 'pdf' && source.kind !== 'image') invalid('This material has an invalid type. Nothing was saved.')
  const pageCount = integer(source.pageCount, 'Page count', 1, STUDY_LIMITS.maxPages)
  if (source.kind === 'image' && pageCount !== 1) invalid('An image must have exactly one page. Nothing was saved.')
  const sourcePages = object(source.pages)
  if (!sourcePages) invalid('The saved pages are invalid. Nothing was saved.')
  const pages: Record<string, StudyPageState> = {}
  const turnIds = new Set<string>()
  for (const [key, value] of Object.entries(sourcePages)) {
    const pageNumber = Number(key)
    if (String(pageNumber) !== key) invalid('The saved page number is invalid. Nothing was saved.')
    integer(pageNumber, 'Saved page number', 1, pageCount)
    const page = object(value)
    if (!page || !Array.isArray(page.history) || page.history.length > STUDY_LIMITS.maxHistoryPerPage) invalid('The page history is invalid or full. Nothing was saved.')
    const history = page.history.map((value): StudyTurn => {
      const turn = object(value)
      if (!turn || !['streaming', 'complete', 'error', 'cancelled', 'interrupted'].includes(String(turn.status))) invalid('A saved response has an invalid status. Nothing was saved.')
      const id = string(turn.id, 'Response ID', 200)
      if (!id || turnIds.has(id)) invalid('Response IDs must be unique. Nothing was saved.')
      turnIds.add(id)
      if (integer(turn.page, 'Response page', 1, pageCount) !== pageNumber) invalid('The response belongs to another page. Nothing was saved.')
      if (!STUDY_MODES.some(mode => mode === turn.mode)) invalid('The Study mode is invalid. Nothing was saved.')
      if (turn.sourceImageUsed !== undefined && typeof turn.sourceImageUsed !== 'boolean') invalid('The source image flag is invalid. Nothing was saved.')
      return {
        id,
        mode: turn.mode as StudyTurn['mode'],
        question: string(turn.question, 'Question'),
        attempt: string(turn.attempt, 'Attempt'),
        sourceText: string(turn.sourceText, 'Source text'),
        ...(turn.sourceImageUsed === undefined ? {} : { sourceImageUsed: turn.sourceImageUsed }),
        ...(turn.sourceWarning === undefined ? {} : { sourceWarning: string(turn.sourceWarning, 'Source warning') }),
        region: region(turn.region),
        answer: string(turn.answer, 'Answer'),
        status: turn.status as StudyTurn['status'],
        ...(turn.error === undefined ? {} : { error: string(turn.error, 'Response error') }),
        createdAt: integer(turn.createdAt, 'Response date', 0),
        providerId: string(turn.providerId, 'Provider', 200),
        model: string(turn.model, 'Model', 500),
        page: pageNumber,
      }
    })
    pages[key] = {
      question: string(page.question, 'Question'),
      attempt: string(page.attempt, 'Attempt'),
      correctedText: string(page.correctedText, 'Corrected text'),
      notes: string(page.notes, 'Notes'),
      region: region(page.region),
      history,
    }
  }
  const name = string(source.name, 'Material name', 1024)
  if (!name.trim()) invalid('A material name is required. Nothing was saved.')
  return {
    id: source.id, revision: integer(source.revision === undefined ? 0 : source.revision, 'Material revision', 0), name, kind: source.kind, pageCount,
    createdAt: integer(source.createdAt, 'Import date', 0),
    updatedAt: integer(source.updatedAt, 'Update date', 0),
    size: integer(source.size, 'Material size', 1, STUDY_LIMITS.maxFileBytes),
    lastPage: integer(source.lastPage, 'Current page', 1, pageCount),
    pages,
  }
}

function documentRecord(document: StudyDocument): DocumentRecord {
  const metadataBytes = new TextEncoder().encode(JSON.stringify(document)).byteLength
  if (metadataBytes > STUDY_LIMITS.maxDocumentMetadataBytes) throw new StudyStorageError('limit', 'This material’s notes and history exceed the 4 MiB local limit. The latest changes were not saved.')
  return { id: document.id, document, metadataBytes }
}

/** Pure preflight; runtime quota and revision conflicts are still checked by the atomic save. */
export function validateStudyDocument(value: StudyDocument, reservedMetadataBytes = 0): number {
  const record = documentRecord(snapshotDocument(value))
  if (record.metadataBytes + reservedMetadataBytes > STUDY_LIMITS.maxDocumentMetadataBytes) {
    throw new StudyStorageError('limit', 'This material has too little local space for another response. Shorten its notes or use another material; your current draft is unchanged.')
  }
  return STUDY_LIMITS.maxDocumentMetadataBytes - record.metadataBytes - reservedMetadataBytes
}

function storedDocument(value: unknown): DocumentRecord {
  try {
    const record = object(value)
    if (!record) invalid('Missing material record.')
    const document = snapshotDocument(record.document)
    const canonical = { id: document.id, document, metadataBytes: new TextEncoder().encode(JSON.stringify(document)).byteLength }
    // Version-one records predate CAS. Their byte accounting excluded the revision field.
    let metadataBytes = canonical.metadataBytes
    if (object(record.document)?.revision === undefined) {
      const legacy: Partial<StudyDocument> = { ...canonical.document }
      delete legacy.revision
      metadataBytes = new TextEncoder().encode(JSON.stringify(legacy)).byteLength
    }
    if (metadataBytes > STUDY_LIMITS.maxDocumentMetadataBytes || record.id !== canonical.id || record.metadataBytes !== metadataBytes) invalid('Invalid material record.')
    return { ...canonical, metadataBytes }
  } catch (error) {
    throw new StudyStorageError('corrupt', 'A saved Study material is damaged. It has been kept in local storage, but could not be loaded.', error)
  }
}

function request<T>(request: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result)
    request.onerror = () => reject(request.error)
  })
}

function database(): Promise<IDBDatabase> {
  if (!databasePromise) {
    databasePromise = new Promise<IDBDatabase>((resolve, reject) => {
      if (typeof indexedDB === 'undefined') {
        reject(new StudyStorageError('unavailable', 'Local Study storage is unavailable in this browser. Materials and notes cannot be saved.'))
        return
      }
      const opening = indexedDB.open(STUDY_DATABASE_NAME, 1)
      let blocked = false
      opening.onupgradeneeded = () => {
        const db = opening.result
        db.createObjectStore(DOCUMENTS, { keyPath: 'id' })
        db.createObjectStore(MATERIALS, { keyPath: 'id' })
        db.createObjectStore(META, { keyPath: 'key' })
      }
      opening.onblocked = () => {
        blocked = true
        reject(new StudyStorageError('blocked', 'Close other Kivio windows using Study, then retry opening your materials.'))
      }
      opening.onerror = () => reject(failure(opening.error))
      opening.onsuccess = () => {
        const db = opening.result
        if (blocked) { db.close(); return }
        db.onversionchange = () => { db.close(); databasePromise = undefined }
        db.onclose = () => { databasePromise = undefined }
        resolve(db)
      }
    }).catch((error: unknown) => { databasePromise = undefined; throw failure(error) })
  }
  return databasePromise
}

/** Never resolves a write until IndexedDB confirms the entire transaction committed. */
async function transaction<T>(stores: string[], mode: IDBTransactionMode, action: (transaction: IDBTransaction) => Promise<T>, signal?: AbortSignal): Promise<T> {
  try {
    const db = await database()
    if (signal?.aborted) throw new DOMException('Study operation cancelled', 'AbortError')
    const tx = db.transaction(stores, mode)
    const abort = () => { try { tx.abort() } catch { /* Already completed. */ } }
    signal?.addEventListener('abort', abort, { once: true })
    const completed = new Promise<void>((resolve, reject) => {
      tx.oncomplete = () => resolve()
      tx.onabort = () => reject(tx.error ?? new Error('Study transaction was aborted'))
    })
    // An abort can precede the final request's rejection; attach a handler immediately.
    void completed.catch(() => {})
    try {
      const result = await action(tx)
      await completed
      return result
    } catch (error) {
      try { tx.abort() } catch { /* It may already have aborted or completed. */ }
      await completed.catch(() => {})
      throw signal?.aborted ? new DOMException('Study operation cancelled', 'AbortError') : error
    } finally { signal?.removeEventListener('abort', abort) }
  } catch (error) { throw failure(error) }
}

async function workspaceRecord(tx: IDBTransaction): Promise<WorkspaceRecord> {
  const value: unknown = await request(tx.objectStore(META).get('workspace'))
  if (value === undefined) {
    const count = await request(tx.objectStore(DOCUMENTS).count())
    if (count !== 0) throw new StudyStorageError('corrupt', 'The Study library index is missing. Existing materials have not been changed.')
    return { key: 'workspace', selectedDocumentId: null, totalBytes: 0, documentCount: 0 }
  }
  const record = object(value)
  if (!record || record.key !== 'workspace' || (record.selectedDocumentId !== null && (typeof record.selectedDocumentId !== 'string' || !HASH.test(record.selectedDocumentId))) || !Number.isSafeInteger(record.totalBytes) || Number(record.totalBytes) < 0 || Number(record.totalBytes) > STUDY_LIMITS.maxTotalBytes || !Number.isSafeInteger(record.documentCount) || Number(record.documentCount) < 0 || Number(record.documentCount) > STUDY_LIMITS.maxDocuments) {
    throw new StudyStorageError('corrupt', 'The Study library index is damaged. Existing materials have not been changed.')
  }
  const count = await request(tx.objectStore(DOCUMENTS).count())
  if (record.documentCount !== count || (count === 0 ? record.totalBytes !== 0 : Number(record.totalBytes) < count)) {
    throw new StudyStorageError('corrupt', 'The Study library index is inconsistent. Existing materials have not been changed.')
  }
  return record as unknown as WorkspaceRecord
}

function checkCapacity(totalBytes: number, documentCount: number): void {
  if (documentCount > STUDY_LIMITS.maxDocuments) throw new StudyStorageError('limit', 'Study holds up to 100 materials locally. Remove a material before importing another.')
  if (totalBytes > STUDY_LIMITS.maxTotalBytes) throw new StudyStorageError('limit', 'Study’s 150 MiB local storage limit would be exceeded. Remove a material or shorten the latest notes, then retry.')
}

async function digest(blob: Blob): Promise<string> {
  if (!globalThis.crypto?.subtle) throw new StudyStorageError('unavailable', 'Secure file identification is unavailable. Open Kivio in a secure browser context to import materials.')
  try {
    const hash = await crypto.subtle.digest('SHA-256', await blob.arrayBuffer())
    return Array.from(new Uint8Array(hash), (byte) => byte.toString(16).padStart(2, '0')).join('')
  } catch (error) { throw failure(error) }
}

function materialBlob(value: unknown, document: StudyDocument): Blob {
  const record = object(value)
  if (!record || record.id !== document.id || !(record.blob instanceof Blob) || record.blob.size !== document.size) throw new StudyStorageError('corrupt', 'This material’s original file is missing or damaged. Your notes have been kept.')
  return record.blob
}

/** Import, content deduplication, capacity accounting, and selection are atomic. */
export async function importStudyDocument(input: { name: string; kind: StudyDocument['kind']; pageCount: number; blob: Blob; signal?: AbortSignal }): Promise<{ document: StudyDocument; duplicate: boolean }> {
  const { name, kind, pageCount, blob, signal } = input
  if (signal?.aborted) throw new StudyStorageError('cancelled', 'The Study import was cancelled.')
  if (!(blob instanceof Blob) || blob.size === 0) throw new StudyStorageError('invalid', 'Choose a non-empty PDF or image file.')
  if (blob.size > STUDY_LIMITS.maxFileBytes) throw new StudyStorageError('limit', 'Each Study material must be 25 MiB or smaller.')
  const id = await digest(blob)
  // Verify a duplicate's original bytes before making its selection visible as a success.
  // Hashing is deliberately outside an IndexedDB transaction, which cannot span crypto work.
  const existingBlob = await transaction([DOCUMENTS, MATERIALS], 'readonly', async (tx) => {
    const value: unknown = await request(tx.objectStore(DOCUMENTS).get(id))
    if (value === undefined) return null
    const saved = storedDocument(value).document
    return materialBlob(await request(tx.objectStore(MATERIALS).get(id)), saved)
  }, signal)
  if (existingBlob && await digest(existingBlob) !== id) throw new StudyStorageError('corrupt', 'This material’s saved file contents are damaged. Its notes have been kept.')
  const now = Date.now()
  const document = snapshotDocument({ id, name, kind, pageCount, createdAt: now, updatedAt: now, size: blob.size, lastPage: 1, pages: {} })
  const record = documentRecord(document)
  return transaction([DOCUMENTS, MATERIALS, META], 'readwrite', async (tx) => {
    const documents = tx.objectStore(DOCUMENTS)
    const existing: unknown = await request(documents.get(id))
    const workspace = await workspaceRecord(tx)
    if (existing !== undefined) {
      const saved = storedDocument(existing).document
      materialBlob(await request(tx.objectStore(MATERIALS).get(id)), saved)
      await request(tx.objectStore(META).put({ ...workspace, selectedDocumentId: id }))
      return { document: restoreInterrupted(saved), duplicate: true }
    }
    const totalBytes = workspace.totalBytes + document.size + record.metadataBytes
    const documentCount = workspace.documentCount + 1
    checkCapacity(totalBytes, documentCount)
    await request(documents.add(record))
    await request(tx.objectStore(MATERIALS).add({ id, blob }))
    await request(tx.objectStore(META).put({ ...workspace, totalBytes, documentCount, selectedDocumentId: id }))
    return { document, duplicate: false }
  }, signal)
}

function restoreInterrupted(document: StudyDocument): StudyDocument {
  for (const page of Object.values(document.pages)) {
    for (const turn of page.history) {
      if (turn.status === 'streaming') {
        turn.status = 'interrupted'
        turn.error = 'This response was interrupted before completion. Any partial answer is preserved.'
      }
    }
  }
  return document
}

export async function loadStudyWorkspace(): Promise<StudyWorkspace> {
  return transaction([DOCUMENTS, MATERIALS, META], 'readonly', async (tx) => {
    const records: unknown[] = await request(tx.objectStore(DOCUMENTS).getAll(undefined, STUDY_LIMITS.maxDocuments + 1))
    const warnings: string[] = []
    const documents: StudyDocument[] = []
    let totalBytes = 0
    for (const value of records) {
      try {
        const record = storedDocument(value)
        totalBytes += record.document.size + record.metadataBytes
        if (await request(tx.objectStore(MATERIALS).getKey(record.id)) === undefined) throw new StudyStorageError('corrupt', `The original file for “${record.document.name}” is missing. Its notes are still stored locally.`)
        documents.push(restoreInterrupted(record.document))
      } catch (error) { warnings.push(failure(error).message) }
    }
    let selectedDocumentId: string | null = null
    try {
      const workspace = await workspaceRecord(tx)
      selectedDocumentId = workspace.selectedDocumentId
      if (workspace.documentCount !== records.length || workspace.totalBytes !== totalBytes || records.length > STUDY_LIMITS.maxDocuments) warnings.push('The Study library’s storage accounting is inconsistent. Existing data has been preserved.')
      if (selectedDocumentId && !documents.some((document) => document.id === selectedDocumentId)) {
        warnings.push('The previously selected material could not be restored.')
        selectedDocumentId = null
      }
    } catch (error) { warnings.push(failure(error).message) }
    documents.sort((a, b) => b.updatedAt - a.updatedAt || a.id.localeCompare(b.id))
    return { documents, selectedDocumentId, warnings }
  })
}

/** Saves only this document, never its binary or unrelated documents. Rejections must stay visible in the UI. */
export async function saveStudyDocument(value: StudyDocument): Promise<number> {
  const snapshot = snapshotDocument(value)
  documentRecord(snapshot)
  return transaction([DOCUMENTS, META], 'readwrite', async (tx) => {
    const documents = tx.objectStore(DOCUMENTS)
    const value: unknown = await request(documents.get(snapshot.id))
    if (value === undefined) throw new StudyStorageError('not-found', 'This material was removed. The latest changes were not saved.')
    const previous = storedDocument(value)
    if (snapshot.revision !== previous.document.revision) {
      throw new StudyStorageError('conflict', 'This material changed in another Kivio window. Your local edits are still in this window and were not saved. Copy your unsaved work before reloading the saved version.')
    }
    if (['kind', 'size', 'pageCount', 'createdAt'].some((key) => previous.document[key as keyof StudyDocument] !== snapshot[key as keyof StudyDocument])) invalid('The original material identity cannot be changed when saving notes.')
    if (snapshot.revision === Number.MAX_SAFE_INTEGER) throw new StudyStorageError('limit', 'This material’s revision limit has been reached. Your latest changes were not saved.')
    const next = documentRecord({ ...snapshot, revision: snapshot.revision + 1 })
    const workspace = await workspaceRecord(tx)
    if (workspace.totalBytes < previous.document.size + previous.metadataBytes) throw new StudyStorageError('corrupt', 'The Study library index is inconsistent. The latest changes were not saved.')
    const totalBytes = workspace.totalBytes - previous.metadataBytes + next.metadataBytes
    checkCapacity(totalBytes, workspace.documentCount)
    await request(documents.put(next))
    await request(tx.objectStore(META).put({ ...workspace, totalBytes }))
    return next.document.revision
  })
}

export async function readStudyDocumentBlob(id: string): Promise<Blob> {
  const blob = await transaction([DOCUMENTS, MATERIALS], 'readonly', async (tx) => {
    const value: unknown = await request(tx.objectStore(DOCUMENTS).get(id))
    if (value === undefined) throw new StudyStorageError('not-found', 'This material is no longer in the Study library.')
    const document = storedDocument(value).document
    return materialBlob(await request(tx.objectStore(MATERIALS).get(id)), document)
  })
  if (await digest(blob) !== id) throw new StudyStorageError('corrupt', 'This material’s file contents are damaged. Your notes have been kept.')
  return blob
}

export async function setSelectedStudyDocument(id: string | null): Promise<void> {
  await transaction([DOCUMENTS, META], 'readwrite', async (tx) => {
    if (id !== null) {
      const value: unknown = await request(tx.objectStore(DOCUMENTS).get(id))
      if (value === undefined) throw new StudyStorageError('not-found', 'This material is no longer in the Study library.')
      storedDocument(value)
    }
    const workspace = await workspaceRecord(tx)
    await request(tx.objectStore(META).put({ ...workspace, selectedDocumentId: id }))
  })
}

/** The caller owns explicit user confirmation; this removes original bytes and all saved work atomically. */
export async function deleteStudyDocument(id: string): Promise<void> {
  await transaction([DOCUMENTS, MATERIALS, META], 'readwrite', async (tx) => {
    const documents = tx.objectStore(DOCUMENTS)
    const value: unknown = await request(documents.get(id))
    if (value === undefined) throw new StudyStorageError('not-found', 'This material has already been removed.')
    const previous = storedDocument(value)
    const workspace = await workspaceRecord(tx)
    if (workspace.totalBytes < previous.document.size + previous.metadataBytes) throw new StudyStorageError('corrupt', 'The Study library index is inconsistent. The material has not been deleted.')
    await request(documents.delete(id))
    await request(tx.objectStore(MATERIALS).delete(id))
    await request(tx.objectStore(META).put({ ...workspace, totalBytes: workspace.totalBytes - previous.document.size - previous.metadataBytes, documentCount: workspace.documentCount - 1, selectedDocumentId: workspace.selectedDocumentId === id ? null : workspace.selectedDocumentId }))
  })
}
