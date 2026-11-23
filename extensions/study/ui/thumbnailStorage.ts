// Version the store with the rendering policy (first page, 160 px, PNG).
const STORE = 'covers'
const MAX_COVERS = 128
const MAX_DATA_URL_LENGTH = 160 * 1024
let opening: Promise<IDBDatabase> | undefined

function database() {
  if (!opening) {
    opening = new Promise<IDBDatabase>((resolve, reject) => {
      const request = indexedDB.open('kivio-study-covers', 1)
      request.onupgradeneeded = () => {
        const store = request.result.createObjectStore(STORE, { keyPath: 'key' })
        store.createIndex('writtenAt', 'writtenAt')
      }
      request.onsuccess = () => {
        const db = request.result
        db.onversionchange = () => { db.close(); opening = undefined }
        resolve(db)
      }
      request.onerror = () => reject(request.error)
    }).catch(error => { opening = undefined; throw error })
  }
  return opening
}

export async function storedThumbnail(key: string): Promise<string | undefined> {
  try {
    const db = await database()
    const value = await new Promise<unknown>((resolve, reject) => {
      const request = db.transaction(STORE).objectStore(STORE).get(key)
      request.onsuccess = () => resolve(request.result?.source)
      request.onerror = () => reject(request.error)
    })
    if (typeof value !== 'string' || value.length > MAX_DATA_URL_LENGTH || !value.startsWith('data:image/png;base64,iVBORw0KGgo')) return
    const image = new Image()
    image.src = value
    await image.decode()
    return value
  } catch { return undefined }
}

export async function storeThumbnail(key: string, source: string) {
  if (source.length > MAX_DATA_URL_LENGTH) return
  try {
    const db = await database()
    const tx = db.transaction(STORE, 'readwrite')
    const store = tx.objectStore(STORE)
    store.put({ key, source, writtenAt: Date.now() })
    const count = store.count()
    count.onsuccess = () => {
      if (count.result <= MAX_COVERS) return
      const oldest = store.index('writtenAt').getAllKeys(null, count.result - MAX_COVERS)
      oldest.onsuccess = () => { for (const key of oldest.result) store.delete(key) }
    }
    await new Promise<void>((resolve, reject) => {
      tx.oncomplete = () => resolve()
      tx.onabort = () => reject(tx.error)
    })
  } catch { /* A full or unavailable store still permits in-memory previews. */ }
}
