import { afterEach, expect, it, vi } from 'vitest'
import { mkdtempSync, mkdirSync, rmSync, symlinkSync } from 'node:fs'
import { createRequire } from 'node:module'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import * as pdfjs from 'pdfjs-dist'
import { loadReadingPdf } from './pdfDocument'

// Real PDF.js decoding; the legacy build supplies Node's missing DOM primitives.
vi.mock('pdfjs-dist', async () => import('pdfjs-dist/legacy/build/pdf.mjs'))
const require = createRequire(import.meta.url)
let root: string | undefined
afterEach(() => {
  vi.unstubAllEnvs()
  if (root) rmSync(root, { recursive: true, force: true })
})

it('extracts Chinese text using the locally packaged CMaps', async () => {
  root = mkdtempSync(join(tmpdir(), 'kivio-pdf-'))
  mkdirSync(join(root, 'pdfjs'))
  const packageRoot = dirname(require.resolve('pdfjs-dist/package.json'))
  for (const directory of ['cmaps', 'standard_fonts', 'wasm'])
    symlinkSync(join(packageRoot, directory), join(root, 'pdfjs', directory), 'dir')
  vi.stubEnv('BASE_URL', `${root}/`)
  pdfjs.GlobalWorkerOptions.workerSrc = require.resolve('pdfjs-dist/legacy/build/pdf.worker.mjs')

  // A valid non-embedded Chinese font requires the named UniGB-UCS2-H CMap.
  const stream = 'BT /F1 24 Tf 50 700 Td <4F60597D> Tj ET'
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 4 0 R >> >> /Contents 6 0 R >>',
    '<< /Type /Font /Subtype /Type0 /BaseFont /STSong-Light /Encoding /UniGB-UCS2-H /DescendantFonts [5 0 R] >>',
    '<< /Type /Font /Subtype /CIDFontType0 /BaseFont /STSong-Light /FontDescriptor 7 0 R /CIDSystemInfo << /Registry (Adobe) /Ordering (GB1) /Supplement 4 >> >>',
    `<< /Length ${stream.length} >>\nstream\n${stream}\nendstream`,
    '<< /Type /FontDescriptor /FontName /STSong-Light /Flags 6 /FontBBox [-25 -254 1000 880] /ItalicAngle 0 /Ascent 880 /Descent -120 /CapHeight 880 /StemV 80 >>',
  ]
  let source = '%PDF-1.4\n'
  const offsets: number[] = []
  objects.forEach((object, index) => {
    offsets.push(source.length)
    source += `${index + 1} 0 obj\n${object}\nendobj\n`
  })
  const xref = source.length
  source += `xref\n0 8\n0000000000 65535 f \n${offsets.map(offset => `${String(offset).padStart(10, '0')} 00000 n \n`).join('')}trailer\n<< /Size 8 /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF`
  const task = loadReadingPdf(new TextEncoder().encode(source))
  try {
    const document = await task.promise
    const text = await (await document.getPage(1)).getTextContent()
    expect(text.items.map(item => 'str' in item ? item.str : '').join('')).toBe('你好')
  } finally {
    await task.destroy()
  }
})
