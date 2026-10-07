import { useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent } from 'react'
import { ChevronLeft, ChevronRight, Crop, Maximize2, Minus, Plus } from 'lucide-react'
import { Button, IconButton } from '../../components/Button'
import { useLang } from '../../components/i18n'
import { Input } from '../../settings/public/controls'
import { normalizeStudyRegion, openStudyMaterial, studyPageImage, studyPageText, studyRegionFromPoints, type StudyMaterial, type StudyPage, type StudyReaderContext, type StudyRegion } from './studyMaterial'
import './StudyReader.css'

type StudyReaderProps = {
  blob: Blob
  page: number
  onPageChange: (page: number) => void
  region: StudyRegion | null
  onRegionChange: (region: StudyRegion | null) => void
  onContextChange: (context: StudyReaderContext) => void
}
type MaterialState = { blob: Blob; material?: StudyMaterial; error?: string }
type PageState = { material: StudyMaterial; page: number; rendered?: StudyPage; error?: string }

export function StudyReader({ blob, page, onPageChange, region, onRegionChange, onContextChange }: StudyReaderProps) {
  const lang = useLang()
  const zh = lang === 'zh'
  const [loaded, setLoaded] = useState<MaterialState | null>(null)
  const [rendered, setRendered] = useState<PageState | null>(null)
  const [retry, setRetry] = useState(0)
  const [pageInput, setPageInput] = useState(String(page))
  const [zoom, setZoom] = useState(1)
  const [selecting, setSelecting] = useState(false)
  const [draftRegion, setDraftRegion] = useState<StudyRegion | null>(null)
  const dragRef = useRef<{ x: number; y: number; pointerId: number } | null>(null)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const contextCallback = useRef(onContextChange)
  const material = loaded?.blob === blob ? loaded.material : undefined
  const pageCount = material?.pageCount ?? 0
  const current = rendered?.material === material && rendered?.page === page ? rendered : null
  const frame = current?.rendered
  const error = (loaded?.blob === blob && loaded.error) || current?.error || undefined
  const selection = useMemo(() => normalizeStudyRegion(region), [region])
  const text = useMemo(() => frame ? studyPageText(frame, selection) : '', [frame, selection])
  const imageDataUrl = useMemo(() => frame ? studyPageImage(frame, selection) : undefined, [frame, selection])
  const displayedRegion = draftRegion ?? selection
  const warning = frame?.warning && `${frame.warning}${!imageDataUrl ? (zh ? ' 图片过大，未附加到问题上下文。' : ' The image is too large to attach as question context.') : ''}`

  useLayoutEffect(() => { contextCallback.current = onContextChange }, [onContextChange])
  // Publish empty context at the same commit as a page/file change, before a stale
  // page can be sent. A late asynchronous render never owns the current context.
  useLayoutEffect(() => {
    contextCallback.current({ page, pageCount, text, imageDataUrl, region: selection, status: error ? 'error' : frame ? 'ready' : 'loading', warning, error })
  }, [blob, page, pageCount, text, imageDataUrl, selection, frame, warning, error])

  useEffect(() => {
    const controller = new AbortController()
    let owned: StudyMaterial | undefined
    void openStudyMaterial(blob, controller.signal, lang).then(material => {
      owned = material
      if (controller.signal.aborted) { void material.dispose(); return }
      setLoaded({ blob, material })
    }).catch(error => {
      if (!controller.signal.aborted) setLoaded({ blob, error: error instanceof Error ? error.message : (lang === 'zh' ? '材料无法打开' : 'Cannot open this material') })
    })
    return () => { controller.abort(); void owned?.dispose().catch(() => undefined) }
  }, [blob, retry, lang])

  useEffect(() => {
    if (!material) return
    const controller = new AbortController()
    let owned: StudyPage | undefined
    void material.renderPage(page, controller.signal).then(result => {
      owned = result
      if (controller.signal.aborted) { result.canvas.width = 0; result.canvas.height = 0; return }
      setRendered({ material, page, rendered: result })
    }).catch(error => {
      if (!controller.signal.aborted) setRendered({ material, page, error: error instanceof Error ? error.message : (lang === 'zh' ? '本页无法显示' : 'Cannot display this page') })
    })
    return () => { controller.abort(); if (owned) { owned.canvas.width = 0; owned.canvas.height = 0 } }
  }, [material, page, lang])

  useLayoutEffect(() => {
    const canvas = canvasRef.current
    if (!canvas || !frame) return
    canvas.width = frame.canvas.width
    canvas.height = frame.canvas.height
    canvas.getContext('2d')?.drawImage(frame.canvas, 0, 0)
  }, [frame])

  useEffect(() => {
    setPageInput(String(page))
    setDraftRegion(null)
    setSelecting(false)
    dragRef.current = null
  }, [blob, page])

  const goToPage = (next: number) => {
    if (!Number.isInteger(next) || next < 1 || next > pageCount || next === page) { setPageInput(String(page)); return }
    onPageChange(next)
  }
  const commitPageInput = () => goToPage(Number(pageInput))
  const keyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement || event.target instanceof HTMLButtonElement) return
    if (event.key === 'Escape') { setSelecting(false); setDraftRegion(null); dragRef.current = null; onRegionChange(null); return }
    if (selection && event.key.startsWith('Arrow')) {
      event.preventDefault()
      const delta = event.altKey ? 0.005 : 0.02
      const dx = event.key === 'ArrowLeft' ? -delta : event.key === 'ArrowRight' ? delta : 0
      const dy = event.key === 'ArrowUp' ? -delta : event.key === 'ArrowDown' ? delta : 0
      onRegionChange(normalizeStudyRegion(event.shiftKey ? { ...selection, width: Math.max(0.01, selection.width + dx), height: Math.max(0.01, selection.height + dy) } : { ...selection, x: Math.max(0, Math.min(1 - selection.width, selection.x + dx)), y: Math.max(0, Math.min(1 - selection.height, selection.y + dy)) }))
    } else if (['PageUp', 'ArrowLeft', 'PageDown', 'ArrowRight'].includes(event.key)) {
      event.preventDefault()
      goToPage(page + (event.key === 'PageUp' || event.key === 'ArrowLeft' ? -1 : 1))
    }
  }
  const point = (event: PointerEvent<HTMLDivElement>) => {
    const rect = event.currentTarget.getBoundingClientRect()
    return { x: Math.max(0, Math.min(1, (event.clientX - rect.left) / rect.width)), y: Math.max(0, Math.min(1, (event.clientY - rect.top) / rect.height)) }
  }
  const pointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (!selecting || event.button !== 0 || !frame) return
    event.preventDefault()
    // Keep the pointer on the same PDF coordinates when the page is scrolled.
    event.currentTarget.focus({ preventScroll: true })
    event.currentTarget.setPointerCapture(event.pointerId)
    dragRef.current = { ...point(event), pointerId: event.pointerId }
    setDraftRegion(null)
  }
  const pointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const start = dragRef.current
    if (start && start.pointerId === event.pointerId) setDraftRegion(studyRegionFromPoints(start, point(event)))
  }
  const pointerUp = (event: PointerEvent<HTMLDivElement>) => {
    const start = dragRef.current
    if (!start || start.pointerId !== event.pointerId) return
    const next = studyRegionFromPoints(start, point(event))
    dragRef.current = null
    setDraftRegion(null)
    setSelecting(false)
    if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId)
    if (next) onRegionChange(next)
  }

  return <section className="kv-study-reader" aria-label={zh ? '学习材料阅读器' : 'Study material reader'} onKeyDown={keyDown}>
    <div className="kv-study-reader-toolbar">
      <div className="kv-study-reader-pages">
        <IconButton label={zh ? '上一页' : 'Previous page'} size="sm" disabled={!material || page <= 1} onClick={() => goToPage(page - 1)}><ChevronLeft size={16} /></IconButton>
        <div className="kv-study-reader-page-input"><Input type="number" aria-label={zh ? '页码' : 'Page number'} min={1} max={pageCount || 1} disabled={!material} value={pageInput} onChange={setPageInput} onBlur={commitPageInput} onKeyDown={event => { if (event.key === 'Enter') { event.preventDefault(); commitPageInput() } }} /></div>
        <span aria-live="polite">/ {pageCount || '—'}</span>
        <IconButton label={zh ? '下一页' : 'Next page'} size="sm" disabled={!material || page >= pageCount} onClick={() => goToPage(page + 1)}><ChevronRight size={16} /></IconButton>
      </div>
      <div className="kv-study-reader-actions">
        <IconButton label={zh ? '缩小页面' : 'Zoom out'} size="sm" disabled={zoom <= 1} onClick={() => setZoom(value => Math.max(1, value - 0.25))}><Minus size={15} /></IconButton>
        <IconButton label={zh ? `放大页面，当前 ${Math.round(zoom * 100)}%` : `Zoom in, currently ${Math.round(zoom * 100)}%`} size="sm" disabled={zoom >= 2} onClick={() => setZoom(value => Math.min(2, value + 0.25))}><Plus size={15} /></IconButton>
        <Button size="sm" variant={selecting ? 'primary' : 'default'} disabled={!frame} aria-pressed={selecting} onClick={() => setSelecting(value => !value)}><Crop size={14} />{zh ? '框选' : 'Select area'}</Button>
        <IconButton label={zh ? '使用整页' : 'Use full page'} size="sm" disabled={!selection && !selecting} onClick={() => { onRegionChange(null); setSelecting(false) }}><Maximize2 size={15} /></IconButton>
      </div>
    </div>
    <div className="kv-study-reader-scroll custom-scrollbar">
      {error ? <div className="kv-study-reader-status" role="alert"><p>{error}</p><Button size="sm" onClick={() => { setLoaded(null); setRendered(null); setRetry(value => value + 1) }}>{zh ? '重新读取' : 'Retry'}</Button></div> : !frame ? <p className="kv-study-reader-status" role="status">{zh ? `正在读取第 ${page} 页…` : `Reading page ${page}…`}</p> : <>
        <div className="kv-study-reader-paper-wrap" style={{ width: `${zoom * 100}%` }}>
          <div className={`kv-study-reader-paper${selecting ? ' is-selecting' : ''}`} role="group" aria-label={zh ? `第 ${page} 页。Page Up / Page Down 翻页；框选后方向键移动，Shift 加方向键调整大小，Escape 清除` : `Page ${page}. Page Up / Page Down to navigate. Arrow keys move a selected area; Shift and arrow keys resize; Escape clears it.`} tabIndex={0} onPointerDown={pointerDown} onPointerMove={pointerMove} onPointerUp={pointerUp} onPointerCancel={() => { dragRef.current = null; setDraftRegion(null) }}>
            <canvas ref={canvasRef} role="img" aria-label={zh ? `材料第 ${page} 页；可提取的文字见下方文字视图` : `Material page ${page}; extracted text is available below`} />
            {displayedRegion && <div className="kv-study-reader-region" aria-hidden="true" style={{ left: `${displayedRegion.x * 100}%`, top: `${displayedRegion.y * 100}%`, width: `${displayedRegion.width * 100}%`, height: `${displayedRegion.height * 100}%` }} />}
          </div>
        </div>
        <div className="kv-study-reader-text">
          {selecting && <p role="status">{zh ? '在页面上拖动框选；也可用下方按钮创建键盘可调整的选区' : 'Drag to select an area, or use the button below to create a keyboard-adjustable selection'}</p>}
          <div className="kv-study-reader-text-actions"><span>{selection ? (zh ? '当前范围：框选区域' : 'Scope: selected area') : (zh ? '当前范围：整页' : 'Scope: full page')}</span><Button size="sm" variant="ghost" onClick={() => { onRegionChange({ x: 0.2, y: 0.2, width: 0.6, height: 0.3 }); setSelecting(false); canvasRef.current?.parentElement?.focus() }}>{zh ? '键盘框选' : 'Select with keyboard'}</Button></div>
          <p className="kv-study-reader-notice">{warning}</p>
          <details><summary>{selection ? (zh ? '查看框选区域文字（尽力提取）' : 'View selected text (best-effort)') : (zh ? '查看本页文字（可复制）' : 'View page text (copyable)')}</summary><p className="kv-study-reader-extracted">{text || (zh ? '没有可提取的文字。请在问题区补充题目文字。' : 'No extractable text. Add the problem text in the question area.')}</p></details>
        </div>
      </>}
    </div>
  </section>
}
