import { act, renderHook, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import type { ReadingDocument } from '../../api/reading'
const storage = vi.hoisted(() => ({
  list: vi.fn(),
  save: vi.fn(),
  import: vi.fn(),
  create: vi.fn(),
  collections: vi.fn().mockResolvedValue([]),
  saveCollections: vi.fn(),
}))
vi.mock('../../api/reading', () => ({ readingApi: storage }))

it('keeps unsaved reading work across unmount and retries the latest snapshot', async () => {
  vi.resetModules()
  const doc: ReadingDocument = {
    id: 'paper',
    name: 'Paper',
    conversationIds: [],
    activeConversationId: null,
    page: 1,
    offset: 0,
    notes: [],
    noteDraft: '',
  }
  storage.list.mockResolvedValue([doc])
  storage.save
    .mockRejectedValueOnce(new Error('disk full'))
    .mockResolvedValue(undefined)
  const library = await import('./readingLibrary')
  await library.loadReadingLibrary()
  const view = renderHook(library.useReadingLibrary)
  await act(async () => {
    await library
      .updateReadingDocument('paper', (doc) => ({
        ...doc,
        noteDraft: 'keep me',
      }))
      .catch(() => {})
  })
  expect(view.result.current.error).toContain('disk full')
  await waitFor(() => expect(view.result.current.saving).toBe(0))
  expect(view.result.current.saveStatus).toBe('保存失败，尚未保存')
  view.unmount()
  const reopened = renderHook(library.useReadingLibrary)
  expect(reopened.result.current.documents[0].noteDraft).toBe('keep me')
  await act(async () => {
    await library.retryReadingSaves()
  })
  await waitFor(() => expect(reopened.result.current.error).toBe(''))
  expect(reopened.result.current.saveStatus).toBe('已保存到本机')
  expect(storage.save.mock.lastCall?.[0].noteDraft).toBe('keep me')
})

describe('serialized document writes', () => {
  it('cannot finish an older save after a newer snapshot', async () => {
    vi.resetModules()
    let release!: () => void
    const firstWrite = new Promise<void>((resolve) => {
      release = resolve
    })
    storage.list.mockResolvedValue([
      {
        id: 'paper',
        name: 'Paper',
        conversationIds: [],
        activeConversationId: null,
        page: 1,
        offset: 0,
        notes: [],
        noteDraft: '',
      },
    ])
    storage.save
      .mockReset()
      .mockReturnValueOnce(firstWrite)
      .mockResolvedValue(undefined)
    const library = await import('./readingLibrary')
    await library.loadReadingLibrary()
    const first = library.updateReadingDocument('paper', (doc) => ({
      ...doc,
      page: 2,
    }))
    const second = library.updateReadingDocument('paper', (doc) => ({
      ...doc,
      page: 3,
    }))
    await waitFor(() => expect(storage.save).toHaveBeenCalledTimes(1))
    release()
    await first
    await second
    expect(storage.save.mock.calls.map((call) => call[0].page)).toEqual([2, 3])
  })
})

it('retains failed classification edits across remount and retries the latest tree', async () => {
  vi.resetModules()
  storage.list.mockResolvedValue([])
  storage.collections.mockResolvedValue([])
  storage.saveCollections
    .mockReset()
    .mockRejectedValueOnce(new Error('disk full'))
    .mockResolvedValue(undefined)
  const library = await import('./readingLibrary')
  await library.loadReadingLibrary()
  await library
    .updateReadingCollections(() => [
      { id: 'collection', name: 'Research', parentId: null },
    ])
    .catch(() => {})
  const first = renderHook(library.useReadingLibrary)
  expect(first.result.current.collections[0].name).toBe('Research')
  first.unmount()
  const reopened = renderHook(library.useReadingLibrary)
  await act(async () => {
    await library.retryReadingSaves()
  })
  await waitFor(() => expect(reopened.result.current.error).toBe(''))
  expect(reopened.result.current.saveStatus).toBe('已保存到本机')
  expect(storage.saveCollections.mock.lastCall?.[0]).toEqual([
    { id: 'collection', name: 'Research', parentId: null },
  ])
})


it('waits for the existing library before creating collections and documents', async () => {
  vi.resetModules()
  const oldCollection = { id: 'old', name: 'Existing', parentId: null }
  const newCollection = { id: 'new', name: 'New', parentId: null }
  let disk = [oldCollection]
  let finish!: (items: typeof disk) => void
  storage.list.mockResolvedValue([])
  storage.collections.mockImplementationOnce(() => new Promise(resolve => { finish = resolve }))
  storage.saveCollections.mockReset().mockImplementation(async items => { disk = items })
  storage.create.mockReset().mockResolvedValue({ id: 'new-paper', name: 'New paper' })
  const library = await import('./readingLibrary')
  const loading = library.loadReadingLibrary()
  const classification = library.updateReadingCollections(items => [...items, newCollection])
  const creation = library.createReadingItem('New paper', { authors: [], year: '', publication: '', doi: '', url: '', abstractText: '', itemType: 'article' })
  await Promise.resolve()
  finish([oldCollection])
  await Promise.all([loading, classification, creation])
  const view = renderHook(library.useReadingLibrary)
  expect(disk).toEqual([oldCollection, newCollection])
  expect(view.result.current.collections).toEqual(disk)
  expect(view.result.current.documents.map(doc => doc.id)).toEqual(['new-paper'])
})

it('does not write an empty replacement after initialization fails, and allows retry', async () => {
  vi.resetModules()
  storage.list.mockResolvedValue([])
  storage.collections.mockRejectedValueOnce(new Error('cannot read collections')).mockResolvedValue([])
  storage.saveCollections.mockReset().mockResolvedValue(undefined)
  const library = await import('./readingLibrary')
  await expect(library.updateReadingCollections(items => [...items, { id: 'new', name: 'New', parentId: null }])).rejects.toThrow('cannot read collections')
  expect(storage.saveCollections).not.toHaveBeenCalled()
  await library.updateReadingCollections(items => [...items, { id: 'retry', name: 'Retry', parentId: null }])
  const view = renderHook(library.useReadingLibrary)
  expect(view.result.current.collections.map(item => item.id)).toEqual(['retry'])
})
