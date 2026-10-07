import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { StudyReader } from './StudyReader'
import type { StudyMaterial, StudyPage, StudyReaderContext } from './studyMaterial'

const locale = vi.hoisted(() => ({ lang: 'zh' }))
vi.mock('../../components/i18n', () => ({ useLang: () => locale.lang }))

const materialMocks = vi.hoisted(() => ({ openStudyMaterial: vi.fn(), studyPageImage: vi.fn<() => string | undefined>(() => 'data:image/png;base64,AAAA') }))
vi.mock('./studyMaterial', async importOriginal => ({ ...await importOriginal<typeof import('./studyMaterial')>(), ...materialMocks }))

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}
function frame(): StudyPage {
  const canvas = document.createElement('canvas')
  canvas.width = 600
  canvas.height = 800
  return { canvas, warning: 'Rendered page size limits' }
}
function material() {
  return { kind: 'pdf' as const, mimeType: 'application/pdf', pageCount: 3, dispose: vi.fn().mockResolvedValue(undefined), renderPage: vi.fn<(page: number, signal: AbortSignal) => Promise<StudyPage>>().mockImplementation(async () => frame()) }
}
const blob = new Blob(['PDF'])
function props() { return { blob, page: 1, region: null, onPageChange: vi.fn(), onRegionChange: vi.fn(), onContextChange: vi.fn<(context: StudyReaderContext) => void>() } }

beforeEach(() => {
  vi.clearAllMocks()
  locale.lang = 'zh'
  materialMocks.studyPageImage.mockReturnValue('data:image/png;base64,AAAA')
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({ drawImage: vi.fn() } as unknown as CanvasRenderingContext2D)
})
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals() })

describe('StudyReader lifecycle and accessibility', () => {
  it('publishes only the requested page image without extracted text and supports page keys', async () => {
    materialMocks.openStudyMaterial.mockResolvedValue(material())
    const input = props()
    render(<StudyReader {...input} />)
    await waitFor(() => expect(input.onContextChange).toHaveBeenLastCalledWith(expect.objectContaining({ page: 1, imageDataUrl: 'data:image/png;base64,AAAA', status: 'ready' })))
    expect(input.onContextChange.mock.calls.at(-1)![0]).not.toHaveProperty('text')
    expect(input.onContextChange.mock.calls.at(-1)![0]).not.toHaveProperty('textRisk')
    expect(screen.getByRole('img', { name: '材料第 1 页的渲染图像' })).toBeInTheDocument()
    expect(screen.getByText(/不做 OCR、文字提取或公式转写/)).toBeInTheDocument()
    expect(screen.queryByText(/查看本页文字|查看框选区域文字|粘贴修正/)).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: '上一页' })).toBeDisabled()
    const page = screen.getByRole('group', { name: /第 1 页/ })
    fireEvent.keyDown(page, { key: 'PageDown' })
    expect(input.onPageChange).toHaveBeenCalledWith(2)
    expect(input.onRegionChange).not.toHaveBeenCalled() // Page-bound selections remain saved when leaving.
    fireEvent.change(screen.getByRole('spinbutton', { name: '页码' }), { target: { value: '9' } })
    fireEvent.keyDown(screen.getByRole('spinbutton', { name: '页码' }), { key: 'Enter' })
    expect(input.onPageChange).toHaveBeenCalledTimes(1)
  })
  it('zooms the reading view without rerendering or changing the context image', async () => {
    const source = material()
    materialMocks.openStudyMaterial.mockResolvedValue(source)
    const input = props()
    const view = render(<StudyReader {...input} />)
    await screen.findByRole('group', { name: /第 1 页/ })
    expect(screen.getByRole('button', { name: '缩小页面' })).toBeDisabled()
    const context = input.onContextChange.mock.calls.at(-1)![0]
    fireEvent.click(screen.getByRole('button', { name: '放大页面，当前 100%' }))
    expect(view.container.querySelector('.kv-study-reader-paper-wrap')).toHaveStyle({ width: '125%' })
    expect(input.onContextChange.mock.calls.at(-1)![0]).toBe(context)
    expect(source.renderPage).toHaveBeenCalledOnce()
    fireEvent.click(screen.getByRole('button', { name: '缩小页面' }))
    expect(view.container.querySelector('.kv-study-reader-paper-wrap')).toHaveStyle({ width: '100%' })
  })
  it('clears context immediately on page navigation and ignores a late old-page render', async () => {
    const first = deferred<StudyPage>()
    const second = deferred<StudyPage>()
    const source = material()
    source.renderPage.mockImplementation((page: number) => page === 1 ? first.promise : second.promise)
    materialMocks.openStudyMaterial.mockResolvedValue(source)
    const input = props()
    const view = render(<StudyReader {...input} />)
    await waitFor(() => expect(source.renderPage).toHaveBeenCalledWith(1, expect.any(AbortSignal)))
    const previousSignal = source.renderPage.mock.calls[0][1] as AbortSignal
    view.rerender(<StudyReader {...input} page={2} />)
    expect(previousSignal.aborted).toBe(true)
    expect(input.onContextChange).toHaveBeenLastCalledWith(expect.objectContaining({ page: 2, imageDataUrl: undefined, status: 'loading' }))
    const currentFrame = frame()
    const oldFrame = frame()
    await act(async () => { second.resolve(currentFrame) })
    await act(async () => { first.resolve(oldFrame) })
    expect(input.onContextChange).toHaveBeenLastCalledWith(expect.objectContaining({ page: 2, imageDataUrl: 'data:image/png;base64,AAAA', status: 'ready' }))
    expect(materialMocks.studyPageImage).toHaveBeenLastCalledWith(currentFrame, null)
    expect(oldFrame.canvas.width).toBe(0)
  })
  it('does not publish a late source after replacing the file and disposes resources on unmount', async () => {
    const first = deferred<StudyMaterial>()
    const old = material()
    const current = material()
    materialMocks.openStudyMaterial.mockReturnValueOnce(first.promise).mockResolvedValueOnce(current)
    const input = props()
    const view = render(<StudyReader {...input} />)
    const oldSignal = materialMocks.openStudyMaterial.mock.calls[0][1] as AbortSignal
    view.rerender(<StudyReader {...input} blob={new Blob(['New PDF'])} />)
    expect(oldSignal.aborted).toBe(true)
    await waitFor(() => expect(input.onContextChange).toHaveBeenLastCalledWith(expect.objectContaining({ status: 'ready' })))
    await act(async () => { first.resolve(old) })
    expect(old.renderPage).not.toHaveBeenCalled()
    expect(old.dispose).toHaveBeenCalledOnce()
    view.unmount()
    expect(current.dispose).toHaveBeenCalledOnce()
    expect((current.renderPage.mock.calls[0][1] as AbortSignal).aborted).toBe(true)
  })
  it('offers a retry for load failure and uses current callback without reloading the material', async () => {
    const source = material()
    materialMocks.openStudyMaterial.mockRejectedValueOnce(new Error('材料损坏')).mockResolvedValueOnce(source)
    const input = props()
    const view = render(<StudyReader {...input} />)
    expect(await screen.findByRole('alert')).toHaveTextContent('材料损坏')
    expect(input.onContextChange).toHaveBeenLastCalledWith(expect.objectContaining({ status: 'error', imageDataUrl: undefined }))
    fireEvent.click(screen.getByRole('button', { name: '重新读取' }))
    await waitFor(() => expect(input.onContextChange).toHaveBeenLastCalledWith(expect.objectContaining({ status: 'ready' })))
    const replacement = vi.fn()
    view.rerender(<StudyReader {...input} onContextChange={replacement} region={{ x: 0.2, y: 0.2, width: 0.3, height: 0.3 }} />)
    expect(replacement).toHaveBeenLastCalledWith(expect.objectContaining({ status: 'ready', region: { x: 0.2, y: 0.2, width: 0.3, height: 0.3 } }))
    expect(materialMocks.openStudyMaterial).toHaveBeenCalledTimes(2)
  })
  it('follows the application language for navigation and image-source controls', async () => {
    locale.lang = 'en'
    materialMocks.openStudyMaterial.mockResolvedValue(material())
    render(<StudyReader {...props()} />)
    expect(await screen.findByRole('button', { name: 'Select with keyboard' })).toBeInTheDocument()
    expect(screen.getByRole('spinbutton', { name: 'Page number' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Previous page' })).toBeDisabled()
    expect(materialMocks.openStudyMaterial).toHaveBeenCalledWith(blob, expect.any(AbortSignal), 'en')
  })
  it('normalizes a reverse pointer drag and cancels unfinished selection without changing context', async () => {
    vi.stubGlobal('PointerEvent', class extends MouseEvent { pointerId = 1 })
    materialMocks.openStudyMaterial.mockResolvedValue(material())
    const input = props()
    render(<StudyReader {...input} />)
    const group = await screen.findByRole('group', { name: /第 1 页/ })
    vi.spyOn(group, 'getBoundingClientRect').mockReturnValue({ left: 10, top: 20, width: 100, height: 200 } as DOMRect)
    group.setPointerCapture = vi.fn()
    group.hasPointerCapture = vi.fn(() => true)
    group.releasePointerCapture = vi.fn()
    fireEvent.click(screen.getByRole('button', { name: '框选' }))
    fireEvent.pointerDown(group, { button: 0, clientX: 90, clientY: 180 })
    fireEvent.pointerMove(group, { clientX: 30, clientY: 60 })
    expect(input.onRegionChange).not.toHaveBeenCalled()
    fireEvent.pointerUp(group, { clientX: 30, clientY: 60 })
    expect(input.onRegionChange).toHaveBeenLastCalledWith({ x: 0.2, y: 0.2, width: 0.6000000000000001, height: 0.6000000000000001 })
    fireEvent.click(screen.getByRole('button', { name: '框选' }))
    fireEvent.pointerDown(group, { button: 0, clientX: 20, clientY: 40 })
    fireEvent.pointerMove(group, { clientX: 70, clientY: 90 })
    fireEvent.pointerCancel(group)
    expect(input.onRegionChange).toHaveBeenCalledTimes(1)
  })
  it('keeps a scrolled page stationary when starting a pointer selection', async () => {
    vi.stubGlobal('PointerEvent', class extends MouseEvent { pointerId = 1 })
    materialMocks.openStudyMaterial.mockResolvedValue(material())
    const input = props()
    render(<StudyReader {...input} />)
    const group = await screen.findByRole('group', { name: /第 1 页/ })
    let top = -300
    vi.spyOn(group, 'getBoundingClientRect').mockImplementation(() => ({ left: 10, top, width: 600, height: 800 } as DOMRect))
    const focus = vi.spyOn(group, 'focus').mockImplementation(options => {
      // Browsers otherwise scroll a large focusable page into view before the
      // pointer handler calculates its page-relative selection coordinates.
      if (!options?.preventScroll) top = 0
    })
    group.setPointerCapture = vi.fn()
    group.hasPointerCapture = vi.fn(() => true)
    group.releasePointerCapture = vi.fn()
    fireEvent.click(screen.getByRole('button', { name: '框选' }))
    fireEvent.pointerDown(group, { button: 0, clientX: 130, clientY: 340 })
    fireEvent.pointerUp(group, { clientX: 310, clientY: 380 })
    expect(focus).toHaveBeenCalledWith({ preventScroll: true })
    expect(input.onRegionChange).toHaveBeenCalledWith(expect.objectContaining({ x: 0.2, y: 0.8, width: 0.3 }))
    expect(input.onRegionChange.mock.calls[0][0].height).toBeCloseTo(0.05)
  })
  it('supports keyboard-only region creation, movement, resizing and clearing', async () => {
    materialMocks.openStudyMaterial.mockResolvedValue(material())
    const input = props()
    const view = render(<StudyReader {...input} />)
    fireEvent.click(await screen.findByRole('button', { name: '键盘框选' }))
    const selection = { x: 0.2, y: 0.2, width: 0.6, height: 0.3 }
    expect(input.onRegionChange).toHaveBeenLastCalledWith(selection)
    view.rerender(<StudyReader {...input} region={selection} />)
    const group = screen.getByRole('group', { name: /第 1 页/ })
    fireEvent.keyDown(group, { key: 'ArrowRight' })
    expect(input.onRegionChange).toHaveBeenLastCalledWith({ ...selection, x: 0.22 })
    fireEvent.keyDown(group, { key: 'ArrowDown', shiftKey: true })
    expect(input.onRegionChange).toHaveBeenLastCalledWith({ ...selection, height: 0.32 })
    fireEvent.keyDown(group, { key: 'Escape' })
    expect(input.onRegionChange).toHaveBeenLastCalledWith(null)
    expect(input.onPageChange).not.toHaveBeenCalled()
  })
})


describe('StudyReader image-only context and preview', () => {
  it.each(['zh', 'en'])('explains image sources without offering a text workflow in %s', async lang => {
    locale.lang = lang
    materialMocks.openStudyMaterial.mockResolvedValue(material())
    const input = props()
    const view = render(<StudyReader {...input} />)
    await waitFor(() => expect(input.onContextChange).toHaveBeenLastCalledWith(expect.objectContaining({ status: 'ready', imageDataUrl: 'data:image/png;base64,AAAA' })))
    const context = input.onContextChange.mock.calls.at(-1)![0]
    expect(context).not.toHaveProperty('text')
    expect(context).not.toHaveProperty('textRisk')
    expect(screen.getByText(lang === 'zh' ? /不做 OCR、文字提取或公式转写/ : /No OCR, text extraction, or formula transcription/)).toBeInTheDocument()
    expect(view.container.querySelector('.kv-study-reader-extracted')).not.toBeInTheDocument()
    expect(screen.queryByText(/查看本页文字|查看框选区域文字|View page text|View selected text/)).not.toBeInTheDocument()
  })
  it('uses the same image-only contract for an imported image', async () => {
    const source = { ...material(), kind: 'image' as const, mimeType: 'image/png', pageCount: 1 }
    materialMocks.openStudyMaterial.mockResolvedValue(source)
    const input = props()
    render(<StudyReader {...input} />)
    await waitFor(() => expect(input.onContextChange).toHaveBeenLastCalledWith(expect.objectContaining({ status: 'ready', pageCount: 1, imageDataUrl: 'data:image/png;base64,AAAA' })))
    expect(input.onContextChange.mock.calls.at(-1)![0]).not.toHaveProperty('text')
    expect(screen.getByRole('button', { name: '下一页' })).toBeDisabled()
  })
  it('previews the exact selected PNG from the outgoing context, updates it, and clears it during navigation', async () => {
    locale.lang = 'en'
    const source = material()
    materialMocks.openStudyMaterial.mockResolvedValue(source)
    const input = props()
    const region = { x: 0.18, y: 0.8, width: 0.31, height: 0.05 }
    const view = render(<StudyReader {...input} region={region} />)
    const preview = await screen.findByRole('img', { name: 'Page 1 selected area image for question context' })
    const context = input.onContextChange.mock.calls.at(-1)![0]
    expect(preview).toHaveAttribute('src', context.imageDataUrl)
    expect(preview.closest('details')).toHaveAttribute('open')
    expect(screen.getByText(/same PNG.*question context/i)).toHaveTextContent(/sent with your question.*selected vision-capable model/i)
    expect(materialMocks.studyPageImage).toHaveBeenLastCalledWith(expect.anything(), region)

    materialMocks.studyPageImage.mockReturnValue('data:image/png;base64,Q1JPUFRXTw==')
    view.rerender(<StudyReader {...input} region={{ ...region, width: 0.2 }} />)
    expect(preview).toHaveAttribute('src', 'data:image/png;base64,Q1JPUFRXTw==')
    expect(input.onContextChange.mock.calls.at(-1)![0].imageDataUrl).toBe(preview.getAttribute('src'))

    view.rerender(<StudyReader {...input} page={2} />)
    expect(screen.queryByRole('img', { name: /selected area image for question context/ })).not.toBeInTheDocument()
    expect(input.onContextChange).toHaveBeenLastCalledWith(expect.objectContaining({ page: 2, status: 'loading', imageDataUrl: undefined }))
    await waitFor(() => expect(input.onContextChange).toHaveBeenLastCalledWith(expect.objectContaining({ page: 2, status: 'ready' })))
    expect(screen.getByAltText('Page 2 full page image for question context').closest('details')).not.toHaveAttribute('open')
  })
  it('omits an unavailable image preview without claiming an image will be attached', async () => {
    locale.lang = 'en'
    materialMocks.studyPageImage.mockReturnValue(undefined)
    const source = material()
    source.renderPage.mockResolvedValue({ canvas: frame().canvas })
    materialMocks.openStudyMaterial.mockResolvedValue(source)
    const input = props()
    const view = render(<StudyReader {...input} region={{ x: 0.1, y: 0.1, width: 0.3, height: 0.2 }} />)
    await waitFor(() => expect(input.onContextChange).toHaveBeenLastCalledWith(expect.objectContaining({ status: 'ready', imageDataUrl: undefined })))
    expect(view.container.querySelector('.kv-study-reader-preview')).not.toBeInTheDocument()
    expect(screen.getByText(/context image could not be generated within the size limit/i)).toBeInTheDocument()
  })
})
