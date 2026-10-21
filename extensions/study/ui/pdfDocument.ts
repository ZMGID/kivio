import * as pdfjs from 'pdfjs-dist'
import worker from 'pdfjs-dist/build/pdf.worker.min.mjs?url'

pdfjs.GlobalWorkerOptions.workerSrc = worker

/** Shared PDF loading policy for reading, metadata extraction and thumbnails. */
export function loadReadingPdf(data: Uint8Array) {
  const assets = `${import.meta.env.BASE_URL}pdfjs/`
  return pdfjs.getDocument({
    data,
    cMapUrl: `${assets}cmaps/`,
    cMapPacked: true,
    standardFontDataUrl: `${assets}standard_fonts/`,
    wasmUrl: `${assets}wasm/`,
    isEvalSupported: false,
  })
}
