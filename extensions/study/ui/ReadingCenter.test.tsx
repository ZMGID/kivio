import { act, cleanup, fireEvent, screen, waitFor } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { render } from '../../../src/test/renderWithNotices'
import ReadingCenter, { type ReadingCenterProps } from './ReadingCenter'
import { loadReadingLibrary, retryReadingSaves, updateReadingDocument } from './readingLibrary'
import type { ReadingDocument } from '../api/reading'
import { readingApi } from '../api/reading'
import { readingOperationsApi } from '../api/readingOperations'
import { chatApi } from '../../../src/chat/api'
import { api } from '../../../src/api/tauri'
import { getComposerDraft, setComposerDraft, migrateComposerDraft } from '../../../src/chat/composerDraft'
import { loadReadingPdf } from './pdfDocument'
const storage = vi.hoisted(() => ({
  document: { id: 'paper', name: 'Paper', hasPdf: true, conversationIds: [], activeConversationId: null, page: 1, offset: 0, notes: [], noteDraft: 'Original thought' } as ReadingDocument,
  save: vi.fn(),
}))
vi.mock('../api/reading', () => ({ readingApi: {
  list: async () => ({ documents: [structuredClone(storage.document)], issues: [] }), collections: async () => [],
  save: (doc: ReadingDocument) => storage.save(doc),
  pdf: vi.fn(() => new Promise(() => {})),
  prepareChat: vi.fn(),
  missingConversations: vi.fn().mockResolvedValue([]),
  text: vi.fn().mockResolvedValue(null),
} }))
vi.mock('./StudyEnvironment', () => ({ default: ({ children }: { children: (open: () => void) => React.ReactNode }) => children(() => {}) }))
vi.mock('../api/readingOperations', () => ({ readingOperationsApi: { status: vi.fn(async () => ({ translation: null, view: { kind: 'source', output: null, latest: null } })), selectView: vi.fn() } }))
vi.mock('./LiteratureLibrary', () => ({ default: ({ onRead }: { onRead: (doc: ReadingDocument) => void }) => <button onClick={() => onRead(storage.document)}>Open paper</button> }))
vi.mock('../../../src/chat/api', () => ({ chatApi: { getConversation: vi.fn() } }))
vi.mock('../../../src/api/tauri', () => ({ api: { chatSavePastedImage: vi.fn() } }))
vi.mock('pdfjs-dist', () => ({}))
vi.mock('./pdfDocument', () => ({ loadReadingPdf: vi.fn() }))
vi.mock('../../../src/chat/InputBar', () => ({ InputBar: ({ onSend }: { onSend: (text: string, attachments: { id: string; type: string; name: string; path: string }[]) => Promise<boolean> }) => <><button onClick={() => void onSend('Explain this', [])}>Ask study</button><button onClick={() => void onSend('Explain image', [{ id: 'image', type: 'image', name: 'Page.png', path: '/page.png' }])}>Ask image</button></> }))
vi.mock('../../../src/chat/MessageList', () => ({ MessageList: ({ messages = [], onSaveMessageToNote }: { messages: { id: string; content: string }[]; onSaveMessageToNote: (id: string) => void }) => <>{messages.map(message => <button key={message.id} onClick={() => onSaveMessageToNote(message.id)}>Save {message.id}</button>)}</> }))
vi.mock('../../../src/chat/ChatMarkdown', () => ({ ChatMarkdown: ({ content }: { content: string }) => <p>{content}</p> }))
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

it('saves multiline underlines with their color and removes them through the notes panel', async () => {
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} })
  vi.stubGlobal('IntersectionObserver', class { observe() {} disconnect() {} })
  vi.stubGlobal('requestAnimationFrame', () => 0)
  vi.stubGlobal('cancelAnimationFrame', () => {})
  vi.mocked(readingOperationsApi.status).mockResolvedValue({ translation: null, view: { kind: 'source', output: null, latest: null } })
  vi.mocked(readingApi.pdf).mockResolvedValue(new Uint8Array([1]))
  vi.mocked(loadReadingPdf).mockReturnValue({ promise: Promise.resolve({ numPages: 1, getOutline: async () => [], getPage: async () => ({ getViewport: () => ({ width: 600, height: 800 }), getTextContent: async () => ({ items: [] }) }) }), destroy: vi.fn() } as unknown as ReturnType<typeof loadReadingPdf>)
  let disk = structuredClone(storage.document)
  storage.save.mockImplementation(async doc => { disk = structuredClone(doc) })
  await loadReadingLibrary()
  render(<ReadingCenter {...({ inputBarProps: {}, messageListProps: {}, onActivateConversation: vi.fn().mockResolvedValue(undefined) } as unknown as ReadingCenterProps)} />)
  fireEvent.click(screen.getByRole('button', { name: 'Open paper' }))
  const page = (await screen.findByLabelText('PDF 第 1 页')).closest<HTMLElement>('[data-page]')!
  const text = document.createElement('span')
  text.textContent = 'First line second line'
  page.append(text)
  const bounds = (x: number, y: number, width: number, height: number) => ({ x, y, width, height, left: x, top: y, right: x + width, bottom: y + height, toJSON() {} })
  vi.spyOn(page, 'getBoundingClientRect').mockReturnValue(bounds(0, 0, 600, 800))
  const selection = window.getSelection()!
  const range = document.createRange()
  range.selectNodeContents(text)
  range.getBoundingClientRect = () => bounds(60, 80, 180, 40)
  range.getClientRects = () => [bounds(60, 80, 180, 16), bounds(60, 80, 180, 16), bounds(60, 104, 120, 16)] as unknown as DOMRectList
  fireEvent.click(screen.getByRole('button', { name: '下划线' }))
  fireEvent.click(screen.getByRole('button', { name: '蓝色标注' }))
  selection.removeAllRanges()
  selection.addRange(range)
  expect(selection.toString()).toBe('First line second line')
  fireEvent.mouseUp(page)
  await waitFor(() => expect(disk.notes[0]?.anchor?.mark).toBe('underline'))
  expect(disk.notes[0].anchor).toMatchObject({ color: '#4b95c9', page: 1, rects: [{ x: 0.1, y: 0.1, w: 0.3, h: 0.02 }, { x: 0.1, y: 0.13, w: 0.2, h: 0.02 }] })
  expect(selection.isCollapsed).toBe(true)
  fireEvent.click(screen.getByRole('button', { name: '查看标注与笔记' }))
  expect(screen.getByText('原文 · 下划线')).toBeInTheDocument()
  const savedId = disk.notes[0].id
  const retainedIds = disk.notes.slice(1).map(note => note.id)
  fireEvent.click(screen.getAllByRole('button', { name: '删除这条笔记或标注' })[0])
  await waitFor(() => expect(disk.notes.map(note => note.id)).toEqual(retainedIds))
  expect(disk.notes.some(note => note.id === savedId)).toBe(false)
})


it('continues an associated document conversation without attaching the PDF again', async () => {
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} })
  vi.stubGlobal('IntersectionObserver', class { observe() {} disconnect() {} })
  const originalScroll = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'scrollTo')
  Object.defineProperty(HTMLElement.prototype, 'scrollTo', { configurable: true, value: vi.fn() })
  const previous = storage.document
  storage.document = { ...previous, conversationIds: ['study-chat'], activeConversationId: 'study-chat' }
  vi.mocked(readingOperationsApi.status).mockResolvedValue({ translation: null, view: { kind: 'source', output: null, latest: null } })
  vi.mocked(readingApi.pdf).mockResolvedValue(new Uint8Array([1]))
  vi.mocked(loadReadingPdf).mockImplementation(() => ({ promise: Promise.resolve({ numPages: 1, getOutline: async () => [], getPage: async () => ({ getViewport: () => ({ width: 612, height: 792 }), getTextContent: async () => ({ items: [{ str: 'Full paper text' }] }) }) }), destroy: vi.fn() }) as unknown as ReturnType<typeof loadReadingPdf>)
  vi.mocked(chatApi.getConversation).mockResolvedValue({ id: 'study-chat', messages: [{ id: 'first', role: 'user', content: 'First question' }] } as never)
  // The persisted source is already present; the backend requests no new attachment.
  vi.mocked(readingApi.prepareChat).mockResolvedValue({ needsText: false, context: null })
  storage.save.mockResolvedValue(undefined)
  const send = vi.fn().mockResolvedValue(true)
  try {
    await loadReadingLibrary()
    await updateReadingDocument(previous.id, doc => ({ ...doc, conversationIds: ['study-chat'], activeConversationId: 'study-chat' }))
    render(<ReadingCenter {...({ inputBarProps: {}, messageListProps: {}, onActivateConversation: vi.fn().mockResolvedValue(undefined), onSend: send } as unknown as ReadingCenterProps)} />)
    fireEvent.click(screen.getByRole('button', { name: 'Open paper' }))
    await screen.findByLabelText('PDF 第 1 页')
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 20)) })
    fireEvent.click(screen.getByRole('button', { name: 'Ask study' }))
    await waitFor(() => expect(send).toHaveBeenCalled())
    expect(send.mock.calls[0][1]).toBe('Explain this')
    expect(send.mock.calls[0][2]).toEqual([])
  } finally {
    cleanup()
    storage.document = previous
    if (originalScroll) Object.defineProperty(HTMLElement.prototype, 'scrollTo', originalScroll)
    else delete (HTMLElement.prototype as unknown as { scrollTo?: unknown }).scrollTo
  }
})

it('crops a saved region from its own edition and merges the result into the migrated draft without losing typing', async () => {
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} })
  vi.stubGlobal('IntersectionObserver', class { observe() {} disconnect() {} })
  vi.stubGlobal('requestAnimationFrame', () => 0)
  vi.stubGlobal('cancelAnimationFrame', () => {})
  const previous = storage.document
  storage.document = { ...previous, activeConversationId: null, conversationIds: [], notes: [{ id: 'region', kind: 'highlight', content: 'A chart', anchor: { page: 1, text: '', mark: 'area', rect: { x: 0.1, y: 0.2, w: 0.3, h: 0.25 }, view: { kind: 'mono', runId: 'old-run' } } }] }
  vi.mocked(readingOperationsApi.status).mockResolvedValue({ translation: null, view: { kind: 'source', output: null, latest: null } })
  vi.mocked(readingApi.pdf).mockResolvedValue(new Uint8Array([1]))
  const renderPage = vi.fn(() => ({ promise: Promise.resolve() }))
  const pdf = { numPages: 1, getOutline: async () => [], getPage: async () => ({ getViewport: ({ scale }: { scale: number }) => ({ width: 600 * scale, height: 800 * scale }), getTextContent: async () => ({ items: [] }), render: renderPage }) }
  vi.mocked(loadReadingPdf).mockImplementation(() => ({ promise: Promise.resolve(pdf), destroy: vi.fn() }) as unknown as ReturnType<typeof loadReadingPdf>)
  const context = vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({} as never)
  const image = vi.spyOn(HTMLCanvasElement.prototype, 'toDataURL').mockReturnValue('data:image/png;base64,crop')
  let finish!: (value: { success: boolean; path: string }) => void
  vi.mocked(api.chatSavePastedImage).mockImplementation(() => new Promise(resolve => { finish = resolve }))
  storage.save.mockResolvedValue(undefined)
  setComposerDraft('reading-paper', { input: 'My question', quotes: ['Existing quote'], attachments: [] })
  setComposerDraft('created-chat', { input: '', quotes: [], attachments: [] })
  try {
    await loadReadingLibrary()
    await updateReadingDocument(previous.id, () => structuredClone(storage.document))
    render(<ReadingCenter {...({ inputBarProps: {}, messageListProps: {}, onActivateConversation: vi.fn().mockResolvedValue(undefined) } as unknown as ReadingCenterProps)} />)
    fireEvent.click(screen.getByRole('button', { name: 'Open paper' }))
    await screen.findByLabelText('PDF 第 1 页')
    fireEvent.click(screen.getByRole('button', { name: '查看标注与笔记' }))
    fireEvent.click(screen.getByRole('button', { name: '交给 AI' }))
    await waitFor(() => expect(finish).toBeTypeOf('function'))
    expect(readingApi.pdf).toHaveBeenLastCalledWith('paper', { kind: 'mono', runId: 'old-run' })
    const rendered = renderPage.mock.calls[0] as unknown as [{ canvas: HTMLCanvasElement; transform: number[] }]
    expect([rendered[0].canvas.width, rendered[0].canvas.height]).toEqual([540, 600])
    expect(rendered[0].transform).toEqual([1, 0, 0, 1, -180, -480])
    migrateComposerDraft('reading-paper', 'created-chat')
    setComposerDraft('created-chat', { ...getComposerDraft('created-chat')!, input: 'Typed while capturing' })
    await act(async () => finish({ success: true, path: '/crop.png' }))
    expect(getComposerDraft('reading-paper')).toBeUndefined()
    const result = getComposerDraft('created-chat')!
    expect(result.input).toContain('Typed while capturing')
    expect(result.quotes[0]).toBe('Existing quote')
    expect(result.quotes[1]).toContain('kind=mono&run=old-run')
    expect(result.attachments).toMatchObject([{ type: 'image', path: '/crop.png', name: 'Paper · 译文第 1 页选区.png' }])
  } finally {
    cleanup(); storage.document = previous; context.mockRestore(); image.mockRestore()
    setComposerDraft('created-chat', { input: '', quotes: [], attachments: [] })
  }
})

it('preserves the cited translation run when saving an answer while a different edition is open', async () => {
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} })
  vi.mocked(readingApi.pdf).mockImplementation(() => new Promise(() => {}))
  vi.mocked(readingOperationsApi.status).mockResolvedValue({ translation: null, view: { kind: 'source', output: null, latest: null } })
  let disk = structuredClone(storage.document)
  storage.save.mockImplementation(async doc => { disk = structuredClone(doc) })
  await loadReadingLibrary()
  await updateReadingDocument('paper', d => ({ ...d, activeConversationId: 'cited-chat', conversationIds: ['cited-chat'], notes: [] }))
  const message = { id: 'cited-answer', role: 'assistant', content: '[第 2 页](#reading-ref?page=2&kind=mono&run=translation-1)' }
  render(<ReadingCenter {...({ inputBarProps: {}, messageListProps: { conversationId: 'cited-chat', messages: [message] }, onActivateConversation: vi.fn().mockResolvedValue(undefined) } as unknown as ReadingCenterProps)} />)
  fireEvent.click(screen.getByRole('button', { name: 'Open paper' }))
  fireEvent.click(await screen.findByRole('button', { name: 'Save cited-answer' }))
  await waitFor(() => expect(disk.notes[0]?.anchor).toMatchObject({ page: 2, view: { kind: 'mono', runId: 'translation-1' } }))
})

it('sends an image question without waiting for unrelated PDF text extraction', async () => {
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} })
  vi.stubGlobal('IntersectionObserver', class { observe() {} disconnect() {} })
  vi.stubGlobal('requestAnimationFrame', () => 0)
  vi.stubGlobal('cancelAnimationFrame', () => {})
  vi.mocked(readingOperationsApi.status).mockResolvedValue({ translation: null, view: { kind: 'source', output: null, latest: null } })
  vi.mocked(readingApi.pdf).mockResolvedValue(new Uint8Array([1]))
  vi.mocked(loadReadingPdf).mockReturnValue({ promise: Promise.resolve({ numPages: 500, getOutline: async () => [], getPage: async () => ({ getViewport: () => ({ width: 600, height: 800 }), getTextContent: () => new Promise(() => {}) }) }), destroy: vi.fn() } as unknown as ReturnType<typeof loadReadingPdf>)
  storage.save.mockResolvedValue(undefined)
  await loadReadingLibrary()
  await updateReadingDocument('paper', d => ({ ...d, activeConversationId: 'image-chat', conversationIds: ['image-chat'] }))
  vi.mocked(chatApi.getConversation).mockResolvedValue({ id: 'image-chat', messages: [] } as never)
  const send = vi.fn().mockResolvedValue(true)
  render(<ReadingCenter {...({ inputBarProps: {}, messageListProps: {}, onActivateConversation: vi.fn().mockResolvedValue(undefined), onSend: send } as unknown as ReadingCenterProps)} />)
  fireEvent.click(screen.getByRole('button', { name: 'Open paper' }))
  await screen.findByLabelText('PDF 第 1 页')
  fireEvent.click(screen.getByRole('button', { name: 'Ask image' }))
  await waitFor(() => expect(send).toHaveBeenCalled())
  expect(send.mock.calls[0][2]).toMatchObject([{ type: 'image', path: '/page.png' }])
})


it('repairs a deleted active chat once, preserves reading work, and allows a new chat', async () => {
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} })
  vi.mocked(readingApi.pdf).mockImplementation(() => new Promise(() => {}))
  const note = { id: 'kept-note', kind: 'note' as const, content: 'Keep this understanding' }
  let disk = structuredClone(storage.document)
  storage.save.mockImplementation(async doc => { disk = structuredClone(doc) })
  await loadReadingLibrary()
  await updateReadingDocument('paper', d => ({ ...d, activeConversationId: 'deleted-chat', conversationIds: ['valid-chat', 'deleted-chat'], notes: [note], noteDraft: 'Keep this draft' }))
  vi.mocked(readingApi.missingConversations).mockResolvedValue(['deleted-chat'])
  setComposerDraft('reading-paper', { input: '', quotes: [], attachments: [] })
  setComposerDraft('deleted-chat', { input: 'Unsent question', quotes: [], attachments: [] })
  const activate = vi.fn(async (id: string | null) => { if (id === 'deleted-chat') throw new Error('对话不存在：deleted-chat') })
  const create = vi.fn().mockResolvedValue({ id: 'replacement-chat' })
  const props = { inputBarProps: {}, messageListProps: {}, onActivateConversation: activate, onCreateConversation: create } as unknown as ReadingCenterProps
  const first = render(<ReadingCenter {...props} />)
  fireEvent.click(screen.getByRole('button', { name: 'Open paper' }))
  await waitFor(() => expect(disk.activeConversationId).toBeNull())
  expect(disk).toMatchObject({ conversationIds: ['valid-chat'], notes: [note], noteDraft: 'Keep this draft' })
  expect(getComposerDraft('reading-paper')?.input).toBe('Unsent question')
  expect(screen.queryByRole('alert')).not.toBeInTheDocument()
  first.unmount()
  activate.mockClear()
  render(<ReadingCenter {...props} />)
  fireEvent.click(screen.getByRole('button', { name: 'Open paper' }))
  await waitFor(() => expect(activate).toHaveBeenCalledWith(null))
  expect(activate).not.toHaveBeenCalledWith('deleted-chat')
  fireEvent.click(screen.getByRole('button', { name: '新建对话' }))
  await waitFor(() => expect(disk.activeConversationId).toBe('replacement-chat'))
  expect(disk.conversationIds).toEqual(['valid-chat', 'replacement-chat'])
  expect(getComposerDraft('replacement-chat')?.input).toBe('Unsent question')
})

it.each(['exists', 'probe-fails'])('preserves chat links on a read failure when existence check %s', async mode => {
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} })
  vi.mocked(readingApi.pdf).mockImplementation(() => new Promise(() => {}))
  let disk = structuredClone(storage.document)
  storage.save.mockImplementation(async doc => { disk = structuredClone(doc) })
  await loadReadingLibrary()
  await updateReadingDocument('paper', d => ({ ...d, activeConversationId: 'unreadable-chat', conversationIds: ['unreadable-chat'] }))
  if (mode === 'exists') vi.mocked(readingApi.missingConversations).mockResolvedValue([])
  else vi.mocked(readingApi.missingConversations).mockRejectedValue(new Error('permission denied'))
  const activate = vi.fn().mockRejectedValue(new Error('Cannot read conversation'))
  render(<ReadingCenter {...({ inputBarProps: {}, messageListProps: {}, onActivateConversation: activate } as unknown as ReadingCenterProps)} />)
  fireEvent.click(screen.getByRole('button', { name: 'Open paper' }))
  expect(await screen.findByRole('alert')).toHaveTextContent('Cannot read conversation')
  expect(disk).toMatchObject({ activeConversationId: 'unreadable-chat', conversationIds: ['unreadable-chat'] })
})

it('ignores a late missing-chat receipt after switching to a valid conversation', async () => {
  vi.stubGlobal('ResizeObserver', class { observe() {} disconnect() {} })
  vi.mocked(readingApi.pdf).mockImplementation(() => new Promise(() => {}))
  let disk = structuredClone(storage.document)
  storage.save.mockImplementation(async doc => { disk = structuredClone(doc) })
  await loadReadingLibrary()
  await updateReadingDocument('paper', d => ({ ...d, activeConversationId: 'old-chat', conversationIds: ['old-chat', 'current-chat'] }))
  let finish!: (ids: string[]) => void
  vi.mocked(readingApi.missingConversations).mockImplementation(() => new Promise(resolve => { finish = resolve }))
  const activate = vi.fn(async (id: string | null) => { if (id === 'old-chat') throw new Error('missing') })
  render(<ReadingCenter {...({ inputBarProps: {}, messageListProps: {}, onActivateConversation: activate } as unknown as ReadingCenterProps)} />)
  fireEvent.click(screen.getByRole('button', { name: 'Open paper' }))
  await waitFor(() => expect(finish).toBeTypeOf('function'))
  await act(async () => { await updateReadingDocument('paper', d => ({ ...d, activeConversationId: 'current-chat' })) })
  await act(async () => { finish(['old-chat']) })
  expect(disk).toMatchObject({ activeConversationId: 'current-chat', conversationIds: ['old-chat', 'current-chat'] })
  expect(screen.queryByRole('alert')).not.toBeInTheDocument()
})
