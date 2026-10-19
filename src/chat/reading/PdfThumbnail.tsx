import { useEffect, useRef, useState } from 'react'
import { FileText } from 'lucide-react'
import { loadReadingPdf } from './pdfDocument'
import { readingApi } from '../../api/reading'

// Small previews are shared across list remounts. Only one PDF is decoded at a time.
const previews = new Map<string, string>()
const pending = new Map<string, Promise<string>>()
let queue: Promise<unknown> = Promise.resolve()

function thumbnail(id: string, key: string): Promise<string> {
  const cached = previews.get(key)
  if (cached) return Promise.resolve(cached)
  const existing = pending.get(key)
  if (existing) return existing
  const work = queue
    .catch(() => undefined)
    .then(async () => {
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
  pending.set(key, work)
  void work.finally(() => pending.delete(key)).catch(() => undefined)
  return work
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
    setSource(previews.get(key))
    setFailed(false)
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (!entry.isIntersecting) return
        observer.disconnect()
        void thumbnail(id, key)
          .then((url) => {
            if (active) setSource(url)
          })
          .catch(() => {
            if (active) setFailed(true)
          })
      },
      { rootMargin: '160px' },
    )
    if (host.current) observer.observe(host.current)
    return () => {
      active = false
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
