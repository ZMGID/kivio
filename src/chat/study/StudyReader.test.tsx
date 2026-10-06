import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { StudyReader } from './StudyReader'
import type { StudyMaterial, StudyPage, StudyReaderContext } from './studyMaterial'

const locale = vi.hoisted(() => ({ lang: 'zh' }))
vi.mock('../../components/i18n', () => ({ useLang: () => locale.lang }))

const materialMocks = vi.hoisted(() => ({ openStudyMaterial: vi.fn(), studyPageImage: vi.fn(() => 'data:image/png;base64,AAAA'), studyPageText: vi.fn((page: { label: string }) => page.label) }))
vi.mock('./studyMaterial', async importOriginal => ({ ...await importOriginal<typeof import('./studyMaterial')>(), ...materialMocks }))

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}
function frame(label: string): StudyPage {
  const canvas = document.createElement('canvas')
  canvas.width = 600
  canvas.height = 800
  return Object.assign({ canvas, spans: [], warning: '当前不提供 OCR' }, { label })
}
function material() {
  return { kind: 'pdf' as const, mimeType: 'application/pdf', pageCount: 3, dispose: vi.fn().mockResolvedValue(undefined), renderPage: vi.fn().mockImplementation(async (page: number, signal: AbortSignal) => frame(`Page ${page}${signal.aborted ? " cancelled" : ""}`)) }
}
const blob = new Blob(['PDF'])
function props() { return { blob, page: 1, region: null, onPageChange: vi.fn(), onRegionChange: vi.fn(), onContextChange: vi.fn<(context: StudyReaderContext) => void>() } }

beforeEach(() => {
  vi.clearAllMocks()
  locale.lang = 'zh'
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({ drawImage: vi.fn() } as unknown as CanvasRenderingContext2D)
})
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals() })

describe('StudyReader lifecycle and accessibility', () => {
  it('publishes only the requested page, exposes a readable text alternative, and supports page keys', async () => {
    materialMocks.openStudyMaterial.mockResolvedValue(material())
    const input = props()
    render(<StudyReader {...input} />)
    await waitFor(() => expect(input.onContextChange).toHaveBeenLastCalledWith(expect.objectContaining({ page: 1, text: 'Page 1', status: 'ready' })))
    expect(screen.getByText('Page 1')).toBeInTheDocument()
    expect(screen.getByText(/当前不提供 OCR/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '上一页' })).toBeDisabled()
    const page = screen.getByRole('group', { name: /第 1 页/ })
    fireEvent.keyDown(page, { key: 'PageDown' })
    expect(input.onPageChange).toHaveBeenCalledWith(2)
    expect(input.onRegionChange).not.toHaveBeenCalled() // Page-bound selections remain saved when leaving.
    fireEvent.change(screen.getByRole('spinbutton', { name: '页码' }), { target: { value: '9' } })
    fireEvent.keyDown(screen.getByRole('spinbutton', { name: '页码' }), { key: 'Enter' })
    expect(input.onPageChange).toHaveBeenCalledTimes(1)
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
    expect(input.onContextChange).toHaveBeenLastCalledWith(expect.objectContaining({ page: 2, text: '', imageDataUrl: undefined, status: 'loading' }))
    await act(async () => { second.resolve(frame('Current page 2')) })
    await act(async () => { first.resolve(frame('Late page 1')) })
    expect(input.onContextChange).toHaveBeenLastCalledWith(expect.objectContaining({ page: 2, text: 'Current page 2', status: 'ready' }))
    expect(screen.queryByText('Late page 1')).not.toBeInTheDocument()
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
    expect(input.onContextChange).toHaveBeenLastCalledWith(expect.objectContaining({ status: 'error', text: '' }))
    fireEvent.click(screen.getByRole('button', { name: '重新读取' }))
    await waitFor(() => expect(input.onContextChange).toHaveBeenLastCalledWith(expect.objectContaining({ status: 'ready' })))
    const replacement = vi.fn()
    view.rerender(<StudyReader {...input} onContextChange={replacement} region={{ x: 0.2, y: 0.2, width: 0.3, height: 0.3 }} />)
    expect(replacement).toHaveBeenLastCalledWith(expect.objectContaining({ status: 'ready', region: { x: 0.2, y: 0.2, width: 0.3, height: 0.3 } }))
    expect(materialMocks.openStudyMaterial).toHaveBeenCalledTimes(2)
  })
  it('follows the application language for navigation and extraction controls', async () => {
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
