import { act, cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { render } from '../../test/renderWithNotices'
import ReadingCenter, { type ReadingCenterProps } from './ReadingCenter'
import { loadReadingLibrary, retryReadingSaves } from './readingLibrary'
import type { ReadingDocument } from '../../api/reading'
const storage = vi.hoisted(() => ({
  document: { id: 'paper', name: 'Paper', hasPdf: true, conversationIds: [], activeConversationId: null, page: 1, offset: 0, notes: [], noteDraft: 'Original thought' } as ReadingDocument,
  save: vi.fn(),
}))
vi.mock('../../api/reading', () => ({ readingApi: {
  list: async () => [structuredClone(storage.document)], collections: async () => [],
  save: (doc: ReadingDocument) => storage.save(doc),
  pdf: () => new Promise(() => {}),
} }))
vi.mock('./LiteratureLibrary', () => ({ default: ({ onRead }: { onRead: (doc: ReadingDocument) => void }) => <button onClick={() => onRead(storage.document)}>Open paper</button> }))
vi.mock('pdfjs-dist', () => ({}))
vi.mock('./pdfDocument', () => ({ loadReadingPdf: vi.fn() }))
vi.mock('../InputBar', () => ({ InputBar: () => null }))
vi.mock('../MessageList', () => ({ MessageList: () => null }))
vi.mock('../ChatMarkdown', () => ({ ChatMarkdown: ({ content }: { content: string }) => <p>{content}</p> }))
afterEach(() => { cleanup(); vi.unstubAllGlobals() })
it('consumes a saved note draft even after failure and leaving, while preserving subsequent typing', async () => {
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} })
  let disk = structuredClone(storage.document)
  storage.save.mockImplementation(async doc => { disk = structuredClone(doc) })
  await loadReadingLibrary()
  const props = { inputBarProps: {}, messageListProps: {}, onActivateConversation: vi.fn().mockResolvedValue(undefined) } as unknown as ReadingCenterProps
  const view = render(<ReadingCenter {...props} />)
  fireEvent.click(screen.getByRole('button', { name: 'Open paper' }))
  fireEvent.click(screen.getByRole('button', { name: '笔记' }))
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 450)) })
  storage.save.mockRejectedValueOnce(new Error('disk full'))
  fireEvent.click(screen.getByRole('button', { name: '保存笔记' }))
  await waitFor(() => expect(screen.getByLabelText('笔记内容')).toHaveValue(''))
  view.unmount()
  await act(async () => { await retryReadingSaves() })
  expect(disk.notes.map(note => note.content)).toEqual(['Original thought'])
  expect(disk.noteDraft).toBe('')
  render(<ReadingCenter {...props} />)
  fireEvent.click(screen.getByRole('button', { name: 'Open paper' }))
  fireEvent.click(screen.getByRole('button', { name: /^笔记/ }))
  expect(screen.getByLabelText('笔记内容')).toHaveValue('')
  fireEvent.change(screen.getByLabelText('笔记内容'), { target: { value: 'Second thought' } })
  let finish!: () => void
  storage.save.mockImplementationOnce(async doc => {
    await new Promise<void>(resolve => { finish = resolve })
    disk = structuredClone(doc)
  })
  fireEvent.click(screen.getByRole('button', { name: '保存笔记' }))
  await waitFor(() => expect(finish).toBeTypeOf('function'))
  fireEvent.change(screen.getByLabelText('笔记内容'), { target: { value: 'Next draft' } })
  await act(async () => { finish(); await new Promise(resolve => setTimeout(resolve, 450)) })
  expect(screen.getByLabelText('笔记内容')).toHaveValue('Next draft')
  expect(disk.notes.map(note => note.content)).toEqual(['Second thought', 'Original thought'])
  expect(disk.noteDraft).toBe('Next draft')
})
