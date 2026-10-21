// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import LiteratureLibrary from './LiteratureLibrary'
import type { ReadingDocument } from '../api/reading'
const library = vi.hoisted(() => ({ documents: [] as ReadingDocument[] }))
beforeEach(() => { vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(1200) })
afterEach(() => { cleanup(); library.documents = []; vi.restoreAllMocks(); vi.unstubAllGlobals() })
const api = vi.hoisted(() => ({ lookup: vi.fn() }))
const operations = vi.hoisted(() => ({ versions: vi.fn() }))
vi.mock('../api/readingOperations', () => ({ readingOperationsApi: operations }))
vi.mock('./pdfDocument', () => ({ loadReadingPdf: vi.fn() }))
vi.mock('../api/reading', async (original) => ({
  ...(await original<typeof import('../api/reading')>()),
  readingApi: api,
}))
vi.mock('./readingLibrary', () => ({
  useReadingLibrary: () => ({
    documents: library.documents,
    collections: [],
    loading: false,
    saving: 0,
    saveStatus: '已保存到本机',
  }),
  createReadingItem: vi.fn(),
  attachReadingPdf: vi.fn(),
  importReadingDocument: vi.fn(),
  importReadingRis: vi.fn(),
  exportReadingRis: vi.fn(),
  updateReadingCollections: vi.fn(),
  updateReadingDocument: vi.fn(),
}))
it('keeps the creation dialog open on Escape during lookup and permits retry after failure', async () => {
  Object.defineProperty(HTMLDialogElement.prototype, 'showModal', {
    configurable: true,
    value() {
      this.setAttribute('open', '')
    },
  })
  let fail!: (error: Error) => void
  api.lookup.mockReturnValue(
    new Promise((_, reject) => {
      fail = reject
    }),
  )
  render(<LiteratureLibrary onRead={() => {}} />)
  fireEvent.click(screen.getByRole('button', { name: '新建文献' }))
  fireEvent.change(screen.getByLabelText('新文献 DOI'), {
    target: { value: '10.1000/notfound' },
  })
  fireEvent.click(screen.getByRole('button', { name: '获取并加入 Kivio Study' }))
  const dialog = screen.getByRole('dialog')
  const cancellation = new Event('cancel', { cancelable: true })
  fireEvent(dialog, cancellation)
  expect(cancellation.defaultPrevented).toBe(true)
  fail(new Error('not found'))
  await waitFor(() =>
    expect(
      screen.getByRole('button', { name: '获取并加入 Kivio Study' }),
    ).toBeEnabled(),
  )
  expect(dialog).toBeVisible()
  expect(
    screen
      .getAllByRole('alert')
      .some((e) => e.textContent?.includes('not found')),
  ).toBe(true)
})

it('sorts by actual opening time and intersects optional filters without trapping navigation', () => {
  const base: ReadingDocument = {
    id: 'recent', name: 'Recently opened paper', hasPdf: false,
    conversationIds: [], activeConversationId: null, page: 1, offset: 0,
    notes: [], noteDraft: '', tags: ['Theory'], favorite: true,
    lastOpenedAt: '2026-10-18T03:00:00Z', addedAt: '2026-10-01',
  }
  library.documents = [
    { ...base, id: 'new', name: 'New import', lastOpenedAt: undefined, addedAt: '2026-10-18', tags: ['Methods'] },
    { ...base, id: 'old', name: 'Older paper', lastOpenedAt: '2026-10-17T03:00:00Z', updatedAt: '2026-10-19', favorite: false },
    base,
  ]
  render(<LiteratureLibrary onRead={() => {}} />)
  const table = screen.getByRole('table')
  const titles = () => within(table).getAllByRole('button', { name: /^(Recently opened paper|Older paper|New import)$/ }).map((el) => el.textContent)
  expect(titles()).toEqual(['Recently opened paper', 'Older paper', 'New import'])
  fireEvent.click(screen.getByRole('button', { name: '筛选' }))
  fireEvent.click(screen.getByRole('button', { name: 'Theory' }))
  fireEvent.click(screen.getByRole('checkbox', { name: '尚未加入文献集' }))
  expect(titles()).toEqual(['Recently opened paper', 'Older paper'])
  expect(screen.queryByRole('textbox', { name: '搜索文献' })).not.toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: '搜索文献' }))
  fireEvent.change(screen.getByRole('textbox', { name: '搜索文献' }), { target: { value: 'Recently' } })
  expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
  expect(screen.getByRole('textbox', { name: '搜索文献' })).toBeVisible()
  expect(titles()).toEqual(['Recently opened paper'])
  fireEvent.click(screen.getByRole('button', { name: '清除搜索' }))
  fireEvent.click(screen.getByRole('button', { name: '收藏 2' }))
  expect(titles()).toEqual(['Recently opened paper', 'New import'])
  expect(screen.getByRole('checkbox', { name: '尚未加入文献集' })).not.toBeChecked()
})

it('clamps keyboard resizing and hides secondary panels as the container narrows', () => {
  let resize!: () => void
  vi.stubGlobal('ResizeObserver', class {
    constructor(callback: () => void) { resize = callback }
    observe() {} disconnect() {}
  })
  library.documents = [{ id: 'paper', name: 'Paper', hasPdf: false, notes: [], noteDraft: '', tags: [], conversationIds: [], activeConversationId: null, page: 1, offset: 0 }]
  const width = vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(1200)
  render(<LiteratureLibrary onRead={() => {}} />)
  const left = screen.getByRole('separator', { name: '调整文献集栏宽度' })
  fireEvent.keyDown(left, { key: 'End' })
  expect(left).toHaveAttribute('aria-valuenow', '320')
  fireEvent.keyDown(left, { key: 'ArrowRight' })
  expect(left).toHaveAttribute('aria-valuenow', '320')
  fireEvent.click(screen.getByRole('button', { name: 'Paper' }))
  const right = screen.getByRole('separator', { name: '调整资料栏宽度' })
  fireEvent.keyDown(right, { key: 'Home' })
  expect(right).toHaveAttribute('aria-valuenow', '280')
  width.mockReturnValue(700)
  act(() => resize())
  expect(screen.queryByRole('separator', { name: '调整资料栏宽度' })).not.toBeInTheDocument()
  expect(screen.getByRole('complementary', { name: '文献资料' })).toBeVisible()
  expect(screen.getByRole('separator', { name: '调整文献集栏宽度' })).toBeVisible()
  width.mockReturnValue(500)
  act(() => resize())
  expect(screen.queryByRole('separator')).not.toBeInTheDocument()
  expect(screen.getByRole('button', { name: '切换文献集' })).toBeVisible()
  width.mockReturnValue(1200)
  act(() => resize())
  expect(screen.getByRole('separator', { name: '调整文献集栏宽度' })).toHaveAttribute('aria-valuenow', '320')
})


it('shows checkboxes only in explicit multiselect mode and clears them on exit', () => {
  library.documents = [{ id: 'paper', name: 'Paper', hasPdf: false, notes: [], noteDraft: '', tags: [], conversationIds: [], activeConversationId: null, page: 1, offset: 0 }]
  render(<LiteratureLibrary onRead={() => {}} />)
  expect(screen.queryByRole('checkbox', { name: '选择 Paper' })).not.toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: 'Paper' }))
  expect(screen.queryByText('已选 1 篇')).not.toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: '多选' }))
  expect(screen.getByRole('checkbox', { name: '选择 Paper' })).not.toBeChecked()
  fireEvent.click(screen.getByRole('button', { name: 'Paper' }))
  expect(screen.getByRole('checkbox', { name: '选择 Paper' })).toBeChecked()
  expect(screen.getByText('已选 1 篇')).toBeVisible()
  fireEvent.click(screen.getByRole('button', { name: '完成多选' }))
  expect(screen.queryByRole('checkbox', { name: '选择 Paper' })).not.toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: '多选' }))
  expect(screen.getByRole('checkbox', { name: '选择 Paper' })).not.toBeChecked()
})


it('opens paper children and keeps annotations with their exact PDF version', async () => {
  const paper: ReadingDocument = { id: 'paper', name: 'Paper', hasPdf: true, notes: [
    { id: 'original', kind: 'highlight', content: 'Original annotation', anchor: { page: 2, text: 'Original' } },
    { id: 'translated', kind: 'highlight', content: 'Translated annotation', anchor: { page: 3, text: 'Translated', view: { kind: 'mono', runId: 'run' } } },
    { id: 'thought', kind: 'note', content: 'My reading note' },
  ], noteDraft: '', conversationIds: [], activeConversationId: null, page: 1, offset: 0 }
  library.documents = [paper]
  operations.versions.mockResolvedValue([{ runId: 'run', targetLang: 'zh', page: null, createdAt: 0 }, { runId: 'older', targetLang: 'zh', page: 4, createdAt: 0 }])
  const read = vi.fn()
  render(<LiteratureLibrary onRead={read} />)
  expect(operations.versions).not.toHaveBeenCalled()
  fireEvent.click(screen.getByRole('button', { name: '展开 Paper 的子项' }))
  const translated = await screen.findByRole('button', { name: /^简体中文译文\s+全文/ })
  expect(screen.getByText('My reading note')).toBeVisible()
  expect(screen.queryByText('Original annotation')).not.toBeInTheDocument()
  expect(screen.queryByText('Translated annotation')).not.toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: '展开原文 PDF的标注' }))
  expect(screen.getByText('Original annotation')).toBeVisible()
  expect(screen.queryByText('Translated annotation')).not.toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: '展开简体中文译文的标注' }))
  fireEvent.doubleClick(screen.getByRole('button', { name: /^Translated annotation/ }))
  expect(read).toHaveBeenLastCalledWith(paper, { noteId: 'translated', page: 3, view: { kind: 'mono', runId: 'run' } })
  fireEvent.doubleClick(translated)
  expect(read).toHaveBeenLastCalledWith(paper, { view: { kind: 'mono', runId: 'run' } })
  expect(screen.getByRole('button', { name: /^简体中文译文\s+第 4 页/ })).toBeVisible()
})


it('targets the right-clicked paper for editing and preserves an existing bulk selection', async () => {
  library.documents = ['Alpha', 'Beta', 'Gamma'].map(name => ({ id: name, name, hasPdf: false, notes: [], noteDraft: '', tags: [], conversationIds: [], activeConversationId: null, page: 1, offset: 0 }))
  render(<LiteratureLibrary onRead={() => {}} />)
  fireEvent.click(screen.getByRole('button', { name: 'Alpha' }))
  fireEvent.contextMenu(screen.getByRole('button', { name: 'Beta' }))
  fireEvent.click(screen.getByRole('menuitem', { name: '编辑资料' }))
  await waitFor(() => expect(screen.getByLabelText('文献标题')).toHaveFocus())
  expect(screen.getByLabelText('文献标题')).toHaveValue('Beta')
  fireEvent.click(screen.getByRole('button', { name: '多选' }))
  fireEvent.click(screen.getByRole('checkbox', { name: '选择 Alpha' }))
  fireEvent.click(screen.getByRole('checkbox', { name: '选择 Beta' }))
  fireEvent.contextMenu(screen.getByRole('button', { name: 'Beta' }))
  expect(screen.getByRole('checkbox', { name: '选择 Alpha' })).toBeChecked()
  expect(screen.getByRole('checkbox', { name: '选择 Beta' })).toBeChecked()
  expect(screen.getByRole('menuitem', { name: '收藏所选 2 篇' })).toBeVisible()
  fireEvent.keyDown(window, { key: 'Escape' })
  fireEvent.contextMenu(screen.getByRole('button', { name: 'Gamma' }))
  expect(screen.getByRole('checkbox', { name: '选择 Alpha' })).not.toBeChecked()
  expect(screen.getByRole('checkbox', { name: '选择 Beta' })).not.toBeChecked()
  expect(screen.getByRole('checkbox', { name: '选择 Gamma' })).toBeChecked()
  expect(screen.getByRole('menuitem', { name: '编辑资料' })).toBeVisible()
})

it('reveals the inspector on paper selection and releases it on close or collection change', () => {
  library.documents = [{ id: 'paper', name: 'Paper', hasPdf: false, notes: [], noteDraft: '', tags: [], conversationIds: [], activeConversationId: null, page: 1, offset: 0 }]
  render(<LiteratureLibrary onRead={() => {}} />)
  expect(screen.queryByRole('complementary', { name: '文献资料' })).not.toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: 'Paper' }))
  expect(screen.getByLabelText('文献标题')).toHaveValue('Paper')
  fireEvent.click(screen.getByRole('button', { name: '关闭资料栏' }))
  expect(screen.queryByRole('complementary', { name: '文献资料' })).not.toBeInTheDocument()
  fireEvent.click(screen.getByRole('button', { name: 'Paper' }))
  expect(screen.getByLabelText('文献标题')).toHaveValue('Paper')
  fireEvent.click(screen.getByRole('button', { name: '收藏 0' }))
  expect(screen.queryByRole('complementary', { name: '文献资料' })).not.toBeInTheDocument()
})
