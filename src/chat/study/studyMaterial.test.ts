import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { loadStudyMaterial, normalizeStudyRegion, openStudyMaterial, STUDY_MATERIAL_LIMITS, studyPageImage, studyRegionFromPoints, studyRenderSize, validateStudyMaterial, type StudyPage } from './studyMaterial'

const pdfMocks = vi.hoisted(() => ({ getDocument: vi.fn(), GlobalWorkerOptions: { workerSrc: '' } }))
vi.mock('pdfjs-dist/legacy/build/pdf.mjs', () => pdfMocks)

const pdfBlob = () => new Blob(['%PDF-1.7\n'])
function pngBlob(width = 100, height = 200) {
  const bytes = new Uint8Array(24)
  bytes.set([137, 80, 78, 71, 13, 10, 26, 10])
  const view = new DataView(bytes.buffer)
  view.setUint32(16, width)
  view.setUint32(20, height)
  return new Blob([bytes], { type: 'image/png' })
}
function mockPdf(pages = 2) {
  const destroy = vi.fn().mockResolvedValue(undefined)
  const cancel = vi.fn()
  const cleanup = vi.fn()
  const render = vi.fn(() => ({ promise: Promise.resolve(), cancel }))
  const textApi = vi.fn(() => { throw new Error('Study must never access PDF text APIs') })
  const getPage = vi.fn().mockImplementation(async () => ({
    cleanup,
    getViewport: ({ scale }: { scale: number }) => ({ width: 600 * scale, height: 800 * scale }),
    render,
    get streamTextContent() { return textApi() },
    get getTextContent() { return textApi() },
  }))
  pdfMocks.getDocument.mockReturnValue({ promise: Promise.resolve({ numPages: pages, getPage }), destroy })
  return { destroy, getPage, render, cancel, cleanup, textApi }
}
function mockCanvas(dataUrl = 'data:image/png;base64,AAAA') {
  const drawImage = vi.fn()
  const canvas = { width: 0, height: 0, getContext: () => ({ drawImage, fillRect: vi.fn() }), toDataURL: vi.fn(() => dataUrl) } as unknown as HTMLCanvasElement
  vi.stubGlobal('document', { createElement: () => canvas })
  return { canvas, drawImage }
}

beforeEach(() => vi.clearAllMocks())
afterEach(() => vi.unstubAllGlobals())

describe('study material validation and resource limits', () => {
  it('recognizes bytes, allowing a PDF with a missing or incorrect MIME label', async () => {
    expect((await validateStudyMaterial(new Blob(['%PDF-1.7'], { type: 'application/octet-stream' }))).kind).toBe('pdf')
    expect((await validateStudyMaterial(pngBlob())).mimeType).toBe('image/png')
  })
  it('rejects spoofed extensions/types, empty files, oversized files and image pixel bombs before decoding', async () => {
    await expect(validateStudyMaterial(new Blob(['<script>bad</script>'], { type: 'application/pdf' }))).rejects.toThrow('仅支持')
    await expect(validateStudyMaterial(new Blob([]))).rejects.toThrow('为空')
    await expect(validateStudyMaterial(new Blob([new Uint8Array(STUDY_MATERIAL_LIMITS.fileBytes + 1)]))).rejects.toThrow('25 MB')
    await expect(validateStudyMaterial(pngBlob(10000, 10000))).rejects.toThrow('1600')
  })
  it('rejects corrupt image data at import even when its signature is valid', async () => {
    class BrokenImage {
      onerror: (() => void) | null = null
      onload: (() => void) | null = null
      set src(value: string) { if (!value) return; queueMicrotask(() => this.onerror?.()) }
    }
    vi.stubGlobal('Image', BrokenImage)
    await expect(loadStudyMaterial(pngBlob(), undefined, 'en')).rejects.toThrow('cannot be decoded')
  })
  it('decodes images before importing and renders pixels without a text representation', async () => {
    class ValidImage {
      onerror: (() => void) | null = null
      onload: (() => void) | null = null
      naturalWidth = 100
      naturalHeight = 200
      set src(value: string) { if (!value) return; queueMicrotask(() => this.onload?.()) }
    }
    vi.stubGlobal('Image', ValidImage)
    const { drawImage } = mockCanvas()
    await expect(loadStudyMaterial(pngBlob())).resolves.toEqual({ kind: 'image', mimeType: 'image/png', pageCount: 1 })
    const image = await openStudyMaterial(pngBlob(), undefined, 'en')
    const rendered = await image.renderPage(1, new AbortController().signal)
    expect(drawImage).toHaveBeenLastCalledWith(expect.any(ValidImage), 0, 0, 200, 400)
    expect(rendered.warning).toContain('2,400')
    expect(rendered).not.toHaveProperty('spans')
    expect(rendered).not.toHaveProperty('text')
  })
  it('rejects cancellation before reading', async () => {
    const controller = new AbortController()
    controller.abort()
    await expect(validateStudyMaterial(pdfBlob(), controller.signal)).rejects.toMatchObject({ name: 'AbortError' })
  })
  it('bounds canvas dimensions and rejects invalid dimensions', () => {
    const size = studyRenderSize(100000, 200000)
    expect(size.width * size.height).toBeLessThanOrEqual(STUDY_MATERIAL_LIMITS.renderPixels)
    expect(Math.max(size.width, size.height)).toBeLessThanOrEqual(STUDY_MATERIAL_LIMITS.renderEdge)
    expect(() => studyRenderSize(Infinity, 10)).toThrow('尺寸无效')
    expect(() => studyRenderSize(10, 0)).toThrow('尺寸无效')
  })
  it('destroys PDFs after metadata validation and rejects excessive page counts', async () => {
    const valid = mockPdf(263)
    await expect(loadStudyMaterial(pdfBlob())).resolves.toEqual({ kind: 'pdf', mimeType: 'application/pdf', pageCount: 263 })
    expect(valid.destroy).toHaveBeenCalledOnce()
    const excessive = mockPdf(1001)
    await expect(loadStudyMaterial(pdfBlob())).rejects.toThrow('1000')
    expect(excessive.destroy).toHaveBeenCalledOnce()
    expect(pdfMocks.getDocument).toHaveBeenCalledWith(expect.objectContaining({ isEvalSupported: false, enableXfa: false, useWorkerFetch: false, maxImageSize: STUDY_MATERIAL_LIMITS.imagePixels }))
  })
  it('reports encrypted files honestly and destroys a failed loading task', async () => {
    const destroy = vi.fn().mockResolvedValue(undefined)
    const error = Object.assign(new Error('password needed'), { name: 'PasswordException' })
    pdfMocks.getDocument.mockImplementation(() => ({ promise: Promise.reject(error), destroy }))
    await expect(loadStudyMaterial(pdfBlob())).rejects.toThrow('加密 PDF')
    expect(destroy).toHaveBeenCalledOnce()
  })
  it('renders only the requested PDF page without accessing text APIs and releases its resources', async () => {
    const mocked = mockPdf()
    mockCanvas()
    const material = await openStudyMaterial(pdfBlob())
    const page = await material.renderPage(2, new AbortController().signal)
    expect(mocked.getPage).toHaveBeenCalledWith(2)
    expect(page.canvas.width).toBe(1200)
    expect(page.canvas.height).toBe(1600)
    expect(mocked.render).toHaveBeenCalledWith({ canvas: page.canvas, viewport: { width: 1200, height: 1600 }, background: '#fff' })
    expect(page).not.toHaveProperty('spans')
    expect(page).not.toHaveProperty('text')
    expect(page.warning).toContain('2400')
    expect(mocked.textApi).not.toHaveBeenCalled()
    expect(mocked.cleanup).toHaveBeenCalledOnce()
    await expect(material.renderPage(3, new AbortController().signal)).rejects.toThrow('页码')
    await material.dispose()
    expect(mocked.destroy).toHaveBeenCalledOnce()
  })
  it('cancels in-flight PDF canvas work instead of publishing its result', async () => {
    mockCanvas()
    let rejectRender: (error: Error) => void = () => undefined
    const cancel = vi.fn(() => rejectRender(new DOMException('Cancelled', 'AbortError')))
    const cleanup = vi.fn()
    const getPage = vi.fn().mockResolvedValue({ cleanup, getViewport: () => ({ width: 600, height: 800 }), render: () => ({ promise: new Promise((_, reject) => { rejectRender = reject }), cancel }) })
    pdfMocks.getDocument.mockReturnValue({ promise: Promise.resolve({ numPages: 2, getPage }), destroy: vi.fn().mockResolvedValue(undefined) })
    const material = await openStudyMaterial(pdfBlob())
    const controller = new AbortController()
    const pending = material.renderPage(1, controller.signal)
    await vi.waitFor(() => expect(getPage).toHaveBeenCalled())
    controller.abort()
    await expect(pending).rejects.toMatchObject({ name: 'AbortError' })
    expect(cancel).toHaveBeenCalledOnce()
    expect(cleanup).toHaveBeenCalledOnce()
  })
})

describe('study region context', () => {
  it('normalizes reverse dragging and bounds malformed regions', () => {
    expect(studyRegionFromPoints({ x: 0.8, y: 0.9 }, { x: 0.2, y: 0.3 })).toEqual({ x: 0.2, y: 0.3, width: 0.6000000000000001, height: 0.6000000000000001 })
    expect(normalizeStudyRegion({ x: 0.9, y: 0.9, width: 2, height: 2 })).toEqual({ x: 0.9, y: 0.9, width: 0.09999999999999998, height: 0.09999999999999998 })
    expect(normalizeStudyRegion({ x: NaN, y: 0, width: 1, height: 1 })).toBeNull()
    expect(normalizeStudyRegion({ x: 0, y: 0, width: 0.001, height: 1 })).toBeNull()
  })
  it('crops the normalized source pixels and releases temporary canvases', () => {
    const { canvas, drawImage } = mockCanvas()
    const source = { width: 1000, height: 2000 } as HTMLCanvasElement
    expect(studyPageImage({ canvas: source }, { x: 0.1, y: 0.2, width: 0.3, height: 0.4 })).toBe('data:image/png;base64,AAAA')
    expect(drawImage).toHaveBeenCalledWith(source, 100, 400, 300, 800, 0, 0, 300, 800)
    expect(canvas.width).toBe(0)
  })
  it('uses the entire page raster for full-page context and bounds its PNG dimensions', () => {
    const { canvas, drawImage } = mockCanvas()
    const source = { width: 1200, height: 2400 } as HTMLCanvasElement
    expect(studyPageImage({ canvas: source }, null)).toBe('data:image/png;base64,AAAA')
    expect(drawImage).toHaveBeenCalledWith(source, 0, 0, 1200, 2400, 0, 0, 800, 1600)
    expect(canvas.toDataURL).toHaveBeenCalledWith('image/png')
    expect(canvas.height).toBe(0)
    expect(source).toEqual({ width: 1200, height: 2400 })
  })
  it('never accepts an external URL or non-PNG data as a context image', () => {
    for (const url of ['https://example.invalid/source.png', 'data:image/svg+xml,<svg />', 'data:image/png;base64,']) {
      mockCanvas(url)
      expect(studyPageImage({ canvas: { width: 100, height: 100 } as HTMLCanvasElement }, null)).toBeUndefined()
    }
  })
  it('bounds image serialization retries and omits an unbounded attachment', () => {
    const { canvas } = mockCanvas(`data:image/png;base64,${'A'.repeat(3_000_000)}`)
    expect(studyPageImage({ canvas: { width: 100, height: 100 } as HTMLCanvasElement } satisfies StudyPage, null)).toBeUndefined()
    expect(canvas.toDataURL).toHaveBeenCalledTimes(5)
  })
})
