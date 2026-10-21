import { act, renderHook, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import type { ReadingDocument } from '../api/reading'
const storage = vi.hoisted(() => ({
  list: vi.fn(),
  save: vi.fn(),
  import: vi.fn(),
  create: vi.fn(),
  attach: vi.fn(),
  collections: vi.fn().mockResolvedValue([]),
  saveCollections: vi.fn(),
}))
vi.mock('../api/reading', () => ({ readingApi: { ...storage, list: async () => ({ documents: await storage.list(), issues: [] }) } }))

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


it('refuses attachment while a failed edit is unsaved, then preserves attachment through later edits', async () => {
  vi.resetModules()
  let disk: ReadingDocument = { id: 'paper', name: 'Paper', hasPdf: false, fingerprint: '', conversationIds: [], activeConversationId: null, page: 1, offset: 0, notes: [], noteDraft: '' }
  storage.list.mockResolvedValue([disk])
  storage.save.mockReset().mockRejectedValueOnce(new Error('disk full')).mockImplementation(async doc => { disk = structuredClone(doc) })
  storage.attach.mockReset().mockImplementation(async () => {
    disk = { ...disk, hasPdf: true, fingerprint: 'pdf-hash' }
    return structuredClone(disk)
  })
  const library = await import('./readingLibrary')
  await library.loadReadingLibrary()
  await library.updateReadingDocument('paper', d => ({ ...d, name: 'Edited' })).catch(() => {})
  await new Promise(resolve => setTimeout(resolve, 0))
  await expect(library.attachReadingPdf('paper', '/paper.pdf')).rejects.toThrow(/保存/)
  expect(storage.attach).not.toHaveBeenCalled()
  await library.retryReadingSaves()
  await library.attachReadingPdf('paper', '/paper.pdf')
  await library.updateReadingDocument('paper', d => ({ ...d, favorite: true }))
  expect(disk).toMatchObject({ name: 'Edited', hasPdf: true, fingerprint: 'pdf-hash', favorite: true })
})

it.each([false, true])('serializes edits arriving during attachment (attachment fails: %s)', async fails => {
  vi.resetModules()
  let disk: ReadingDocument = { id: 'paper', name: 'Paper', hasPdf: false, fingerprint: '', conversationIds: [], activeConversationId: null, page: 1, offset: 0, notes: [], noteDraft: '' }
  storage.list.mockResolvedValue([disk])
  storage.save.mockReset().mockImplementation(async doc => { disk = structuredClone(doc) })
  let finish!: () => void
  storage.attach.mockReset().mockImplementation(async () => {
    await new Promise<void>(resolve => { finish = resolve })
    if (fails) throw new Error('invalid PDF')
    disk = { ...disk, hasPdf: true, fingerprint: 'pdf-hash' }
    return structuredClone(disk)
  })
  const library = await import('./readingLibrary')
  await library.loadReadingLibrary()
  const attaching = library.attachReadingPdf('paper', '/paper.pdf').catch(() => {})
  await waitFor(() => expect(finish).toBeTypeOf('function'))
  const editing = library.updateReadingDocument('paper', d => ({ ...d, name: 'During attachment' }))
  finish()
  await Promise.all([attaching, editing])
  expect(disk).toMatchObject({ name: 'During attachment', hasPdf: !fails, fingerprint: fails ? '' : 'pdf-hash' })
  const view = renderHook(library.useReadingLibrary)
  expect(view.result.current.documents[0]).toEqual(disk)
  view.unmount()
})

it('bounds pending disk writes during rapid edits and flushes the final combined document', async () => {
  vi.resetModules()
  let disk: ReadingDocument = { id: 'paper', name: 'Paper', conversationIds: [], activeConversationId: null, page: 1, offset: 0, notes: [], noteDraft: '' }
  storage.list.mockResolvedValue([disk])
  let release!: () => void
  let writes = 0
  storage.save.mockReset().mockImplementation(async doc => {
    if (++writes === 1) await new Promise<void>(resolve => { release = resolve })
    disk = structuredClone(doc)
  })
  const library = await import('./readingLibrary')
  await library.loadReadingLibrary()
  const first = library.updateReadingDocument('paper', d => ({ ...d, favorite: true }))
  await waitFor(() => expect(release).toBeTypeOf('function'))
  const edits = Array.from({ length: 100 }, (_, page) => library.updateReadingDocument('paper', d => ({ ...d, page: page + 1, noteDraft: `Draft ${page}` })))
  const flush = library.flushReadingItems(['paper'])
  release()
  await Promise.all([first, ...edits, flush])
  expect(disk).toMatchObject({ favorite: true, page: 100, noteDraft: 'Draft 99' })
  expect(writes).toBeLessThanOrEqual(2)
})

it('debounces typing but lets export flush the latest draft immediately', async () => {
  vi.resetModules()
  vi.useFakeTimers()
  try {
    const doc: ReadingDocument = { id: 'paper', name: 'Paper', conversationIds: [], activeConversationId: null, page: 1, offset: 0, notes: [], noteDraft: '' }
    storage.list.mockResolvedValue([doc])
    storage.save.mockReset().mockResolvedValue(undefined)
    const library = await import('./readingLibrary')
    await library.loadReadingLibrary()
    const first = library.updateReadingDocument('paper', d => ({ ...d, name: 'P' }), true)
    await vi.advanceTimersByTimeAsync(100)
    const second = library.updateReadingDocument('paper', d => ({ ...d, name: 'Paper edited' }), true)
    await vi.advanceTimersByTimeAsync(150)
    expect(storage.save).not.toHaveBeenCalled()
    await library.flushReadingItems(['paper'])
    await Promise.all([first, second])
    expect(storage.save).toHaveBeenCalledTimes(1)
    expect(storage.save.mock.lastCall?.[0].name).toBe('Paper edited')
  } finally { vi.useRealTimers() }
})

it('freezes inactive library subscriptions and catches up when returning', async () => {
  vi.resetModules()
  storage.list.mockResolvedValue([{ id: 'paper', name: 'Before', notes: [] }])
  storage.save.mockReset().mockResolvedValue(undefined)
  const library = await import('./readingLibrary')
  await library.loadReadingLibrary()
  const view = renderHook(({ active }) => library.useReadingLibrary(active), { initialProps: { active: true } })
  view.rerender({ active: false })
  await act(async () => { await library.updateReadingDocument('paper', d => ({ ...d, name: 'After' })) })
  expect(view.result.current.documents[0].name).toBe('Before')
  view.rerender({ active: true })
  expect(view.result.current.documents[0].name).toBe('After')
})

it('retains a missing-chat repair for retry without dropping notes or valid links', async () => {
  vi.resetModules()
  let disk: ReadingDocument = { id: 'paper', name: 'Paper', conversationIds: ['deleted', 'valid'], activeConversationId: 'deleted', page: 3, offset: 10, notes: [{ id: 'note', kind: 'note', content: 'Saved evidence' }], noteDraft: 'Unfinished thought' }
  const original = structuredClone(disk)
  storage.list.mockResolvedValue([disk])
  storage.save.mockReset().mockRejectedValueOnce(new Error('disk full')).mockImplementation(async doc => { disk = structuredClone(doc) })
  const library = await import('./readingLibrary')
  await library.loadReadingLibrary()
  await expect(library.removeMissingReadingConversations('paper', ['deleted'])).rejects.toThrow('disk full')
  const view = renderHook(library.useReadingLibrary)
  expect(view.result.current.documents[0].activeConversationId).toBeNull()
  expect(view.result.current.saveStatus).toBe('保存失败，尚未保存')
  expect(disk).toEqual(original)
  view.unmount()
  await library.retryReadingSaves()
  expect(disk).toMatchObject({ activeConversationId: null, conversationIds: ['valid'], notes: original.notes, noteDraft: original.noteDraft, page: 3, offset: 10 })
})
