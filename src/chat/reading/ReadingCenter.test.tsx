import { act, cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { render } from '../../test/renderWithNotices'
import ReadingCenter, { type ReadingCenterProps } from './ReadingCenter'
import { loadReadingLibrary, retryReadingSaves, updateReadingDocument } from './readingLibrary'
import type { ReadingDocument } from '../../api/reading'
import { readingApi } from '../../api/reading'
import { readingOperationsApi } from '../../api/readingOperations'
import { chatApi } from '../api'
import { loadReadingPdf } from './pdfDocument'
const storage = vi.hoisted(() => ({
  document: { id: 'paper', name: 'Paper', hasPdf: true, conversationIds: [], activeConversationId: null, page: 1, offset: 0, notes: [], noteDraft: 'Original thought' } as ReadingDocument,
  save: vi.fn(),
}))
vi.mock('../../api/reading', () => ({ readingApi: {
  list: async () => [structuredClone(storage.document)], collections: async () => [],
  save: (doc: ReadingDocument) => storage.save(doc),
  pdf: vi.fn(() => new Promise(() => {})),
} }))
vi.mock('./StudyEnvironment', () => ({ default: ({ children }: { children: (open: () => void) => React.ReactNode }) => children(() => {}) }))
vi.mock('../../api/readingOperations', () => ({ readingOperationsApi: { status: vi.fn(async () => ({ translation: null, view: { kind: 'source', output: null, latest: null } })), selectView: vi.fn() } }))
vi.mock('./LiteratureLibrary', () => ({ default: ({ onRead }: { onRead: (doc: ReadingDocument) => void }) => <button onClick={() => onRead(storage.document)}>Open paper</button> }))
vi.mock('../api', () => ({ chatApi: { getConversation: vi.fn() } }))
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

it('reads and searches the selected PDF version while ignoring an older PDF load that finishes late', async () => {
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} })
  vi.stubGlobal('IntersectionObserver', class { observe() {} disconnect() {} })
  vi.stubGlobal('requestAnimationFrame', (callback: () => void) => setTimeout(callback, 0))
  vi.stubGlobal('cancelAnimationFrame', (timer: ReturnType<typeof setTimeout>) => clearTimeout(timer))
  const originalScroll = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'scrollTo')
  Object.defineProperty(HTMLElement.prototype, 'scrollTo', { configurable: true, value: vi.fn() })
  const output = { runId: 'version', targetLang: 'zh', page: null }
  const source = { translation: null, view: { kind: 'source' as const, output: null, latest: output } }
  vi.mocked(readingOperationsApi.status).mockResolvedValue(source)
  vi.mocked(readingOperationsApi.selectView).mockImplementation(async (_, kind) => ({ ...source, view: { ...source.view, kind, output: kind === 'source' ? null : output } }))
  vi.mocked(readingApi.pdf).mockResolvedValue(new Uint8Array([1]))
  const pdf = (text: string) => ({ numPages: 2, getOutline: async () => [], getPage: async (page: number) => ({ getViewport: () => ({ width: 612, height: 792 }), getTextContent: async () => ({ items: [{ str: page === 1 ? text : 'Second page' }] }) }) })
  let finish!: (value: ReturnType<typeof pdf>) => void
  vi.mocked(loadReadingPdf).mockReturnValueOnce({ promise: new Promise(resolve => { finish = resolve }), destroy: vi.fn() } as unknown as ReturnType<typeof loadReadingPdf>)
    .mockImplementation(() => ({ promise: Promise.resolve(pdf('神经网络译文')), destroy: vi.fn() }) as unknown as ReturnType<typeof loadReadingPdf>)
  await loadReadingLibrary()
  const props = { inputBarProps: {}, messageListProps: {}, onActivateConversation: vi.fn().mockResolvedValue(undefined) } as unknown as ReadingCenterProps
  render(<ReadingCenter {...props} />)
  fireEvent.click(screen.getByRole('button', { name: 'Open paper' }))
  await waitFor(() => expect(finish).toBeTypeOf('function'))
  fireEvent.click(screen.getByRole('button', { name: '阅读版本' }))
  fireEvent.click(screen.getByRole('option', { name: /^译文/ }))
  await screen.findByLabelText('PDF 第 1 页')
  fireEvent.click(screen.getByRole('button', { name: '搜索文档' }))
  fireEvent.change(screen.getByLabelText('搜索文档内容'), { target: { value: '神经网络' } })
  await screen.findByText('1 页包含匹配内容')
  expect(screen.getByText('神经网络译文')).toBeInTheDocument()
  await act(async () => finish(pdf('English original')))
  expect(screen.getByText('神经网络译文')).toBeInTheDocument()
  expect(screen.queryByText('English original')).not.toBeInTheDocument()
  fireEvent.change(screen.getByLabelText('跳转页码'), { target: { value: '2' } })
  fireEvent.submit(screen.getByLabelText('跳转页码').closest('form')!)
  await waitFor(() => expect(screen.getByLabelText('跳转页码')).toHaveValue('2'))
  vi.mocked(loadReadingPdf).mockImplementation(() => ({ promise: Promise.resolve(pdf('English original')), destroy: vi.fn() }) as unknown as ReturnType<typeof loadReadingPdf>)
  fireEvent.click(screen.getByRole('button', { name: '阅读版本' }))
  fireEvent.click(screen.getByRole('option', { name: '原文' }))
  fireEvent.change(screen.getByLabelText('搜索文档内容'), { target: { value: 'English' } })
  await screen.findByText('English original')
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)) })
  expect(screen.getByLabelText('跳转页码')).toHaveValue('2')
  if (originalScroll) Object.defineProperty(HTMLElement.prototype, 'scrollTo', originalScroll)
  else delete (HTMLElement.prototype as unknown as { scrollTo?: unknown }).scrollTo
})


it('shows a linked conversation once when older document links contain duplicates', async () => {
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} })
  vi.stubGlobal('IntersectionObserver', class { observe() {} disconnect() {} })
  const previous = storage.document
  storage.document = { ...previous, conversationIds: ['same', 'same'], activeConversationId: 'same' }
  vi.mocked(readingApi.pdf).mockImplementation(() => new Promise(() => {}))
  vi.mocked(chatApi.getConversation).mockResolvedValue({ id: 'same', updated_at: 1, messages: [{ id: 'q', role: 'user', content: 'Explain attention' }] } as never)
  storage.save.mockResolvedValue(undefined)
  try {
    await loadReadingLibrary()
    await updateReadingDocument(previous.id, doc => ({ ...doc, conversationIds: ['same', 'same'], activeConversationId: 'same' }))
    const activate = vi.fn().mockResolvedValue(undefined)
    render(<ReadingCenter {...({ inputBarProps: {}, messageListProps: {}, onActivateConversation: activate } as unknown as ReadingCenterProps)} />)
    fireEvent.click(screen.getByRole('button', { name: 'Open paper' }))
    fireEvent.click(screen.getByRole('button', { name: '历史' }))
    await waitFor(() => expect(screen.getAllByRole('button', { name: /Explain attention/ })).toHaveLength(1))
    fireEvent.click(screen.getByRole('button', { name: /Explain attention/ }))
    expect(screen.getByRole('button', { name: '对话' })).toHaveAttribute('aria-pressed', 'true')
  } finally {
    cleanup()
    storage.document = previous
  }
})
