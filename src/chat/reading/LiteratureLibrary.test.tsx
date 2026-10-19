// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import LiteratureLibrary from './LiteratureLibrary'
import type { ReadingDocument } from '../../api/reading'
const library = vi.hoisted(() => ({ documents: [] as ReadingDocument[] }))
beforeEach(() => { vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(1200) })
afterEach(() => { cleanup(); library.documents = []; vi.restoreAllMocks(); vi.unstubAllGlobals() })
const api = vi.hoisted(() => ({ lookup: vi.fn() }))
vi.mock('./pdfDocument', () => ({ loadReadingPdf: vi.fn() }))
vi.mock('../../api/reading', async (original) => ({
  ...(await original<typeof import('../../api/reading')>()),
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
  const titles = () => within(table).getAllByRole('button').map((el) => el.textContent)
  expect(titles()).toEqual(['Recently opened paper', 'Older paper', 'New import'])
  fireEvent.click(screen.getByRole('button', { name: '筛选' }))
  fireEvent.click(screen.getByRole('button', { name: 'Theory' }))
  fireEvent.click(screen.getByRole('checkbox', { name: '尚未加入文献集' }))
  expect(titles()).toEqual(['Recently opened paper', 'Older paper'])
  fireEvent.change(screen.getByRole('textbox', { name: '搜索文献' }), { target: { value: 'Recently' } })
  expect(titles()).toEqual(['Recently opened paper'])
  fireEvent.change(screen.getByRole('textbox', { name: '搜索文献' }), { target: { value: '' } })
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
  const width = vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(1200)
  render(<LiteratureLibrary onRead={() => {}} />)
  const left = screen.getByRole('separator', { name: '调整文献集栏宽度' })
  fireEvent.keyDown(left, { key: 'End' })
  expect(left).toHaveAttribute('aria-valuenow', '320')
  fireEvent.keyDown(left, { key: 'ArrowRight' })
  expect(left).toHaveAttribute('aria-valuenow', '320')
  const right = screen.getByRole('separator', { name: '调整资料栏宽度' })
  fireEvent.keyDown(right, { key: 'Home' })
  expect(right).toHaveAttribute('aria-valuenow', '280')
  width.mockReturnValue(700)
  act(() => resize())
  expect(screen.queryByRole('separator', { name: '调整资料栏宽度' })).not.toBeInTheDocument()
  expect(screen.queryByLabelText('文献资料')).not.toBeInTheDocument()
  expect(screen.getByRole('separator', { name: '调整文献集栏宽度' })).toBeVisible()
  width.mockReturnValue(500)
  act(() => resize())
  expect(screen.queryByRole('separator')).not.toBeInTheDocument()
  expect(screen.getByRole('button', { name: '切换文献集' })).toBeVisible()
  width.mockReturnValue(1200)
  act(() => resize())
  expect(screen.getByRole('separator', { name: '调整文献集栏宽度' })).toHaveAttribute('aria-valuenow', '320')
})
