import type { PDFDocumentProxy, PDFPageProxy } from 'pdfjs-dist'
import pdfWorkerUrl from 'pdfjs-dist/legacy/build/pdf.worker.min.mjs?url'

export const STUDY_MATERIAL_LIMITS = {
  fileBytes: 25 * 1024 * 1024,
  pages: 1000,
  imagePixels: 16_000_000,
  renderPixels: 4_000_000,
  renderEdge: 2400,
  textCharacters: 16_000,
  contextImageBytes: 1_500_000,
} as const

type MaterialLanguage = 'zh' | 'en'
const materialMessages = {
  empty: ['材料为空，请选择有效文件', 'The material is empty. Choose a valid file.'],
  fileLimit: ['材料不能超过 25 MB', 'Material must be 25 MB or smaller.'],
  types: ['仅支持有效的 PDF、PNG、JPEG 和 WebP 文件', 'Choose a valid PDF, PNG, JPEG, or WebP file.'],
  imageSize: ['无法读取图片尺寸，请重新导出为 PNG、JPEG 或 WebP', 'Image dimensions are unreadable. Export it again as PNG, JPEG, or WebP.'],
  imageLimit: ['图片过大，请缩小到 1600 万像素以内', 'Image is too large. Resize it to 16 megapixels or less.'],
  canvas: ['当前环境无法显示材料，请使用文字输入', 'This environment cannot display the material. Use the text input.'],
  decode: ['图片无法解码，请重新导出', 'The image cannot be decoded. Export it again.'],
  noImageText: ['图片没有可提取的文字。当前不提供 OCR；请粘贴或修正题目文字，或选择支持图片的模型。', 'Images have no extractable text. OCR is not available. Paste or correct the problem text, or choose a vision-capable model.'],
  pageRange: ['页码超出材料范围', 'The page number is outside this material.'],
  extraction: ['PDF 文字为尽力提取，公式、阅读顺序和框选范围可能不准确；请核对并修正。当前不提供 OCR。', 'PDF text extraction is best-effort. Formulas, reading order, and crop boundaries may be inaccurate; check and correct them. OCR is not available.'],
  truncated: [' 本页文字已截取前 16000 字。', ' Page text is limited to the first 16,000 characters.'],
  renderLimit: [' 为控制内存，超过 1600 万像素的内嵌图片可能不显示。', ' To limit memory use, embedded images above 16 megapixels may be omitted.'],
  noPdfText: ['这一页没有可提取的文字，可能是扫描件。当前不提供 OCR；请粘贴或修正题目文字，或选择支持图片的模型。', 'This page has no extractable text and may be scanned. OCR is not available. Paste or correct the problem text, or choose a vision-capable model.'],
  missingGlyphs: ['提取文字中发现缺失或未映射字符，公式可能不完整。请对照页面核对公式，再粘贴修正文字，或使用支持图片的模型发送选区图片。', 'Extracted text contains missing or unmapped characters; formulas may be incomplete. Check formulas against the page, then paste corrected text or send the selected image with a vision-capable model.'],
  emptySelection: ['当前页面或选区没有可提取的文字。请对照页面核对公式，再粘贴修正文字，或使用支持图片的模型发送选区图片。当前不提供 OCR。', 'No text could be extracted from this page or selection. Check formulas against the page, then paste corrected text or send the selected image with a vision-capable model. OCR is not available.'],
  extractionFailed: ['本页文字提取失败。当前不提供 OCR；请粘贴或修正题目文字，或选择支持图片的模型。', 'Text extraction failed for this page. OCR is not available. Paste or correct the problem text, or choose a vision-capable model.'],
  imagePage: ['图片只有一页', 'An image has only one page.'],
  pageLimit: ['PDF 最多支持 1000 页，请拆分后导入', 'PDFs support up to 1000 pages. Split the file before importing.'],
  encrypted: ['暂不支持加密 PDF，请解锁后重新导入', 'Encrypted PDFs are not supported. Unlock the file before importing.'],
  pdfFailed: ['PDF 无法打开，请检查文件是否损坏或加密', 'The PDF cannot be opened. Check whether it is damaged or encrypted.'],
} as const
const message = (key: keyof typeof materialMessages, lang: MaterialLanguage) => materialMessages[key][lang === 'zh' ? 0 : 1]

export type StudyTextExtractionRisk = 'empty' | 'unmapped-glyphs'
export type StudyRegion = { x: number; y: number; width: number; height: number }
export type StudyMaterialInfo = { kind: 'pdf' | 'image'; pageCount: number; mimeType: string }
export type StudyReaderContext = {
  page: number
  pageCount: number
  text: string
  imageDataUrl?: string
  region: StudyRegion | null
  status: 'loading' | 'ready' | 'error'
  warning?: string
  textRisk?: StudyTextExtractionRisk
  error?: string
}
type TextSpan = { text: string; region: StudyRegion; lineBreak: boolean }
export type StudyPage = {
  canvas: HTMLCanvasElement
  spans: TextSpan[]
  warning?: string
}
export type StudyMaterial = StudyMaterialInfo & {
  renderPage: (page: number, signal: AbortSignal) => Promise<StudyPage>
  dispose: () => Promise<void>
}

const cMapUrls = import.meta.glob('/node_modules/pdfjs-dist/cmaps/*.bcmap', { query: '?url', import: 'default', eager: true }) as Record<string, string>
const fontUrls = import.meta.glob('/node_modules/pdfjs-dist/standard_fonts/*', { query: '?url', import: 'default', eager: true }) as Record<string, string>

// PDF assets are bundled locally; imported documents never choose network URLs.
class LocalCMapReader {
  async fetch({ name }: { name: string }) {
    const url = cMapUrls[`/node_modules/pdfjs-dist/cmaps/${name}.bcmap`]
    if (!url) throw new Error('Unsupported PDF character map')
    const response = await fetch(url)
    if (!response.ok) throw new Error('PDF character map unavailable')
    return { cMapData: new Uint8Array(await response.arrayBuffer()), compressionType: 1 }
  }
}
class LocalFontReader {
  async fetch({ filename }: { filename: string }) {
    const url = fontUrls[`/node_modules/pdfjs-dist/standard_fonts/${filename}`]
    if (!url) throw new Error('Unsupported PDF font')
    const response = await fetch(url)
    if (!response.ok) throw new Error('PDF font unavailable')
    return new Uint8Array(await response.arrayBuffer())
  }
}

function checkAbort(signal?: AbortSignal) {
  if (signal?.aborted) throw new DOMException('已取消读取', 'AbortError')
}

export function normalizeStudyRegion(region: StudyRegion | null): StudyRegion | null {
  if (!region || !Object.values(region).every(Number.isFinite)) return null
  const x = Math.max(0, Math.min(1, region.x))
  const y = Math.max(0, Math.min(1, region.y))
  const width = Math.max(0, Math.min(1 - x, region.width))
  const height = Math.max(0, Math.min(1 - y, region.height))
  return width >= 0.005 && height >= 0.005 ? { x, y, width, height } : null
}

export function studyRegionFromPoints(a: { x: number; y: number }, b: { x: number; y: number }) {
  return normalizeStudyRegion({ x: Math.min(a.x, b.x), y: Math.min(a.y, b.y), width: Math.abs(a.x - b.x), height: Math.abs(a.y - b.y) })
}

export function studyRenderSize(width: number, height: number) {
  if (!Number.isFinite(width) || !Number.isFinite(height) || width <= 0 || height <= 0) throw new Error('材料尺寸无效')
  const scale = Math.min(2, STUDY_MATERIAL_LIMITS.renderEdge / Math.max(width, height), Math.sqrt(STUDY_MATERIAL_LIMITS.renderPixels / (width * height)))
  return { width: Math.max(1, Math.floor(width * scale)), height: Math.max(1, Math.floor(height * scale)), scale }
}

function imageDimensions(bytes: Uint8Array, mimeType: string, lang: MaterialLanguage) {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  if (mimeType === 'image/png' && bytes.length >= 24) return [view.getUint32(16), view.getUint32(20)]
  if (mimeType === 'image/jpeg') {
    let offset = 2
    while (offset + 9 <= bytes.length) {
      if (bytes[offset] !== 0xff) break
      const marker = bytes[offset + 1]
      if (marker === 0xd9 || marker === 0xda) break
      if (marker === 0xd8 || marker === 0x01 || (marker >= 0xd0 && marker <= 0xd7)) { offset += 2; continue }
      const length = view.getUint16(offset + 2)
      if (length < 2) break
      if ([0xc0, 0xc1, 0xc2, 0xc3, 0xc5, 0xc6, 0xc7, 0xc9, 0xca, 0xcb, 0xcd, 0xce, 0xcf].includes(marker)) return [view.getUint16(offset + 7), view.getUint16(offset + 5)]
      offset += 2 + length
    }
  }
  if (mimeType === 'image/webp' && bytes.length >= 25) {
    const tag = new TextDecoder().decode(bytes.subarray(12, 16))
    const uint24 = (offset: number) => bytes[offset] + (bytes[offset + 1] << 8) + (bytes[offset + 2] << 16)
    if (tag === 'VP8X' && bytes.length >= 30) return [uint24(24) + 1, uint24(27) + 1]
    if (tag === 'VP8 ' && bytes.length >= 30 && bytes[23] === 0x9d && bytes[24] === 0x01 && bytes[25] === 0x2a) return [view.getUint16(26, true) & 0x3fff, view.getUint16(28, true) & 0x3fff]
    if (tag === 'VP8L' && bytes[20] === 0x2f) return [1 + (((bytes[22] & 0x3f) << 8) | bytes[21]), 1 + (((bytes[24] & 0x0f) << 10) | (bytes[23] << 2) | (bytes[22] >> 6))]
  }
  throw new Error(message('imageSize', lang))
}

/** Validate byte signatures, not the filename or an untrusted browser MIME label. */
export async function validateStudyMaterial(blob: Blob, signal?: AbortSignal, lang: MaterialLanguage = 'zh') {
  checkAbort(signal)
  if (blob.size === 0) throw new Error(message('empty', lang))
  if (blob.size > STUDY_MATERIAL_LIMITS.fileBytes) throw new Error(message('fileLimit', lang))
  const bytes = new Uint8Array(await blob.arrayBuffer())
  checkAbort(signal)
  let mimeType = ''
  if (new TextDecoder().decode(bytes.subarray(0, 5)) === '%PDF-') mimeType = 'application/pdf'
  else if ([137, 80, 78, 71, 13, 10, 26, 10].every((value, index) => bytes[index] === value)) mimeType = 'image/png'
  else if (bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff) mimeType = 'image/jpeg'
  else if (new TextDecoder().decode(bytes.subarray(0, 4)) === 'RIFF' && new TextDecoder().decode(bytes.subarray(8, 12)) === 'WEBP') mimeType = 'image/webp'
  if (!mimeType) throw new Error(message('types', lang))
  if (mimeType !== 'application/pdf') {
    const [width, height] = imageDimensions(bytes, mimeType, lang)
    if (!width || !height || width * height > STUDY_MATERIAL_LIMITS.imagePixels || Math.max(width, height) > 16_000) throw new Error(message('imageLimit', lang))
  }
  return { bytes, mimeType, kind: mimeType === 'application/pdf' ? 'pdf' as const : 'image' as const }
}

function makeCanvas(width: number, height: number, lang: MaterialLanguage = 'zh') {
  const canvas = document.createElement('canvas')
  canvas.width = width
  canvas.height = height
  const context = canvas.getContext('2d')
  if (!context) throw new Error(message('canvas', lang))
  return { canvas, context }
}

async function renderImage(blob: Blob, signal: AbortSignal, lang: MaterialLanguage): Promise<StudyPage> {
  checkAbort(signal)
  const url = URL.createObjectURL(blob)
  const image = new Image()
  const cancel = () => { image.src = ''; image.onerror?.(new Event('error')) }
  try {
    await new Promise<void>((resolve, reject) => {
      image.onload = () => resolve()
      image.onerror = () => reject(signal.aborted ? new DOMException('已取消读取', 'AbortError') : new Error(message('decode', lang)))
      signal.addEventListener('abort', cancel, { once: true })
      image.src = url
    })
    checkAbort(signal)
    if (image.naturalWidth * image.naturalHeight > STUDY_MATERIAL_LIMITS.imagePixels) throw new Error(message('imageLimit', lang))
    const size = studyRenderSize(image.naturalWidth, image.naturalHeight)
    const { canvas, context } = makeCanvas(size.width, size.height, lang)
    context.fillStyle = '#fff'
    context.fillRect(0, 0, canvas.width, canvas.height)
    context.drawImage(image, 0, 0, canvas.width, canvas.height)
    return { canvas, spans: [], warning: message('noImageText', lang) }
  } finally {
    signal.removeEventListener('abort', cancel)
    image.onload = null
    image.onerror = null
    URL.revokeObjectURL(url)
  }
}

async function renderPdfPage(pdf: PDFDocumentProxy, pageNumber: number, signal: AbortSignal, lang: MaterialLanguage): Promise<StudyPage> {
  checkAbort(signal)
  if (!Number.isInteger(pageNumber) || pageNumber < 1 || pageNumber > pdf.numPages) throw new Error(message('pageRange', lang))
  let page: PDFPageProxy | undefined
  try {
    page = await pdf.getPage(pageNumber)
    checkAbort(signal)
    const natural = page.getViewport({ scale: 1 })
    const size = studyRenderSize(natural.width, natural.height)
    const viewport = page.getViewport({ scale: size.scale })
    const { canvas } = makeCanvas(size.width, size.height, lang)
    const task = page.render({ canvas, viewport, background: '#fff' })
    const cancel = () => task.cancel()
    signal.addEventListener('abort', cancel, { once: true })
    try { await task.promise } finally { signal.removeEventListener('abort', cancel) }
    checkAbort(signal)
    const spans: TextSpan[] = []
    let warning = message('extraction', lang)
    try {
      const reader = page.streamTextContent().getReader()
      const abortText = () => { void reader.cancel().catch(() => undefined) }
      signal.addEventListener('abort', abortText, { once: true })
      let count = 0
      try {
        while (count <= STUDY_MATERIAL_LIMITS.textCharacters) {
          const chunk = await reader.read()
          if (chunk.done) break
          checkAbort(signal)
          for (const item of chunk.value.items) {
            if (!('str' in item)) continue
            const [x1, y1, x2, y2] = viewport.convertToViewportRectangle([item.transform[4], item.transform[5], item.transform[4] + item.width, item.transform[5] + item.height])
            spans.push({ text: item.str.slice(0, STUDY_MATERIAL_LIMITS.textCharacters - count), lineBreak: item.hasEOL, region: { x: Math.min(x1, x2) / viewport.width, y: Math.min(y1, y2) / viewport.height, width: Math.abs(x2 - x1) / viewport.width, height: Math.abs(y2 - y1) / viewport.height } })
            count += item.str.length
            if (count > STUDY_MATERIAL_LIMITS.textCharacters) break
          }
        }
        if (count > STUDY_MATERIAL_LIMITS.textCharacters) warning += message('truncated', lang)
      } finally {
        signal.removeEventListener('abort', abortText)
        await reader.cancel().catch(() => undefined)
        reader.releaseLock()
      }
      if (!spans.some(span => span.text.trim())) warning = message('noPdfText', lang)
    } catch (error) {
      checkAbort(signal)
      warning = message('extractionFailed', lang)
    }
    checkAbort(signal)
    return { canvas, spans, warning: warning + message('renderLimit', lang) }
  } finally { page?.cleanup() }
}

export async function openStudyMaterial(blob: Blob, signal?: AbortSignal, lang: MaterialLanguage = 'zh'): Promise<StudyMaterial> {
  const validated = await validateStudyMaterial(blob, signal, lang)
  checkAbort(signal)
  if (validated.kind === 'image') return { kind: 'image', pageCount: 1, mimeType: validated.mimeType, renderPage: (page, pageSignal) => {
    if (page !== 1) return Promise.reject(new Error(message('imagePage', lang)))
    return renderImage(new Blob([validated.bytes], { type: validated.mimeType }), pageSignal, lang)
  }, dispose: async () => undefined }
  const pdfjs = await import('pdfjs-dist/legacy/build/pdf.mjs')
  checkAbort(signal)
  pdfjs.GlobalWorkerOptions.workerSrc = pdfWorkerUrl
  const task = pdfjs.getDocument({ data: validated.bytes, isEvalSupported: false, enableXfa: false, useWasm: false, useWorkerFetch: false, CMapReaderFactory: LocalCMapReader, StandardFontDataFactory: LocalFontReader, maxImageSize: STUDY_MATERIAL_LIMITS.imagePixels, canvasMaxAreaInBytes: STUDY_MATERIAL_LIMITS.renderPixels * 4 })
  const cancel = () => { void task.destroy().catch(() => undefined) }
  signal?.addEventListener('abort', cancel, { once: true })
  try {
    const pdf = await task.promise
    checkAbort(signal)
    if (pdf.numPages < 1 || pdf.numPages > STUDY_MATERIAL_LIMITS.pages) throw new Error(message('pageLimit', lang))
    return { kind: 'pdf', pageCount: pdf.numPages, mimeType: validated.mimeType, renderPage: (page, pageSignal) => renderPdfPage(pdf, page, pageSignal, lang), dispose: () => task.destroy() }
  } catch (error) {
    await task.destroy().catch(() => undefined)
    checkAbort(signal)
    if (error instanceof Error && error.name === 'PasswordException') throw new Error(message('encrypted', lang))
    if (error instanceof Error && error.message.includes('1000')) throw error
    throw new Error(message('pdfFailed', lang))
  } finally { signal?.removeEventListener('abort', cancel) }
}

export async function loadStudyMaterial(blob: Blob, signal?: AbortSignal, lang: MaterialLanguage = 'zh'): Promise<StudyMaterialInfo> {
  const material = await openStudyMaterial(blob, signal, lang)
  try {
    // A valid header is not enough: reject corrupt image bytes before persistence.
    if (material.kind === 'image') {
      const preview = await material.renderPage(1, signal ?? new AbortController().signal)
      preview.canvas.width = 0
      preview.canvas.height = 0
    }
    return { kind: material.kind, pageCount: material.pageCount, mimeType: material.mimeType }
  }
  finally { await material.dispose() }
}

/** Detect observable extraction failures, not mathematical correctness or OCR quality. */
export function studyTextExtractionRisk(text: string): StudyTextExtractionRisk | undefined {
  if (!text.trim()) return 'empty'
  for (const character of text) {
    const code = character.codePointAt(0)!
    const control = (code < 0x20 && ![0x09, 0x0a, 0x0d].includes(code)) || (code >= 0x7f && code <= 0x9f)
    const privateUse = (code >= 0xe000 && code <= 0xf8ff) || (code >= 0xf0000 && code <= 0xffffd) || (code >= 0x100000 && code <= 0x10fffd)
    const missingGlyph = code === 0xfffd || code === 0xfffc || (code >= 0xd800 && code <= 0xdfff) || (code >= 0xfdd0 && code <= 0xfdef) || (code & 0xffff) >= 0xfffe
    if (control || privateUse || missingGlyph) return 'unmapped-glyphs'
  }
  // A normal-looking string can still lose fractions, exponents or reading order.
  return undefined
}

export function studyTextExtractionNotice(risk: StudyTextExtractionRisk | undefined, lang: MaterialLanguage = 'zh'): string | undefined {
  return risk ? message(risk === 'empty' ? 'emptySelection' : 'missingGlyphs', lang) : undefined
}

export function studyPageText(page: StudyPage, region: StudyRegion | null): string {
  const selection = normalizeStudyRegion(region)
  return page.spans.filter(span => !selection || (span.region.x + span.region.width >= selection.x && span.region.x <= selection.x + selection.width && span.region.y + span.region.height >= selection.y && span.region.y <= selection.y + selection.height)).map(span => span.text + (span.lineBreak ? '\n' : ' ')).join('').trim().slice(0, STUDY_MATERIAL_LIMITS.textCharacters)
}

/** Context PNGs are separate from the full reading canvas and always bounded. */
export function studyPageImage(page: StudyPage, region: StudyRegion | null): string | undefined {
  const source = page.canvas
  const crop = normalizeStudyRegion(region) ?? { x: 0, y: 0, width: 1, height: 1 }
  const sw = source.width * crop.width
  const sh = source.height * crop.height
  let scale = Math.min(1, 1600 / Math.max(sw, sh))
  const { canvas, context } = makeCanvas(1, 1)
  try {
    for (let attempt = 0; attempt < 5; attempt++) {
      canvas.width = Math.max(1, Math.floor(sw * scale))
      canvas.height = Math.max(1, Math.floor(sh * scale))
      context.drawImage(source, source.width * crop.x, source.height * crop.y, sw, sh, 0, 0, canvas.width, canvas.height)
      const dataUrl = canvas.toDataURL('image/png')
      if (dataUrl.length * 0.75 <= STUDY_MATERIAL_LIMITS.contextImageBytes && /^data:image\/png;base64,[A-Za-z0-9+/]+={0,2}$/.test(dataUrl)) return dataUrl
      scale *= 0.7
    }
    return undefined
  } finally { canvas.width = 0; canvas.height = 0 }
}
