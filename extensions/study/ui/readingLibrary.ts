import { useCallback, useRef, useSyncExternalStore } from 'react'
import { migrateComposerDraft } from '../../sdk/chat'
import {
  readingApi,
  type ReadingDocument,
  type ReadingCollection,
  type Bibliography,
} from '../api/reading'

// One window-owned writer keeps pending edits alive when the route unmounts.
let snapshot = {
  documents: [] as ReadingDocument[],
  collections: [] as ReadingCollection[],
  loading: true,
  error: '',
  issues: [] as string[],
  saving: 0,
  saveStatus: '正在加载…',
}
const subscribers = new Set<() => void>()
const pending = new Map<string, Promise<void>>()
const dirty = new Map<string, ReadingDocument>()
const delayed = new Map<string, { timer: ReturnType<typeof setTimeout>; release: () => void }>()
function deferWrite(id: string) {
  return new Promise<void>(resolve => {
    const release = () => {
      const wait = delayed.get(id)
      if (wait) clearTimeout(wait.timer)
      delayed.delete(id)
      resolve()
    }
    delayed.set(id, { timer: setTimeout(release, 200), release })
  })
}
// Attachment commands replace backend metadata; defer edits until their receipt is merged.
const attaching = new Map<string, Promise<ReadingDocument>>()
let collectionWrite = Promise.resolve()
let dirtyCollections: ReadingCollection[] | undefined
let initialization: Promise<void> | undefined
let initialized = false
function publish(patch: Partial<typeof snapshot>) {
  snapshot = { ...snapshot, ...patch }
  snapshot.saveStatus = snapshot.loading ? '正在加载…'
    : !initialized ? '加载失败'
    : snapshot.saving > 0 ? '正在保存…'
    : dirty.size || dirtyCollections ? '保存失败，尚未保存'
    : '已保存到本机'
  subscribers.forEach((listener) => listener())
}
const subscribe = (listener: () => void) => {
  subscribers.add(listener)
  return () => {
    subscribers.delete(listener)
  }
}
export function useReadingLibrary(active = true) {
  const retained = useRef(snapshot)
  const read = useCallback(() => {
    if (active) retained.current = snapshot
    return retained.current
  }, [active])
  const listen = useCallback((listener: () => void) => active ? subscribe(listener) : () => {}, [active])
  return useSyncExternalStore(listen, read, read)
}
export function loadReadingLibrary() {
  if (!initialization) {
    publish({ loading: true })
    initialization = Promise.all([readingApi.list(), readingApi.collections()])
      .then(([listing, collections]) => {
        initialized = true
        publish({ documents: listing.documents, issues: listing.issues, collections, loading: false, error: '' })
      })
      .catch((error) => {
        initialization = undefined
        publish({ loading: false, error: String(error) })
        throw error
      })
  }
  return initialization
}
export async function refreshReadingLibrary() {
  if (!initialized) return loadReadingLibrary()
  const previous = new Map(snapshot.documents.map(document => [document.id, document]))
  const listing = await readingApi.list()
  const documents = new Map(listing.documents.map(document => [document.id, document]))
  // A reload receipt cannot overwrite edits made while the scan was in flight.
  for (const document of snapshot.documents) {
    if (dirty.has(document.id) || previous.get(document.id) !== document) documents.set(document.id, document)
  }
  publish({ documents: [...documents.values()], issues: listing.issues })
}
export async function importReadingDocument(path: string) {
  await loadReadingLibrary()
  const document = await readingApi.import(path)
  acceptDocuments([document])
  return document
}
export function updateReadingDocument(
  id: string,
  update: (document: ReadingDocument) => ReadingDocument,
  defer = false,
): Promise<void> {
  if (!initialized) return loadReadingLibrary().then(() => updateReadingDocument(id, update, defer))
  const attachment = attaching.get(id)
  if (attachment) return attachment.catch(() => {}).then(() => updateReadingDocument(id, update, defer))
  const current = snapshot.documents.find((document) => document.id === id)
  if (!current) return Promise.reject(new Error('文档不存在'))
  const next = { ...update(current), updatedAt: new Date().toISOString() }
  dirty.set(id, next)
  const existing = pending.get(id)
  publish({
    documents: snapshot.documents.map(document => document.id === id ? next : document),
    saving: snapshot.saving + (existing ? 0 : 1),
  })
  if (existing) {
    const wait = delayed.get(id)
    if (wait) {
      if (defer) { clearTimeout(wait.timer); wait.timer = setTimeout(wait.release, 200) }
      else wait.release()
    }
    return existing
  }
  // One active write plus the latest dirty snapshot. Every caller waits until
  // its edits (or a newer combined snapshot) have reached disk.
  const write = (async () => {
    try {
      if (defer) await deferWrite(id)
      while (dirty.has(id)) {
        const candidate = dirty.get(id)!
        try {
          await readingApi.save(candidate)
          if (dirty.get(id) === candidate) dirty.delete(id)
        } catch (error) {
          if (dirty.get(id) !== candidate) continue
          publish({ error: `保存失败，修改仍保留在此窗口：${String(error)}` })
          throw error
        }
      }
      if (!dirty.size && !dirtyCollections) publish({ error: '' })
    } finally {
      pending.delete(id)
      publish({ saving: snapshot.saving - 1 })
    }
  })()
  pending.set(id, write)
  return write
}
export async function retryReadingSaves() {
  if (dirtyCollections) await updateReadingCollections(() => dirtyCollections!)
  await Promise.all(
    [...dirty.keys()].map((id) =>
      updateReadingDocument(id, (document) => document),
    ),
  )
}

export function removeMissingReadingConversations(id: string, missing: string[]) {
  const removed = new Set(missing)
  return updateReadingDocument(id, document => {
    const active = document.activeConversationId
    const clearActive = active !== null && removed.has(active)
    if (clearActive) migrateComposerDraft(active, `reading-${id}`)
    return {
      ...document,
      conversationIds: document.conversationIds.filter(value => !removed.has(value)),
      activeConversationId: clearActive ? null : active,
    }
  })
}

function acceptDocuments(documents: ReadingDocument[]) {
  const merged = new Map(snapshot.documents.map((d) => [d.id, d]))
  for (const doc of documents) if (!dirty.has(doc.id)) merged.set(doc.id, doc)
  publish({ documents: [...merged.values()] })
}
export async function createReadingItem(
  name: string,
  bibliography: Bibliography,
) {
  await loadReadingLibrary()
  const doc = await readingApi.create(name, bibliography)
  acceptDocuments([doc])
  return doc
}
export async function attachReadingPdf(id: string, path: string): Promise<ReadingDocument> {
  if (!initialized) {
    await loadReadingLibrary()
    return attachReadingPdf(id, path)
  }
  const previous = attaching.get(id)
  if (previous) {
    await previous.catch(() => {})
    return attachReadingPdf(id, path)
  }
  const write = (async () => {
    await pending.get(id)
    if (dirty.has(id)) throw new Error('请先重试保存未写入的文献资料，再关联 PDF')
    const doc = await readingApi.attach(id, path)
    acceptDocuments([doc])
    return doc
  })()
  attaching.set(id, write)
  try {
    return await write
  } finally {
    if (attaching.get(id) === write) attaching.delete(id)
  }
}
export async function importReadingRis(path: string) {
  await loadReadingLibrary()
  try {
    const docs = await readingApi.importRis(path)
    acceptDocuments(docs)
    return docs
  } catch (error) {
    // A disk failure can occur after some records were written. Keep those visible.
    acceptDocuments((await readingApi.list()).documents)
    throw error
  }
}
export function updateReadingCollections(
  update: (collections: ReadingCollection[]) => ReadingCollection[],
): Promise<void> {
  if (!initialized) return loadReadingLibrary().then(() => updateReadingCollections(update))
  const next = update(snapshot.collections)
  dirtyCollections = next
  publish({ collections: next, saving: snapshot.saving + 1 })
  const write = collectionWrite
    .catch(() => {})
    .then(async () => {
      await readingApi.saveCollections(next)
      if (dirtyCollections === next) dirtyCollections = undefined
    })
  collectionWrite = write
  void write
    .then(
      () => {
        if (!dirty.size && !dirtyCollections) publish({ error: '' })
      },
      (error) =>
        publish({
          error: `分类保存失败，修改仍保留在此窗口：${String(error)}`,
        }),
    )
    .finally(() => publish({ saving: snapshot.saving - 1 }))
  return write
}

export async function flushReadingItems(ids: string[]) {
  await loadReadingLibrary()
  for (const id of ids) delayed.get(id)?.release()
  await Promise.all(ids.map((id) => pending.get(id)))
  if (ids.some((id) => dirty.has(id)))
    throw new Error('请先重试保存未写入的文献资料，再导出')
}

export async function exportReadingRis(ids: string[], path: string) {
  await flushReadingItems(ids)
  await readingApi.exportRis(ids, path)
}

export async function importReadingDocuments(paths: string[]) {
  await loadReadingLibrary()
  const result = await readingApi.importMany(paths)
  acceptDocuments(result.documents)
  return result
}

/** Disable barrier: never initialize an unused library merely to turn it off. */
export async function flushReadingLibrary() {
  if (!initialization) return
  await initialization
  for (const wait of delayed.values()) wait.release()
  await Promise.all([...attaching.values(), ...pending.values(), collectionWrite])
  if (dirty.size || dirtyCollections) throw new Error('请回到文献库重试保存')
}
