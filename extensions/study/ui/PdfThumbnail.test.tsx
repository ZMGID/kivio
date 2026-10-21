import { act, cleanup, render, screen, waitFor } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { PdfThumbnail } from './PdfThumbnail'
import { readingApi } from '../api/reading'

vi.mock('../api/reading', () => ({ readingApi: { pdf: vi.fn() } }))
vi.mock('./pdfDocument', () => ({ loadReadingPdf: () => ({
  promise: Promise.resolve({ getPage: async () => ({ getViewport: () => ({ width: 120, height: 160 }), render: () => ({ promise: Promise.resolve() }) }) }),
  destroy: async () => {},
}) }))
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals() })

it('skips an offscreen queued preview and loads it when it becomes visible again', async () => {
  const observers = new Map<Element, (entries: { isIntersecting: boolean }[]) => void>()
  vi.stubGlobal('IntersectionObserver', class {
    constructor(private callback: (entries: { isIntersecting: boolean }[]) => void) {}
    observe(element: Element) { observers.set(element, this.callback) }
    disconnect() {}
  })
  vi.spyOn(HTMLCanvasElement.prototype, 'toDataURL').mockReturnValue('data:image/png;base64,preview')
  let release!: (bytes: Awaited<ReturnType<typeof readingApi.pdf>>) => void
  vi.mocked(readingApi.pdf).mockImplementation(id => id === 'first' ? new Promise(resolve => { release = resolve }) : Promise.resolve(new Uint8Array([2])))
  render(<><PdfThumbnail id="first" name="First" /><PdfThumbnail id="second" name="Second" /></>)
  const [first, second] = [...observers.values()]
  act(() => first([{ isIntersecting: true }]))
  await waitFor(() => expect(release).toBeTypeOf('function'))
  act(() => { second([{ isIntersecting: true }]); second([{ isIntersecting: false }]) })
  await act(async () => release(new Uint8Array([1])))
  await screen.findByAltText('First 的 PDF 首页')
  expect(readingApi.pdf).not.toHaveBeenCalledWith('second')
  act(() => second([{ isIntersecting: true }]))
  await screen.findByAltText('Second 的 PDF 首页')
  expect(readingApi.pdf).toHaveBeenCalledWith('second')
})
