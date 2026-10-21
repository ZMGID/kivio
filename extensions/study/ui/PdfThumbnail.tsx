import { useEffect, useRef, useState } from 'react'
import { FileText } from 'lucide-react'
import { loadReadingPdf } from './pdfDocument'
import { readingApi } from '../api/reading'

// Small previews are shared across list remounts. Only one PDF is decoded at a time.
const previews = new Map<string, string>()
type PreviewRequest = { promise: Promise<string | undefined>; users: number }
const pending = new Map<string, PreviewRequest>()
let queue: Promise<unknown> = Promise.resolve()

function thumbnail(id: string, key: string): { promise: Promise<string | undefined>; release: () => void } {
  const cached = previews.get(key)
  if (cached) return { promise: Promise.resolve(cached), release: () => {} }
  const existing = pending.get(key)
  if (existing && existing.users > 0) {
    existing.users++
    return { promise: existing.promise, release: () => { existing.users-- } }
  }
  const request: PreviewRequest = { promise: Promise.resolve(undefined), users: 1 }
  const work = queue
    .catch(() => undefined)
    .then(async () => {
      if (!request.users) return undefined
      const task = loadReadingPdf(await readingApi.pdf(id))
      try {
        const pdf = await task.promise
        const page = await pdf.getPage(1)
        const base = page.getViewport({ scale: 1 })
        const viewport = page.getViewport({
          scale: 160 / Math.max(base.width, base.height),
        })
        const canvas = document.createElement('canvas')
        canvas.width = Math.ceil(viewport.width)
        canvas.height = Math.ceil(viewport.height)
        await page.render({ canvas, viewport, background: '#ffffff' }).promise
        const result = canvas.toDataURL('image/png')
        if (previews.size >= 128) previews.delete(previews.keys().next().value!)
        previews.set(key, result)
        return result
      } finally {
        await task.destroy()
      }
    })
  queue = work.catch(() => undefined)
  request.promise = work
  pending.set(key, request)
  void work.finally(() => { if (pending.get(key) === request) pending.delete(key) }).catch(() => undefined)
  return { promise: work, release: () => { request.users-- } }
}

export function PdfThumbnail({
  id,
  fingerprint,
  name,
}: {
  id: string
  fingerprint?: string
  name: string
}) {
  const key = `${id}:${fingerprint ?? ''}`
  const host = useRef<HTMLSpanElement>(null)
  const [source, setSource] = useState(() => previews.get(key))
  const [failed, setFailed] = useState(false)
  useEffect(() => {
    let active = true
    let request: ReturnType<typeof thumbnail> | undefined
    setSource(previews.get(key))
    setFailed(false)
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (!entry.isIntersecting) { request?.release(); request = undefined; return }
        if (request) return
        const current = thumbnail(id, key)
        request = current
        void current.promise
          .then((url) => {
            if (active && request === current && url) setSource(url)
          })
          .catch(() => {
            if (active && request === current) setFailed(true)
          })
      },
      { rootMargin: '160px' },
    )
    if (host.current) observer.observe(host.current)
    return () => {
      active = false
      request?.release()
      observer.disconnect()
    }
  }, [id, key])
  return (
    <span
      ref={host}
      className="literature-thumbnail"
      title={failed ? '首页预览不可用，仍可打开 PDF' : `${name} · PDF 首页`}
    >
      {source ? (
        <img src={source} alt={`${name} 的 PDF 首页`} />
      ) : (
        <FileText
          size={20}
          aria-label={failed ? 'PDF 预览不可用' : '正在加载 PDF 首页'}
        />
      )}
    </span>
  )
}
