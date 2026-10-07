import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { loadStudyMaterial, normalizeStudyRegion, openStudyMaterial, STUDY_MATERIAL_LIMITS, studyPageImage, studyPageText, studyTextExtractionRisk, studyTextExtractionNotice, studyRegionFromPoints, studyRenderSize, validateStudyMaterial, type StudyPage } from './studyMaterial'

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
  const readCancel = vi.fn().mockResolvedValue(undefined)
  const getPage = vi.fn().mockImplementation(async (number: number) => ({
    cleanup,
    getViewport: ({ scale }: { scale: number }) => ({ width: 600 * scale, height: 800 * scale, convertToViewportRectangle: (rect: number[]) => rect.map(value => value * scale) }),
    render: vi.fn(() => ({ promise: Promise.resolve(), cancel })),
    streamTextContent: () => ({ getReader: () => ({ read: vi.fn().mockResolvedValueOnce({ value: { items: [{ str: `Exact page ${number}`, transform: [1, 0, 0, 1, 10, 20], width: 80, height: 12, hasEOL: true }] }, done: false }).mockResolvedValue({ done: true }), cancel: readCancel, releaseLock: vi.fn() }) }),
  }))
  pdfMocks.getDocument.mockReturnValue({ promise: Promise.resolve({ numPages: pages, getPage }), destroy })
  return { destroy, getPage, cancel, cleanup, readCancel }
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
  it('decodes images before importing and honestly reports the absence of OCR', async () => {
    class ValidImage {
      onerror: (() => void) | null = null
      onload: (() => void) | null = null
      naturalWidth = 100
      naturalHeight = 200
      set src(value: string) { if (!value) return; queueMicrotask(() => this.onload?.()) }
    }
    vi.stubGlobal('Image', ValidImage)
    mockCanvas()
    await expect(loadStudyMaterial(pngBlob())).resolves.toEqual({ kind: 'image', mimeType: 'image/png', pageCount: 1 })
    const image = await openStudyMaterial(pngBlob(), undefined, 'en')
    const rendered = await image.renderPage(1, new AbortController().signal)
    expect(rendered.warning).toContain('OCR is not available')
    expect(studyPageText(rendered, null)).toBe('')
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
  it('extracts the requested PDF page only and releases page/text resources', async () => {
    const mocked = mockPdf()
    mockCanvas()
    const material = await openStudyMaterial(pdfBlob())
    const page = await material.renderPage(2, new AbortController().signal)
    expect(mocked.getPage).toHaveBeenCalledWith(2)
    expect(studyPageText(page, null)).toBe('Exact page 2')
    expect(page.warning).toContain('不提供 OCR')
    expect(mocked.cleanup).toHaveBeenCalledOnce()
    expect(mocked.readCancel).toHaveBeenCalledOnce()
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
  it('includes only overlapping text for a region and never calls the whole page a crop', () => {
    const page = { canvas: {} as HTMLCanvasElement, spans: [{ text: 'First', lineBreak: true, region: { x: 0.1, y: 0.1, width: 0.1, height: 0.1 } }, { text: 'Second', lineBreak: true, region: { x: 0.7, y: 0.7, width: 0.1, height: 0.1 } }] }
    expect(studyPageText(page, { x: 0, y: 0, width: 0.3, height: 0.3 })).toBe('First')
    expect(studyPageText(page, { x: 0.4, y: 0.4, width: 0.1, height: 0.1 })).toBe('')
  })
  it('crops the normalized source pixels and releases temporary canvases', () => {
    const { canvas, drawImage } = mockCanvas()
    const source = { width: 1000, height: 2000 } as HTMLCanvasElement
    expect(studyPageImage({ canvas: source, spans: [] }, { x: 0.1, y: 0.2, width: 0.3, height: 0.4 })).toBe('data:image/png;base64,AAAA')
    expect(drawImage).toHaveBeenCalledWith(source, 100, 400, 300, 800, 0, 0, 300, 800)
    expect(canvas.width).toBe(0)
  })
  it('never accepts an external URL or non-PNG data as a context image', () => {
    for (const url of ['https://example.invalid/source.png', 'data:image/svg+xml,<svg />', 'data:image/png;base64,']) {
      mockCanvas(url)
      expect(studyPageImage({ canvas: { width: 100, height: 100 } as HTMLCanvasElement, spans: [] }, null)).toBeUndefined()
    }
  })
  it('bounds image serialization retries and omits an unbounded attachment', () => {
    const { canvas } = mockCanvas(`data:image/png;base64,${'A'.repeat(3_000_000)}`)
    expect(studyPageImage({ canvas: { width: 100, height: 100 } as HTMLCanvasElement, spans: [] } satisfies StudyPage, null)).toBeUndefined()
    expect(canvas.toDataURL).toHaveBeenCalledTimes(5)
  })
})


describe('PDF text extraction risk', () => {
  it('flags the replacement glyph in the observed MIT 18.01SC exercise 5B-13 extraction', () => {
    // Actual PDF.js text from page 2, selection x=.18, y=.80, width=.31, height=.05.
    // The rendered source has an integral and a fraction; plain text loses both.
    const extracted = '� \nx 2 dx \n5B-13.   .   Hint:   Try   u   =   x 3   . \n1 +   x 6'
    expect(studyTextExtractionRisk(extracted)).toBe('unmapped-glyphs')
    expect(studyTextExtractionNotice('unmapped-glyphs', 'en')).toMatch(/missing or unmapped characters/i)
  })
  it.each(['', ' \t\r\n '])('flags empty extraction without asserting the page is scanned: %j', text => {
    expect(studyTextExtractionRisk(text)).toBe('empty')
    expect(studyTextExtractionNotice('empty', 'en')).not.toMatch(/scanned/i)
  })
  it.each(['x\u0000y', 'x\u0007y', 'x\u0085y', 'x\u007fy', 'x\uE000y', 'x\u{F0000}y', 'x\u{100000}y', 'x\uFFFCy', 'x\uFFFFy', 'x\uD800y'])('flags only observable invalid or unmapped character evidence: %j', text => {
    expect(studyTextExtractionRisk(text)).toBe('unmapped-glyphs')
  })
  it.each(['Read the paragraph.\nThen explain it.\t第 2 页', '∫ x²/(1+x⁶) dx; u=x³; ∑ αᵢ ≤ ∞; □', String.raw`\int \frac{x^2}{1+x^6} \, dx`, 'x 2 dx \n1 + x 6'])('does not certify or reject valid-looking text and mathematical layouts: %j', text => {
    expect(studyTextExtractionRisk(text)).toBeUndefined()
    expect(studyTextExtractionNotice(undefined, 'en')).toBeUndefined()
  })
  it.each(['empty', 'unmapped-glyphs'] as const)('gives localized correction and vision fallback actions for %s', risk => {
    expect(studyTextExtractionNotice(risk, 'en')).toMatch(/check formulas against the page/i)
    expect(studyTextExtractionNotice(risk, 'en')).toMatch(/paste corrected text/i)
    expect(studyTextExtractionNotice(risk, 'en')).toMatch(/selected image.*vision-capable model/i)
    expect(studyTextExtractionNotice(risk, 'zh')).toContain('核对公式')
    expect(studyTextExtractionNotice(risk, 'zh')).toContain('粘贴修正文字')
    expect(studyTextExtractionNotice(risk, 'zh')).toContain('支持图片的模型')
  })
})
