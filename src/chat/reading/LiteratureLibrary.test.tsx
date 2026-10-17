// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { expect, it, vi } from 'vitest'
import LiteratureLibrary from './LiteratureLibrary'
const api = vi.hoisted(() => ({ lookup: vi.fn() }))
vi.mock('pdfjs-dist', () => ({ getDocument: vi.fn() }))
vi.mock('../../api/reading', async (original) => ({
  ...(await original<typeof import('../../api/reading')>()),
  readingApi: api,
}))
vi.mock('./readingLibrary', () => ({
  useReadingLibrary: () => ({
    documents: [],
    collections: [],
    loading: false,
    saving: 0,
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
  fireEvent.click(screen.getByRole('button', { name: '获取并加入文献库' }))
  const dialog = screen.getByRole('dialog')
  const cancellation = new Event('cancel', { cancelable: true })
  fireEvent(dialog, cancellation)
  expect(cancellation.defaultPrevented).toBe(true)
  fail(new Error('not found'))
  await waitFor(() =>
    expect(
      screen.getByRole('button', { name: '获取并加入文献库' }),
    ).toBeEnabled(),
  )
  expect(dialog).toBeVisible()
  expect(
    screen
      .getAllByRole('alert')
      .some((e) => e.textContent?.includes('not found')),
  ).toBe(true)
})
