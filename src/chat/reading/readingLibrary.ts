import { useSyncExternalStore } from 'react'
import {
  readingApi,
  type ReadingDocument,
  type ReadingCollection,
  type Bibliography,
} from '../../api/reading'

// One window-owned writer keeps pending edits alive when the route unmounts.
let snapshot = {
  documents: [] as ReadingDocument[],
  collections: [] as ReadingCollection[],
  loading: true,
  error: '',
  saving: 0,
}
const subscribers = new Set<() => void>()
const pending = new Map<string, Promise<void>>()
const dirty = new Map<string, ReadingDocument>()
let collectionWrite = Promise.resolve()
let dirtyCollections: ReadingCollection[] | undefined
let initialization: Promise<void> | undefined
function publish(patch: Partial<typeof snapshot>) {
  snapshot = { ...snapshot, ...patch }
  subscribers.forEach((listener) => listener())
}
const subscribe = (listener: () => void) => {
  subscribers.add(listener)
  return () => {
    subscribers.delete(listener)
  }
}
const getSnapshot = () => snapshot
export function useReadingLibrary() {
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot)
}
export function loadReadingLibrary() {
  if (!initialization)
    initialization = Promise.all([readingApi.list(), readingApi.collections()])
      .then(([documents, collections]) => {
        publish({ documents, collections, loading: false, error: '' })
      })
      .catch((error) => {
        initialization = undefined
        publish({ loading: false, error: String(error) })
      })
  return initialization
}
export async function importReadingDocument(path: string) {
  const document = await readingApi.import(path)
  acceptDocuments([document])
  return document
}
export function updateReadingDocument(
  id: string,
  update: (document: ReadingDocument) => ReadingDocument,
) {
  const current = snapshot.documents.find((document) => document.id === id)
  if (!current) return Promise.reject(new Error('文档不存在'))
  const next = { ...update(current), updatedAt: new Date().toISOString() }
  dirty.set(id, next)
  publish({
    documents: snapshot.documents.map((document) =>
      document.id === id ? next : document,
    ),
    saving: snapshot.saving + 1,
  })
  // Each save runs after the previous one, so older snapshots cannot overwrite new edits.
  const write = (pending.get(id) ?? Promise.resolve())
    .catch(() => {})
    .then(async () => {
      await readingApi.save(next)
      if (dirty.get(id) === next) dirty.delete(id)
    })
  pending.set(id, write)
  void write
    .then(
      () => {
        if (!dirty.size && !dirtyCollections) publish({ error: '' })
      },
      (error) =>
        publish({ error: `保存失败，修改仍保留在此窗口：${String(error)}` }),
    )
    .finally(() => {
      if (pending.get(id) === write) pending.delete(id)
      publish({ saving: snapshot.saving - 1 })
    })
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

function acceptDocuments(documents: ReadingDocument[]) {
  const merged = new Map(snapshot.documents.map((d) => [d.id, d]))
  for (const doc of documents) if (!dirty.has(doc.id)) merged.set(doc.id, doc)
  publish({ documents: [...merged.values()] })
}
export async function createReadingItem(
  name: string,
  bibliography: Bibliography,
) {
  const doc = await readingApi.create(name, bibliography)
  acceptDocuments([doc])
  return doc
}
export async function attachReadingPdf(id: string, path: string) {
  await pending.get(id)
  const doc = await readingApi.attach(id, path)
  acceptDocuments([doc])
  return doc
}
export async function importReadingRis(path: string) {
  try {
    const docs = await readingApi.importRis(path)
    acceptDocuments(docs)
    return docs
  } catch (error) {
    // A disk failure can occur after some records were written. Keep those visible.
    acceptDocuments(await readingApi.list())
    throw error
  }
}
export function updateReadingCollections(
  update: (collections: ReadingCollection[]) => ReadingCollection[],
) {
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

export async function exportReadingRis(ids: string[], path: string) {
  await Promise.all(ids.map((id) => pending.get(id)))
  if (ids.some((id) => dirty.has(id)))
    throw new Error('请先重试保存未写入的文献资料，再导出')
  await readingApi.exportRis(ids, path)
}
