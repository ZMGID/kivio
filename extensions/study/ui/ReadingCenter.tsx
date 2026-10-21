import { AttachmentPresentationContext } from '../../sdk/chat'
import { ErrorNotice, OperationNotice } from '../../sdk/ui'
import { notifyError } from '../../sdk/ui'
import React, { useState, useRef, useEffect, useMemo, useCallback } from 'react'
import {
  BookOpen,
  ChevronLeft,
  ChevronRight,
  PanelLeft,
  Search,
  Plus,
  X,
  ArrowLeft,
  ArrowUpRight,
  FileText,
  Minus,
  Highlighter,
  MousePointer2,
  Underline,
  Strikethrough,
  Scan,
  Camera,
  MessageSquare,
  StickyNote,
  Trash2,
  Circle,
  PanelRightClose,
  PanelRightOpen,
  Loader2,
} from 'lucide-react'
import { Button, IconButton } from '../../sdk/ui'
import { ComposeIcon } from '../../sdk/ui'
import { Input, Select, TextArea } from '../../sdk/ui'
import { savePastedImage } from '../../sdk/files'
import { ChatMarkdown } from '../../sdk/chat'
import { MessageList, type MessageListProps } from '../../sdk/chat'
import { InputBar, type InputBarProps } from '../../sdk/chat'
import { insertTextIntoComposer } from '../../sdk/chat'
import { beginComposerAttachmentOperation, updateComposerDraft, migrateComposerDraft } from '../../sdk/chat'
import * as pdfjs from 'pdfjs-dist'
import { loadReadingPdf } from './pdfDocument'
import './pdfTextLayer.css'
import './reading.css'

import LiteratureLibrary from './LiteratureLibrary'
import StudyEnvironment from './StudyEnvironment'
import ReadingDocumentTools from './ReadingDocumentTools'
import { useReadingSession } from './useReadingSession'
import type { ReadingPdfKind } from '../api/readingOperations'
import { readingLanguageName } from './readingLanguages'
import {
  readingApi,
  type ReadingDocument,
  type ReadingOpenTarget,
  type ReadingAnchor as Anchor,
  type ReadingRect as Rect,
  type ReadingNote as Note,
} from '../api/reading'
import {
  useReadingLibrary,
  loadReadingLibrary,
  refreshReadingLibrary,
  updateReadingDocument,
  retryReadingSaves,
  removeMissingReadingConversations,
} from './readingLibrary'
import { readingMessageAnchor, readingReference } from './readingContext'
import { chatApi } from '../../sdk/chat'
import type { Conversation, PendingAttachment } from '../../sdk/chat'
const PdfPage = React.memo(function PdfPage({
  pdf,
  number,
  width,
  zoom,
  initialRatio,
  onReady,
}: {
  pdf: pdfjs.PDFDocumentProxy
  number: number
  width: number
  zoom: number
  initialRatio: number
  onReady: () => void
}) {
  const onReadyRef = useRef(onReady)
  onReadyRef.current = onReady
  const el = useRef<HTMLDivElement>(null)
  const [near, setNear] = useState(false),
    [ratio, setRatio] = useState(initialRatio),
    [error, setError] = useState(false)
  useEffect(() => {
    const observer = new IntersectionObserver(
      ([entry]) => setNear(entry.isIntersecting),
      { rootMargin: '1000px' },
    )
    if (el.current) observer.observe(el.current)
    return () => observer.disconnect()
  }, [])
  useEffect(() => {
    if (!near || !el.current) return
    let cancelled = false
    let render: pdfjs.RenderTask | undefined
    let layer: pdfjs.TextLayer | undefined
    const target = el.current
    setError(false)
    void pdf
      .getPage(number)
      .then(async (p) => {
        if (cancelled) return
        const base = p.getViewport({ scale: 1 })
        setRatio(base.height / base.width)
        const view = p.getViewport({ scale: (width * zoom) / base.width })
        target.replaceChildren()
        target.style.setProperty('--scale-factor', String(view.scale))
        target.style.setProperty('--total-scale-factor', String(view.scale))
        const canvas = document.createElement('canvas'),
          dpr = window.devicePixelRatio || 1
        canvas.width = view.width * dpr
        canvas.height = view.height * dpr
        canvas.style.width = `${view.width}px`
        canvas.style.height = `${view.height}px`
        target.append(canvas)
        render = p.render({
          canvas,
          canvasContext: canvas.getContext('2d')!,
          viewport: view,
          transform: [dpr, 0, 0, dpr, 0, 0],
        })
        await render.promise
        if (cancelled) return
        const div = document.createElement('div')
        div.className = 'textLayer'
        target.append(div)
        layer = new pdfjs.TextLayer({
          textContentSource: await p.getTextContent(),
          container: div,
          viewport: view,
        })
        await layer.render()
        if (!cancelled) onReadyRef.current()
      })
      .catch((e) => {
        if (!cancelled && e.name !== 'RenderingCancelledException')
          setError(true)
      })
    return () => {
      cancelled = true
      render?.cancel()
      layer?.cancel()
      target.replaceChildren()
    }
  }, [pdf, number, width, zoom, near])
  return (
    <div
      className="page-frame"
      data-page={number}
      style={{ width: width * zoom, height: width * zoom * ratio }}
    >
      <div
        ref={el}
        className="pdf-sheet"
        aria-label={`PDF 第 ${number} 页`}
        style={{ height: '100%' }}
      />
      {error && (
        <div role="alert" className="page-error">
          这一页加载失败，请重新打开文件。
        </div>
      )}
      <span className="paper-page-number">{number}</span>
    </div>
  )
})

const PdfPages = React.memo(function PdfPages({ pdf, width, zoom, pageRatio, notes, viewKind, viewRunId, highlight, areaDraft, markColor, onReady }: {
  pdf: pdfjs.PDFDocumentProxy; width: number; zoom: number; pageRatio: number; notes: Note[];
  viewKind: ReadingPdfKind; viewRunId?: string; highlight: Anchor | null; areaDraft: Anchor | null;
  markColor: string; onReady: () => void;
}) {
  const marks = useMemo(() => {
    const grouped = new Map<number, Note[]>()
    for (const note of notes) {
      const anchor = note.anchor
      if (note.kind !== 'highlight' || !anchor?.rect || (anchor.view?.kind ?? 'source') !== viewKind ||
        (viewKind !== 'source' && anchor.view?.runId !== viewRunId)) continue
      const items = grouped.get(anchor.page) ?? []
      items.push(note); grouped.set(anchor.page, items)
    }
    return grouped
  }, [notes, viewKind, viewRunId])
  const matchingHighlight = highlight && (highlight.view?.kind ?? 'source') === viewKind &&
    (viewKind === 'source' || highlight.view?.runId === viewRunId) ? highlight : null
  return <>{Array.from({ length: pdf.numPages }, (_, i) => <div className="page-container" key={i}>
    <PdfPage pdf={pdf} number={i + 1} width={width} zoom={zoom} initialRatio={pageRatio} onReady={onReady} />
    {(marks.get(i + 1) ?? []).flatMap(n => (n.anchor!.rects?.length ? n.anchor!.rects : [n.anchor!.rect!]).map((rect, index) =>
      <div key={`${n.id}-${index}`} className={`source-mark saved mark-${n.anchor!.mark ?? 'highlight'}`}
        style={{ ...rectStyle(rect, width * zoom), '--mark-color': n.anchor!.color || '#d3a51d' } as React.CSSProperties} />))}
    {areaDraft?.page === i + 1 && areaDraft.rect && <div className="source-mark saved mark-area" style={{ ...rectStyle(areaDraft.rect, width * zoom), '--mark-color': markColor } as React.CSSProperties} />}
    {matchingHighlight?.page === i + 1 && matchingHighlight.rect && <div className="source-mark" style={rectStyle(matchingHighlight.rect, width * zoom)} />}
  </div>)}</>
})

export interface ReadingCenterProps {
  headerControls?: React.ReactNode
  inputBarProps: InputBarProps
  messageListProps: MessageListProps
  modelControls: React.ReactNode
  selectedModel?: { providerId: string; model: string }
  onCreateConversation: (name: string) => Promise<Conversation>
  onActivateConversation: (id: string | null) => Promise<void>
  onSend: (
    conversation: Conversation,
    content: string,
    attachments: PendingAttachment[],
    onAccepted?: () => void,
  ) => Promise<boolean>
}
export default function ReadingCenter(props: ReadingCenterProps) {
  const library = useReadingLibrary()
  const [activeId, setActiveId] = useState<string | null>(null)
  const [error, setError] = useState('')
  const [openTarget, setOpenTarget] = useState<ReadingOpenTarget | undefined>()
  useEffect(() => {
    void loadReadingLibrary().catch(() => { /* Library exposes the load error. */ })
  }, [])
  const select = (document: ReadingDocument, target?: ReadingOpenTarget) => {
    setOpenTarget(target)
    setActiveId(document.id)
    void updateReadingDocument(document.id, (d) => ({
      ...d,
      lastOpenedAt: new Date().toISOString(),
    })).catch((e) => setError(String(e)))
  }
  const doc = library.documents.find(
    (document) =>
      document.id === activeId &&
      !document.trashedAt &&
      document.hasPdf !== false,
  )
  return (
    <StudyEnvironment headerControls={props.headerControls}>{openSetup => (
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        <ErrorNotice
          message={error || library.error || library.issues?.join('\n')}
          actionLabel="重试保存 / 加载"
          onAction={() => {
            setError('')
            void retryReadingSaves().then(() => refreshReadingLibrary()).catch((error) => setError(String(error)))
          }}
        />
        <div className={doc ? 'hidden' : 'flex min-h-0 flex-1 flex-col'}>
          <LiteratureLibrary active={!doc} onRead={select} headerControls={props.headerControls} onConfigure={openSetup} />
        </div>
        {doc ? (
          <ReadingWorkspace
            key={doc.id}
            {...props}
            doc={doc}
            initialTarget={openTarget}
            openPdf={async () => setActiveId(null)}
            saveStatus={library.saveStatus}
            onConfigure={openSetup}
          />
        ) : null}
      </div>
    )}</StudyEnvironment>
  )
}
function ReadingWorkspace({
  doc,
  initialTarget,
  openPdf,
  saveStatus,
  inputBarProps,
  messageListProps,
  modelControls,
  selectedModel,
  onConfigure,
  headerControls,
  onCreateConversation,
  onActivateConversation,
  onSend,
}: ReadingCenterProps & {
  doc: ReadingDocument
  initialTarget?: ReadingOpenTarget
  openPdf: () => Promise<void>
  saveStatus: string
  onConfigure: () => void
}) {
  const reading = useReadingSession(doc.id, initialTarget?.view)
  const viewKind = reading.session?.view.kind ?? 'source'
  const viewRunId = reading.session?.view.output?.runId
  const viewKey = `${viewKind}:${viewRunId || ''}`
  const edition = viewKind === 'source' ? '原文' : viewKind === 'mono' ? '译文' : '双语对照'
  const shownOutput = reading.session?.view.output ?? reading.session?.view.latest
  const [pageRatio, setPageRatio] = useState(792 / 612)
  const locationView: Anchor['view'] = viewKind === 'source' || !viewRunId ? undefined : { kind: viewKind, runId: viewRunId }
  const anchorMatches = (anchor?: Anchor | null) => (anchor?.view?.kind ?? 'source') === viewKind && (viewKind === 'source' || anchor?.view?.runId === viewRunId)
  const pendingRestore = useRef<{ page: number; offset: number } | null>(null)
  const loadedView = useRef<string | null>(null)
  const sessionLoaded = !!reading.session, sessionFailed = !!reading.error
  const [pdf, setPdf] = useState<pdfjs.PDFDocumentProxy | null>(null),
    [error, setError] = useState('')
  const extraction = useRef<Promise<string[] | null>>(Promise.resolve(null))
  const [pages, setPages] = useState<string[]>([]),
    [indexing, setIndexing] = useState(true)
  const [page, setPage] = useState(initialTarget?.page ?? doc.page),
    [width, setWidth] = useState(720),
    [zoom, setZoom] = useState(1)
  const [fit, setFit] = useState('width'), [pageInput, setPageInput] = useState(String(initialTarget?.page ?? doc.page))
  const pageEditing = useRef(false)
  const [capturing, setCapturing] = useState(false)
  const [readerHeight, setReaderHeight] = useState(0)
  const captureLock = useRef(false)
  const [side, setSide] = useState<'chat' | 'notes' | 'history'>(initialTarget?.noteId ? 'notes' : 'chat'),
    [sideOpen, setSideOpen] = useState(true),
    [mobile, setMobile] = useState(initialTarget?.noteId && !initialTarget.page ? 'chat' : 'reader')
  const [nav, setNav] = useState<'outline' | 'search' | null>(null),
    [toc, setToc] = useState<{ title: string; page: number; depth: number }[]>(
      [],
    )
  const [query, setQuery] = useState(''),
    [notice, setNotice] = useState('')
  const [selection, setSelection] = useState<
      (Anchor & { left: number; top: number }) | null
    >(null),
    [highlight, setHighlight] = useState<Anchor | null>(doc.notes.find(note => note.id === initialTarget?.noteId)?.anchor ?? null)
  const [markTool, setMarkTool] = useState<NonNullable<Anchor['mark']> | 'select' | 'capture'>('select')
  const [markColor, setMarkColor] = useState('#d3a51d')
  const [areaDraft, setAreaDraft] = useState<Anchor | null>(null)
  const areaDrag = useRef<{ element: HTMLElement; x: number; y: number; page: number; pointer: number } | null>(null)
  useEffect(() => { setSelection(null); setAreaDraft(null); areaDrag.current = null }, [viewKey])
  const initialNoteRef = useCallback((element: HTMLElement | null) => element?.scrollIntoView?.({ block: 'nearest' }), [])
  const [noteDraft, setNoteDraft] = useState(doc.noteDraft),
    [returnTo, setReturnTo] = useState<{ page: number; offset: number; view?: Anchor['view'] } | null>(
      null,
    ),
    [split, setSplit] = useState(60)
  const [preparing, setPreparing] = useState(false),
    [chatLoading, setChatLoading] = useState(true),
    [chatError, setChatError] = useState('')
  const [history, setHistory] = useState<{ id: string; conversation: Conversation | null }[]>([])
  const [historyLoading, setHistoryLoading] = useState(false)
  const [historyRevision, setHistoryRevision] = useState(0)
  useEffect(() => {
    if (side !== 'history') return
    let live = true
    setHistoryLoading(true)
    void Promise.all([...new Set(doc.conversationIds)].map(async (id) => {
      try { return { id, conversation: await chatApi.getConversation(id) } }
      catch { return { id, conversation: null } }
    })).then(async (items) => {
      if (!live) return
      const failed = items.filter(item => !item.conversation).map(item => item.id)
      const missing = failed.length ? await readingApi.missingConversations(failed).catch(() => []) : []
      if (!live) return
      setHistory(items.filter(item => !missing.includes(item.id)).reverse())
      setHistoryLoading(false)
      if (missing.length) {
        await removeMissingReadingConversations(doc.id, missing).catch(() => { /* Library retains dirty links for retry. */ })
      }
    })
    return () => { live = false }
  }, [side, doc.id, doc.conversationIds, historyRevision])
  const host = useRef<HTMLDivElement>(null),
    reader = useRef<HTMLElement>(null),
    alive = useRef(true)
  const position = useRef({ page: initialTarget?.page ?? doc.page, offset: initialTarget?.page ? 0 : doc.offset }),
    draftRef = useRef(noteDraft)
  const pendingPosition = useRef<ReturnType<typeof setTimeout>>()
  const sendLock = useRef(false)
  const sendSequence = useRef(0)
  const noteSaveLock = useRef(false)
  const [noteSaving, setNoteSaving] = useState(false)
  draftRef.current = noteDraft
  const update = useCallback(
    (change: Partial<ReadingDocument>) =>
      updateReadingDocument(doc.id, (value) => ({ ...value, ...change })),
    [doc.id],
  )
  const persist = useCallback(
    (change: Partial<ReadingDocument>) => {
      void update(change).catch(() => {
        /* Library keeps unsaved changes and shows retry. */
      })
    },
    [update],
  )
  useEffect(() => {
    alive.current = true
    return () => {
      alive.current = false
      clearTimeout(pendingPosition.current)
      persist({ ...position.current, noteDraft: draftRef.current })
    }
  }, [persist])
  useEffect(() => {
    const timer = setTimeout(() => persist({ noteDraft }), 400)
    return () => clearTimeout(timer)
  }, [noteDraft, persist])
  useEffect(() => {
    if (!notice) return
    const timer = setTimeout(() => setNotice(''), 3500)
    return () => clearTimeout(timer)
  }, [notice])
  useEffect(() => {
    let live = true
    setChatLoading(true)
    setChatError('')
    void onActivateConversation(doc.activeConversationId).then(
      () => {
        if (live) setChatLoading(false)
      },
      async (error) => {
        if (!live) return
        const id = doc.activeConversationId
        if (id) {
          try {
            const missing = await readingApi.missingConversations([id])
            if (!live) return
            if (missing.includes(id)) {
              await removeMissingReadingConversations(doc.id, [id]).catch(() => { /* Library exposes save failures and retains retry state. */ })
              return
            }
          } catch { /* An unavailable probe is not evidence that a chat was deleted. */ }
        }
        if (live) {
          setChatLoading(false)
          setChatError(String(error))
        }
      },
    )
    return () => {
      live = false
    }
  }, [doc.id, doc.activeConversationId, onActivateConversation])
  useEffect(() => {
    if (!sessionLoaded && !sessionFailed) return
    let live = true
    let task: pdfjs.PDFDocumentLoadingTask | undefined
    let completeText!: (pages: string[] | null) => void
    extraction.current = new Promise(resolve => { completeText = resolve })
    const cachedText = readingApi.text(doc.id, { kind: viewKind, runId: viewRunId }).catch(() => null)
    pendingRestore.current = { ...position.current, offset: loadedView.current && loadedView.current !== viewKey ? 0 : position.current.offset }
    loadedView.current = viewKey
    setPdf(null); setPages([]); setToc([]); setIndexing(true); setError(''); setSelection(null)
    void (async () => {
      try {
        const data = await readingApi.pdf(doc.id, { kind: viewKind, runId: viewRunId })
        if (!live) return
        task = loadReadingPdf(data)
        const pdf = await task.promise
        const first = await pdf.getPage(1)
        const viewport = first.getViewport({ scale: 1 })
        if (!live) return
        setPageRatio(viewport.height / viewport.width)
        setPdf(pdf)
        void (async () => {
        const outline = await pdf.getOutline()
        const items: { title: string; page: number; depth: number }[] = []
        const visit = async (list: typeof outline, depth = 0) => {
          for (const item of list || []) {
            try {
              const dest =
                typeof item.dest === 'string'
                  ? await pdf.getDestination(item.dest)
                  : item.dest
              if (dest) {
                const n =
                  typeof dest[0] === 'number'
                    ? dest[0]
                    : await pdf.getPageIndex(dest[0])
                items.push({ title: item.title, page: n + 1, depth })
              }
            } catch {
              /* Invalid outline entries do not block reading. */
            }
            await visit(item.items, depth + 1)
          }
        }
        await visit(outline)
        if (!live) return
        setToc(items)
        })().catch(() => { /* A broken outline must not block extraction or image questions. */ })
        const cached = await cachedText
        if (!live) return
        if (cached && cached.length === pdf.numPages) {
          setPages(cached); setIndexing(false); completeText(cached); return
        }
        const extracted: string[] = []
        for (let number = 1; number <= pdf.numPages; number++) {
          if (!live) return
          const page = await pdf.getPage(number)
          const content = await page.getTextContent()
          extracted.push(
            content.items
              .map((item) =>
                'str' in item
                  ? item.str + ('hasEOL' in item && item.hasEOL ? '\n' : ' ')
                  : '',
              )
              .join(''),
          )
        }
        if (live) {
          setPages(extracted)
          setIndexing(false)
          completeText(extracted)
          void readingApi.text(doc.id, { kind: viewKind, runId: viewRunId }, extracted).catch(() => { /* Disposable cache; reading remains available. */ })
        }
      } catch (error) {
        if (live) {
          completeText(null)
          setError(`无法读取 PDF：${String(error)}`)
          setIndexing(false)
        }
      }
    })()
    return () => {
      live = false
      completeText(null)
      void task?.destroy()
    }
  }, [doc.id, viewKind, viewRunId, viewKey, sessionLoaded, sessionFailed])
  useEffect(() => {
    const el = host.current
    if (!el) return
    const observer = new ResizeObserver(() => {
      setReaderHeight(el.clientHeight)
      if (el.clientWidth > 0)
        setWidth(Math.max(280, Math.min(1080, el.clientWidth - 48)))
    })
    observer.observe(el)
    return () => observer.disconnect()
  }, [])
  const frame = useCallback(
    (number: number) =>
      host.current?.querySelector<HTMLElement>(`[data-page="${number}"]`),
    [],
  )
  const jump = useCallback(
    (number: number, offset = 0) => {
      const target = frame(number)
      if (target && host.current) {
        // Navigation commits before the browser's deferred scroll event, so a
        // version switch in the same turn restores the requested page.
        position.current = { page: number, offset }
        host.current.scrollTo({
          top: target.parentElement!.offsetTop + offset,
          behavior: 'instant',
        })
        setPage(number)
      }
    },
    [frame],
  )
  useEffect(() => { if (!pageEditing.current) setPageInput(String(page)) }, [page])
  useEffect(() => {
    if (fit === 'width') { setZoom(1); return }
    if (fit !== 'page' || !pdf) return
    let live = true
    void pdf.getPage(page).then(p => {
      const view = p.getViewport({ scale: 1 })
      if (live && host.current) setZoom(Math.max(0.2, Math.min(1, (host.current.clientHeight - 48) / (width * view.height / view.width))))
    }).catch(error => { if (live) notifyError(String(error)) })
    return () => { live = false }
  }, [fit, pdf, page, width, readerHeight])
  useEffect(() => {
    if (!pdf) return
    const target = pendingRestore.current ?? position.current
    const frame = requestAnimationFrame(() => {
      jump(Math.min(target.page, pdf.numPages), target.offset)
      pendingRestore.current = null
    })
    return () => cancelAnimationFrame(frame)
  }, [pdf, jump])
  const locate = useCallback(() => {
    if (!highlight || !host.current) return
    if ((highlight.view?.kind ?? 'source') !== viewKind || (viewKind !== 'source' && highlight.view?.runId !== viewRunId)) return
    const target = frame(highlight.page)
    if (!target) return
    let rect = highlight.rect
    if (!rect && highlight.text) {
      const span = Array.from(target.querySelectorAll('.textLayer span')).find(
        (span) =>
          span.textContent?.trim() === highlight.text ||
          span.textContent
            ?.toLowerCase()
            .includes(highlight.text.toLowerCase()),
      )
      if (!span) return
      const r = span.getBoundingClientRect(),
        s = target.getBoundingClientRect()
      rect = {
        x: (r.x - s.x) / s.width,
        y: (r.y - s.y) / s.height,
        w: r.width / s.width,
        h: r.height / s.height,
      }
      setHighlight({ ...highlight, rect })
      return
    }
    if (rect)
      host.current.scrollTo({
        top:
          target.parentElement!.offsetTop + rect.y * target.clientHeight - 100,
        behavior: 'smooth',
      })
  }, [highlight, frame, viewKind, viewRunId])
  useEffect(locate, [locate])
  useEffect(() => {
    if (!highlight) return
    const timer = setTimeout(() => setHighlight(null), 8000)
    return () => clearTimeout(timer)
  }, [highlight])
  const cite = (anchor: Anchor) => {
    setReturnTo({ ...position.current, view: locationView })
    setMobile('reader')
    setSelection(null)
    if (anchorMatches(anchor)) { setHighlight(anchor); jump(anchor.page); return }
    position.current = { page: anchor.page, offset: 0 }
    void reading.selectView(anchor.view?.kind ?? 'source', anchor.view?.runId).then(value => {
      if (!value || !alive.current) return
      position.current = { page: anchor.page, offset: 0 }
      setHighlight(anchor)
    }).catch(error => notifyError(String(error)))
  }
  const pageFrames = useRef<HTMLElement[]>([])
  const scrollFrame = useRef<number | null>(null)
  useEffect(() => {
    pageFrames.current = Array.from(host.current?.querySelectorAll<HTMLElement>('[data-page]') ?? [])
    return () => { if (scrollFrame.current !== null) cancelAnimationFrame(scrollFrame.current); scrollFrame.current = null }
  }, [pdf])
  const onScroll = () => {
    if (scrollFrame.current !== null) return
    scrollFrame.current = requestAnimationFrame(() => {
      scrollFrame.current = null
      if (!host.current || pendingRestore.current) return
      setSelection(null)
      const top = host.current.scrollTop + 100
      const frames = pageFrames.current
      let low = 0, high = frames.length - 1, found = -1
      while (low <= high) {
        const middle = (low + high) >>> 1
        if (frames[middle].parentElement!.offsetTop <= top) { found = middle; low = middle + 1 }
        else high = middle - 1
      }
      const visible = frames[found]
      if (!visible) return
      const page = Number(visible.dataset.page)
      setPage(page)
      position.current = { page, offset: host.current.scrollTop - visible.parentElement!.offsetTop }
      clearTimeout(pendingPosition.current)
      pendingPosition.current = setTimeout(() => persist(position.current), 400)
    })
  }
  const selectText = () => {
    setSelection(null)
    const selected = window.getSelection()
    if (!selected || selected.isCollapsed || !host.current || !reader.current) return
    const frame =
      selected.anchorNode?.parentElement?.closest<HTMLElement>('[data-page]')
    if (
      !frame || !host.current.contains(frame) ||
      frame !== selected.focusNode?.parentElement?.closest('[data-page]')
    )
      return
    const text = selected.toString().trim()
    if (!text) return
    const r = selected.getRangeAt(0).getBoundingClientRect(),
      s = frame.getBoundingClientRect(),
      bounds = reader.current.getBoundingClientRect()
    const anchor: Anchor & { left: number; top: number } = {
      page: Number(frame.dataset.page),
      text: text.slice(0, 6000),
      view: locationView,
      // A Range may report the same rectangle for a span and its text node.
      // Persist each painted fragment once so highlights do not darken twice.
      rects: Array.from(selected.getRangeAt(0).getClientRects())
        .filter((box, index, boxes) => box.width > 0 && box.height > 0 && !boxes.slice(0, index).some(previous =>
          Math.abs(previous.x - box.x) < 0.5 && Math.abs(previous.y - box.y) < 0.5 &&
          Math.abs(previous.width - box.width) < 0.5 && Math.abs(previous.height - box.height) < 0.5))
        .map(box => ({ x: (box.x - s.x) / s.width, y: (box.y - s.y) / s.height, w: box.width / s.width, h: box.height / s.height })),
      rect: {
        x: (r.x - s.x) / s.width,
        y: (r.y - s.y) / s.height,
        w: r.width / s.width,
        h: r.height / s.height,
      },
      left: Math.max(12, Math.min(r.x - bounds.x, bounds.width - 300)),
      top: Math.max(48, Math.min(r.bottom - bounds.y + 8, bounds.height - 56)),
    }
    if (markTool !== 'select' && markTool !== 'area' && markTool !== 'capture') saveMark(anchor, markTool)
    else setSelection(anchor)
  }
  const composerId = doc.activeConversationId ?? `reading-${doc.id}`
  const composerRef = useRef(composerId)
  composerRef.current = composerId
  const openSide = (value: typeof side) => {
    setSide(value); setSideOpen(true); setMobile('chat')
  }
  const quote = (anchor: Anchor) => {
    const label = anchor.view?.kind === 'dual' ? '双语对照' : anchor.view ? '译文' : '原文'
    return `[${label} · 第 ${anchor.page} 页](${readingReference(anchor)})\n${anchor.text}`
  }
  const capturePage = async (anchor: Anchor = { page, text: '', view: locationView }) => {
    if (!pdf || captureLock.current) return
    captureLock.current = true; setCapturing(true)
    const scope = beginComposerAttachmentOperation(composerId)
    let loading: ReturnType<typeof loadReadingPdf> | undefined
    try {
      // A saved annotation may belong to another translation run. Never crop the visible edition instead.
      const document = anchorMatches(anchor) ? pdf : await (async () => {
        loading = loadReadingPdf(await readingApi.pdf(doc.id, { kind: anchor.view?.kind ?? 'source', runId: anchor.view?.runId }))
        return loading.promise
      })()
      const source = await document.getPage(anchor.page)
      const base = source.getViewport({ scale: 1 })
      const rect = anchor.rect ?? { x: 0, y: 0, w: 1, h: 1 }
      if (![rect.x, rect.y, rect.w, rect.h].every(Number.isFinite) || rect.w <= 0 || rect.h <= 0) throw new Error('选区位置无效，请重新框选')
      const x = Math.max(0, rect.x), y = Math.max(0, rect.y)
      const w = Math.min(1, rect.x + rect.w) - x, h = Math.min(1, rect.y + rect.h) - y
      if (w <= 0 || h <= 0) throw new Error('选区在页面之外，请重新框选')
      const view = source.getViewport({ scale: Math.min(3, 1800 / Math.max(base.width * w, base.height * h)) })
      const canvas = window.document.createElement('canvas')
      canvas.width = Math.max(1, Math.ceil(view.width * w - 1e-8)); canvas.height = Math.max(1, Math.ceil(view.height * h - 1e-8))
      await source.render({ canvas, canvasContext: canvas.getContext('2d')!, viewport: view,
        transform: [1, 0, 0, 1, -x * view.width, -y * view.height], background: 'white' }).promise
      const label = anchor.view?.kind === 'dual' ? '双语对照' : anchor.view ? '译文' : '原文'
      const name = `${doc.name} · ${label}第 ${anchor.page} 页${anchor.rect ? '选区' : ''}.png`
      const saved = await savePastedImage(name, 'image/png', canvas.toDataURL('image/png').split(',')[1])
      if (!saved.success || !saved.path) throw new Error(saved.error || '无法保存页面图片')
      updateComposerDraft(scope.key, draft => ({ ...draft, attachments: [...draft.attachments, {
        id: crypto.randomUUID(), type: 'image', name, path: saved.path!,
      }], quotes: [...draft.quotes, quote(anchor)], input: [draft.input, '请解释这张文献截图中的内容；如有图表或公式，请结合上下文说明。'].filter(Boolean).join('\n') }))
      if (alive.current && composerRef.current === scope.key) { openSide('chat'); setNotice('截图已加入右侧对话，可补充问题后发送') }
    } catch (error) { if (alive.current) notifyError(String(error)) }
    finally { void loading?.destroy(); scope.release(); captureLock.current = false; if (alive.current) setCapturing(false) }
  }
  const attach = (action: string, anchor: Anchor | null = selection) => {
    if (!anchor) return
    const prompts: Record<string, string> = {
      '解释': '请用通俗的话解释这段内容。', '翻译': '请把这段内容翻译成中文。',
      '总结': '请提炼这段内容的要点，说明它与全文的关系。',
      '公式': '请逐项解释这里的符号、公式含义和推导过程，并说明适用条件。',
    }
    openSide('chat')
    updateComposerDraft(composerId, draft => ({ ...draft,
      input: [draft.input, prompts[action]].filter(Boolean).join('\n'),
      quotes: [...draft.quotes, quote(anchor)],
    }))
    setSelection(null)
    window.getSelection()?.removeAllRanges()
  }
  const addNote = async (
    content: string,
    kind: Note['kind'],
    anchor?: Anchor,
    consumeDraft = false,
  ) => {
    if (!content.trim()) return
    try {
      await updateReadingDocument(doc.id, (document) => ({
        ...document,
        noteDraft: consumeDraft ? '' : document.noteDraft,
        notes: [
          { id: crypto.randomUUID(), content, kind, anchor },
          ...document.notes,
        ],
      }))
      setNotice(kind === 'highlight' ? '已保存标注' : '已保存到阅读笔记')
      return true
    } catch {
      setChatError('保存失败，请通过操作提示重试保存')
      return false
    }
  }
  const saveMark = (anchor: Anchor, mark: NonNullable<Anchor['mark']>) => {
    const { page, text, rect, rects, view } = anchor
    void addNote(text || `第 ${page} 页区域标注`, 'highlight', { page, text, rect, rects, view, mark, color: markColor })
    setSelection(null)
    window.getSelection()?.removeAllRanges()
  }
  const areaPoint = (event: React.PointerEvent, element: HTMLElement) => {
    const bounds = element.getBoundingClientRect()
    return { x: Math.max(0, Math.min(1, (event.clientX - bounds.x) / bounds.width)), y: Math.max(0, Math.min(1, (event.clientY - bounds.y) / bounds.height)) }
  }
  const areaAnchor = (event: React.PointerEvent): Anchor | null => {
    const drag = areaDrag.current
    if (!drag) return null
    const point = areaPoint(event, drag.element)
    return { page: drag.page, text: '', view: locationView, rect: { x: Math.min(point.x, drag.x), y: Math.min(point.y, drag.y), w: Math.abs(point.x - drag.x), h: Math.abs(point.y - drag.y) } }
  }
  const saveDraft = async () => {
    if (noteSaveLock.current || !noteDraft.trim()) return
    const submitted = noteDraft
    noteSaveLock.current = true
    setNoteSaving(true)
    try {
      // The optimistic note owns this text even if its disk write needs retry.
      // Clear the ref too, so leaving before React renders cannot restore the old draft.
      draftRef.current = ''
      setNoteDraft('')
      await addNote(submitted, 'note', undefined, true)
    } finally {
      noteSaveLock.current = false
      if (alive.current) setNoteSaving(false)
    }
  }
  const createConversation = async () => {
    const conversation = await onCreateConversation(`阅读 · ${doc.name}`)
    if (!doc.activeConversationId)
      migrateComposerDraft(`reading-${doc.id}`, conversation.id)
    await updateReadingDocument(doc.id, (document) => ({
      ...document,
      conversationIds: [...document.conversationIds, conversation.id],
      activeConversationId: conversation.id,
    }))
    return conversation
  }
  const newConversation = async () => {
    if (preparing) return
    setPreparing(true)
    try {
      await createConversation()
      if (alive.current) setSide('chat')
    } catch (error) {
      setChatError(String(error))
    } finally {
      if (alive.current) setPreparing(false)
    }
  }
  const send = async (
    content: string,
    attachments: PendingAttachment[],
    options?: { onAccepted?: () => void },
  ) => {
    if (sendLock.current || chatLoading || !pdf) return false
    sendLock.current = true
    const sequence = ++sendSequence.current
    setPreparing(true)
    setChatError('')
    try {
      const hasImage = attachments.some(item => item.type === 'image')
      const conversation = doc.activeConversationId
        ? await chatApi.getConversation(doc.activeConversationId)
        : await createConversation()
      if (!alive.current || loadedView.current !== viewKey) return false
      // Images are independently useful. Text-only first questions wait for
      // extraction; follow-ups use the backend's committed source receipt.
      let context = null
      if (!hasImage || pages.some(text => text.trim())) {
        let prepared = await readingApi.prepareChat(doc.id, conversation.id, null, { kind: viewKind, runId: viewRunId })
        if (prepared.needsText) {
          const extracted = await extraction.current
          if (!alive.current || loadedView.current !== viewKey) return false
          if (!extracted?.some(text => text.trim())) throw new Error('这份 PDF 没有可提取的文字。请用图片提问，将当前页交给模型阅读。')
          prepared = await readingApi.prepareChat(doc.id, conversation.id, extracted, { kind: viewKind, runId: viewRunId })
        }
        context = prepared.context
      }
      if (!alive.current || loadedView.current !== viewKey) return false
      await onActivateConversation(conversation.id)
      if (!alive.current || loadedView.current !== viewKey) return false
      return await onSend(
        conversation,
        content,
        [...(context ? [{ ...context, id: crypto.randomUUID(), type: 'file' as const, path: '' }] : []), ...attachments],
        () => {
          sendLock.current = false
          if (alive.current) setPreparing(false)
          options?.onAccepted?.()
        },
      )
    } catch (error) {
      if (alive.current) setChatError(String(error))
      return false
    } finally {
      if (sequence === sendSequence.current) {
        sendLock.current = false
        if (alive.current) setPreparing(false)
      }
    }
  }
  const chatReady =
    messageListProps.conversationId === doc.activeConversationId &&
    !!doc.activeConversationId
  const messages = chatReady ? messageListProps.messages : []
  const busy = chatReady && inputBarProps.disabled
  const results = useMemo(() => {
    const term = query.trim().toLowerCase()
    if (!term) return []
    return pages.flatMap((text, index) => {
      const at = text.toLowerCase().indexOf(term)
      return at < 0
        ? []
        : [
            {
              page: index + 1,
              text: text.slice(Math.max(0, at - 45), at + 120),
            },
          ]
    })
  }, [query, pages])
  const searching = indexing
  const saveAnswer = async (id: string) => {
    const message = messages.find((message) => message.id === id)
    if (!message) return false
    const citation = /\((#reading-(?:ref\?[^)]+|page-\d+))\)/.exec(message.content)
    const anchor = citation ? readingMessageAnchor(citation[1], messages, id) : null
    return (
      (await addNote(
        message.content,
        'answer',
        anchor ?? undefined,
      )) ?? false
    )
  }
  const state = { notes: doc.notes }
  return (
    <div className="reading-app">
      <header className={`workspace-bar ${headerControls ? 'has-window-controls' : ''}`} data-tauri-drag-region>
        {headerControls}
        <div className="document-identity">
          <Button size="sm" variant="ghost" onClick={() => void openPdf()}>
            <ArrowLeft size={15} />
            Kivio Study
          </Button>
          <BookOpen size={19} />
          <span className="document-name">{doc.name}</span>
        </div>
        <div className="workspace-actions">
          <span className="preview-label">
            {saveStatus}
          </span>

          <IconButton
            label={sideOpen ? '收起对话与笔记' : '展开对话与笔记'}
            variant="ghost"
            onClick={() => {
              if (sideOpen) setMobile('reader')
              setSideOpen(!sideOpen)
            }}
          >
            {sideOpen ? (
              <PanelRightClose size={17} />
            ) : (
              <PanelRightOpen size={17} />
            )}
          </IconButton>
        </div>
      </header>
      <nav className="mobile-tabs">
        <button
          aria-pressed={mobile === 'reader'}
          onClick={() => setMobile('reader')}
        >
          {edition}
        </button>
        <button
          aria-pressed={mobile === 'chat'}
          onClick={() => {
            setMobile('chat')
            setSideOpen(true)
          }}
        >
          对话与笔记
        </button>
      </nav>
      <main
        className={!sideOpen ? 'reader-only' : ''}
        style={{ '--reader-share': `${split}%` } as React.CSSProperties}
      >
        <section
          ref={reader}
          className={`reader-pane ${mobile === 'reader' ? 'is-mobile-active' : ''}`}
          aria-label="文档阅读器"
          onKeyDown={event => {
            if (event.key === 'Escape') { setMarkTool('select'); setSelection(null); setAreaDraft(null); areaDrag.current = null; return }
            if ((event.target as HTMLElement).closest('input, textarea, [contenteditable="true"], [role="combobox"]')) return
            if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'f') { event.preventDefault(); setNav('search'); return }
            const next = event.key === 'PageDown' ? page + 1 : event.key === 'PageUp' ? page - 1 : event.key === 'Home' ? 1 : event.key === 'End' ? pdf?.numPages : null
            if (next && pdf && next >= 1 && next <= pdf.numPages) { event.preventDefault(); jump(next) }
          }}
        >
          <div className="reader-tools">
            <div className="tool-group">
              <IconButton
                label="目录与页面"
                variant={nav === 'outline' ? 'default' : 'ghost'}
                onClick={() => setNav(nav === 'outline' ? null : 'outline')}
              >
                <PanelLeft size={16} />
              </IconButton>
              <IconButton
                label="搜索文档"
                variant={nav === 'search' ? 'default' : 'ghost'}
                onClick={() => setNav(nav === 'search' ? null : 'search')}
              >
                <Search size={16} />
              </IconButton>
              <span className="tool-rule" />
              <IconButton
                label="上一页"
                variant="ghost"
                disabled={page <= 1}
                onClick={() => jump(page - 1)}
              >
                <ChevronLeft size={15} />
              </IconButton>
              <form className="reader-page-jump" onSubmit={event => {
                event.preventDefault()
                const number = Number(pageInput)
                if (!pdf || !Number.isInteger(number) || number < 1 || number > pdf.numPages) { setPageInput(String(page)); return }
                jump(number)
                pageEditing.current = false
                ;(document.activeElement as HTMLElement)?.blur()
              }}>
                <div className="reader-page-input"><Input aria-label="跳转页码" inputMode="numeric" value={pageInput} disabled={!pdf} onChange={setPageInput} onFocus={event => { pageEditing.current = true; event.target.select() }} onBlur={() => { pageEditing.current = false; setPageInput(String(page)) }} /></div>
                <span className="page-count">/ {pdf?.numPages || '—'}</span>
              </form>
              <IconButton
                label="下一页"
                variant="ghost"
                disabled={!pdf || page === pdf.numPages}
                onClick={() => jump(page + 1)}
              >
                <ChevronRight size={15} />
              </IconButton>
            </div>
            {reading.session?.view.latest && <Select size="sm" triggerLabel={edition} ariaLabel="阅读版本" value={viewKind}
              disabled={!reading.session} onChange={kind => {
                setHighlight(null)
                void reading.selectView(kind as ReadingPdfKind).catch(error => notifyError(String(error)))
              }} options={[{ value: 'source', label: '原文' }, { value: 'mono', label: `译文 · ${readingLanguageName(shownOutput?.targetLang)}${shownOutput?.page ? ` · 第 ${shownOutput.page} 页` : ''}` }, { value: 'dual', label: '双语对照' }]} />}
            <div className="tool-group reader-zoom-controls">
              <IconButton
                label="缩小"
                variant="ghost"
                disabled={!pdf || zoom <= 0.4}
                onClick={() => { setFit('custom'); setZoom((z) => Math.max(0.4, Math.round((z - 0.1) * 100) / 100)) }}
              >
                <Minus size={15} />
              </IconButton>
              <Select size="sm" triggerLabel={fit === 'width' ? '适合宽度' : fit === 'page' ? '适合整页' : `${Math.round(zoom * 100)}%`} ariaLabel="页面缩放" value={fit === 'custom' ? String(Math.round(zoom * 100)) : fit} disabled={!pdf} onChange={value => {
                if (value === 'width' || value === 'page') setFit(value)
                else { setFit('custom'); setZoom(Number(value) / 100) }
              }} options={[{value:'width',label:'适合宽度'},{value:'page',label:'适合整页'}, ...Array.from(new Set([50,75,100,125,150,200, ...(fit === 'custom' ? [Math.round(zoom * 100)] : [])])).sort((a,b) => a-b).map(value => ({value:String(value),label:`${value}%`}))]} />
              <IconButton
                label="放大"
                variant="ghost"
                disabled={!pdf || zoom >= 2.5}
                onClick={() => { setFit('custom'); setZoom((z) => Math.min(2.5, Math.round((z + 0.1) * 100) / 100)) }}
              >
                <Plus size={15} />
              </IconButton>
            </div>
          <ReadingDocumentTools doc={doc} page={page} ready={!!pdf} noteDraft={noteDraft} model={selectedModel} modelControls={modelControls}
            session={reading.session} onTranslate={reading.translate} onCancel={reading.cancel}
            onRead={kind => { void reading.selectView(kind).catch(error => notifyError(String(error))) }}
            onConfigure={onConfigure}
            capturing={capturing} onPageImage={() => void capturePage()} />
          </div>
          <div className="reader-body">
          {nav && (
            <aside className="navigation-panel custom-scrollbar">
              <div className="navigation-heading">
                <strong>{nav === 'outline' ? '文档导航' : `搜索${edition}`}</strong>
                <IconButton
                  label="关闭导航"
                  variant="ghost"
                  onClick={() => setNav(null)}
                >
                  <X size={14} />
                </IconButton>
              </div>
              {nav === 'search' ? (
                <>
                  <Input
                    value={query}
                    onChange={setQuery}
                    placeholder="搜索词句…"
                    aria-label="搜索文档内容"
                    autoFocus
                  />
                  <p className="quiet">
                    {searching
                      ? '正在搜索…'
                      : query
                        ? `${results.length} 页包含匹配内容`
                        : '在整份文档中查找'}
                  </p>
                  {results.map((r) => (
                    <button
                      className="search-result"
                      key={r.page}
                      onClick={() => {
                        cite({ page: r.page, text: query, view: locationView })
                        setNav(null)
                      }}
                    >
                      <small>第 {r.page} 页</small>
                      <span>{r.text}</span>
                    </button>
                  ))}
                </>
              ) : toc.length ? (
                toc.map((t, i) => (
                  <button
                    className="outline-row"
                    key={i}
                    style={{ paddingLeft: 12 + t.depth * 12 }}
                    onClick={() => {
                      jump(t.page)
                      setNav(null)
                    }}
                  >
                    <span>{t.title}</span>
                    <small>{t.page}</small>
                  </button>
                ))
              ) : (
                <>
                  <p className="quiet">这份文件没有内置目录</p>
                  <div className="page-grid">
                    {Array.from({ length: pdf?.numPages || 0 }, (_, i) => (
                      <Button
                        key={i}
                        variant={page === i + 1 ? 'primary' : 'ghost'}
                        onClick={() => {
                          jump(i + 1)
                          setNav(null)
                        }}
                      >
                        {i + 1}
                      </Button>
                    ))}
                  </div>
                </>
              )}
            </aside>
          )}
          <div
            className={`document-scroll custom-scrollbar ${markTool === 'area' || markTool === 'capture' ? 'area-mode' : ''}`}
            tabIndex={0}
            aria-label="PDF 页面，可使用 PageUp 和 PageDown 翻页"
            ref={host}
            onScroll={onScroll}
            onMouseUp={() => { if (markTool !== 'area' && markTool !== 'capture') selectText() }}
            onKeyUp={event => { if (event.key === 'Shift') selectText() }}
            onPointerDown={event => {
              if ((markTool !== 'area' && markTool !== 'capture') || event.button !== 0) return
              const element = (event.target as HTMLElement).closest<HTMLElement>('[data-page]')
              if (!element) return
              event.preventDefault()
              const point = areaPoint(event, element)
              areaDrag.current = { element, ...point, page: Number(element.dataset.page), pointer: event.pointerId }
              event.currentTarget.setPointerCapture(event.pointerId)
              setSelection(null)
            }}
            onPointerMove={event => { if (areaDrag.current?.pointer === event.pointerId) setAreaDraft(areaAnchor(event)) }}
            onPointerUp={event => {
              if (areaDrag.current?.pointer !== event.pointerId) return
              const anchor = areaAnchor(event)
              areaDrag.current = null
              setAreaDraft(null)
              event.currentTarget.releasePointerCapture(event.pointerId)
              if (anchor?.rect && anchor.rect.w * width * zoom > 4 && anchor.rect.h * width * zoom > 4) {
                if (markTool === 'capture') { setMarkTool('select'); void capturePage(anchor) }
                else saveMark(anchor, 'area')
              }
            }}
            onLostPointerCapture={() => { areaDrag.current = null; setAreaDraft(null) }}
          >
            {!pdf && !error && (
              <div className="document-loading">
                <Loader2 size={20} />
                正在打开{edition}…
              </div>
            )}
            {error && (
              <div role="alert" className="document-loading">
                {error}
              </div>
            )}
            {pdf && <PdfPages key={viewKey} pdf={pdf} width={width} zoom={zoom} pageRatio={pageRatio}
              notes={doc.notes} viewKind={viewKind} viewRunId={viewRunId}
              highlight={highlight} areaDraft={areaDraft} markColor={markColor} onReady={locate} />}
          </div>
          <aside className="annotation-rail custom-scrollbar" aria-label="文献标注工具">
            <div role="group" aria-label="标注方式">
              {([
                ['select', '选择文字', MousePointer2], ['highlight', '高亮', Highlighter],
                ['underline', '下划线', Underline], ['strike', '删除线', Strikethrough], ['area', '框选标注', Scan],
              ] as const).map(([tool, label, Icon]) => <IconButton key={tool} label={label} size="md" variant={markTool === tool ? 'default' : 'ghost'} aria-pressed={markTool === tool} disabled={!pdf} onClick={() => { setMarkTool(tool); if (selection && tool !== 'select' && tool !== 'area') saveMark(selection, tool); else setSelection(null) }}><Icon size={18} /></IconButton>)}
            </div>
            <IconButton label={capturing ? '正在准备截图' : '框选截图到 AI'} size="md" variant={markTool === 'capture' ? 'default' : 'ghost'} aria-pressed={markTool === 'capture'} disabled={!pdf || capturing} onClick={() => { setMarkTool('capture'); setSelection(null) }}><Camera size={18} /></IconButton>
            <div role="group" aria-label="标注颜色" className="annotation-colors">
              {([['#d3a51d', '黄色'], ['#db627e', '粉色'], ['#4b95c9', '蓝色'], ['#55a477', '绿色']] as const).map(([color, label]) => <IconButton key={color} label={`${label}标注`} variant={markColor === color ? 'default' : 'ghost'} aria-pressed={markColor === color} onClick={() => setMarkColor(color)}><Circle size={14} fill={color} stroke={color} /></IconButton>)}
            </div>
            <IconButton label="查看标注与笔记" size="md" variant="ghost" onClick={() => openSide('notes')}><StickyNote size={18} /></IconButton>
          </aside>
          </div>
          {markTool === 'capture' && <div className="capture-hint" role="status">拖动框选图表或公式，松开后加入右侧对话 · Esc 取消</div>}
          {selection && (
            <div
              className="selection-actions"
              style={{ left: selection.left, top: selection.top }}
            >
              <Button
                size="sm"
                variant="primary"
                onClick={() => attach('提问')}
              >
                提问
              </Button>
              <Button size="sm" variant="ghost" onClick={() => attach('解释')}>
                解释
              </Button>
              <Button size="sm" variant="ghost" onClick={() => attach('翻译')}>
                翻译
              </Button>
              <Button size="sm" variant="ghost" onClick={() => attach('总结')}>总结</Button>
              <Button size="sm" variant="ghost" onClick={() => attach('公式')}>讲公式</Button>
              <IconButton label="选区截图到 AI" variant="ghost" disabled={capturing} onClick={() => { void capturePage(selection); setSelection(null); window.getSelection()?.removeAllRanges() }}><Camera size={16} /></IconButton>
              <IconButton
                label="保存高亮"
                variant="ghost"
                onClick={() => {
                  saveMark(selection, 'highlight')
                }}
              >
                <Highlighter size={16} />
              </IconButton>
              <IconButton
                label="取消选区"
                variant="ghost"
                onClick={() => setSelection(null)}
              >
                <X size={14} />
              </IconButton>
            </div>
          )}
          {returnTo && (
            <div className="return-position">
              <Button
                size="sm"
                onClick={() => {
                  const target = returnTo
                  if (anchorMatches({ page: target.page, text: '', view: target.view })) jump(target.page, target.offset)
                  else {
                    position.current = { page: target.page, offset: target.offset }
                    void reading.selectView(target.view?.kind ?? 'source', target.view?.runId).catch(error => notifyError(String(error)))
                  }
                  setReturnTo(null)
                  setHighlight(null)
                }}
              >
                <ArrowLeft size={14} />
                返回阅读位置
              </Button>
            </div>
          )}
        </section>
        {sideOpen && (
          <>
            <div
              className="reading-divider"
              role="separator"
              aria-label="调整阅读区宽度"
              aria-valuenow={split}
              aria-valuemin={48}
              aria-valuemax={75}
              tabIndex={0}
              onKeyDown={(e) => {
                if (e.key === 'ArrowLeft') setSplit((v) => Math.max(48, v - 2))
                if (e.key === 'ArrowRight') setSplit((v) => Math.min(75, v + 2))
              }}
              onPointerDown={(e) => {
                e.currentTarget.setPointerCapture(e.pointerId)
                e.currentTarget.dataset.drag = 'yes'
              }}
              onPointerUp={(e) => delete e.currentTarget.dataset.drag}
              onPointerCancel={(e) => delete e.currentTarget.dataset.drag}
              onPointerMove={(e) => {
                if (e.currentTarget.dataset.drag !== 'yes') return
                const r = e.currentTarget.parentElement!.getBoundingClientRect()
                setSplit(
                  Math.max(
                    48,
                    Math.min(75, ((e.clientX - r.x) / r.width) * 100),
                  ),
                )
              }}
            />
            <section
              className={`conversation-pane ${mobile === 'chat' ? 'is-mobile-active' : ''}`}
              aria-label="对话与笔记"
            >
              <div className="reading-chat-toolbar">
                <IconButton label="新建对话" variant="ghost" disabled={preparing || !!busy} onClick={() => void newConversation()}><ComposeIcon size={16} strokeWidth={1.75} /></IconButton>
                {side === 'chat' && <div className="reading-chat-model">{modelControls}</div>}
                <nav className="reading-chat-actions" aria-label="学习工作区">
                  <Button size="sm" variant={side === 'chat' ? 'default' : 'ghost'} aria-pressed={side === 'chat'} onClick={() => setSide('chat')}>对话</Button>
                  <Button size="sm" variant={side === 'notes' ? 'default' : 'ghost'} aria-pressed={side === 'notes'} onClick={() => setSide('notes')}>笔记{state.notes.length ? ` ${state.notes.length}` : ''}</Button>
                  <Button size="sm" variant={side === 'history' ? 'default' : 'ghost'} aria-pressed={side === 'history'} onClick={() => setSide('history')}>历史</Button>
                </nav>

              </div>
              <div
                className="chat-surface"
                style={{ display: side === 'chat' ? 'flex' : 'none' }}
                onClickCapture={(e) => {
                  const href = (e.target as HTMLElement)
                    .closest('a')
                    ?.getAttribute('href')
                  if (!href?.startsWith('#reading-ref?') && !href?.startsWith('#reading-page-')) return
                  e.preventDefault()
                  e.stopPropagation()
                  const messageId = (e.target as HTMLElement).closest<HTMLElement>('[data-message-id]')?.dataset.messageId
                  const anchor = readingMessageAnchor(href, messages, messageId)
                  if (anchor) cite(anchor)
                  else notifyError('这条旧引用未记录完整的文档版本，无法准确定位。请重新引用对应版本。')
                }}
              >
                {!messages.length && (
                  <div className="reading-empty custom-scrollbar">
                    <div className="reading-empty-content">
                      <h2>一起读懂这篇文献</h2>
                      <p>选中文档内容深入提问，或从一个问题开始。</p>
                      <div className="reading-starters">
                        {[
                          ['概括核心观点', '请概括这篇文献的研究问题、核心观点和主要结论，并引用原文页码。'],
                          ['梳理研究方法', '请梳理这篇文献的研究方法，解释关键步骤、假设与适用条件，并引用原文页码。'],
                          ['分析贡献与局限', '请分析这篇文献的主要贡献与局限，区分作者的陈述和你的判断，并引用原文页码。'],
                        ].map(([label, prompt]) => (
                          <Button
                            key={label}
                            variant="ghost"
                            size="sm"
                            onClick={() => insertTextIntoComposer(prompt)}
                          >
                            {label}
                            <ArrowUpRight size={13} />
                          </Button>
                        ))}
                      </div>
                    </div>
                  </div>
                )}
                {chatReady && messages.length > 0 && (
                  <AttachmentPresentationContext.Provider value="compact"><MessageList
                    density="compact"
                    {...messageListProps}
                    onSaveMessageToNote={saveAnswer}
                  /></AttachmentPresentationContext.Provider>
                )}
                <InputBar
                  {...inputBarProps}
                  appearance="embedded"
                  usageSlot={undefined}
                  conversationId={composerId}
                  onSend={send}
                  onSendWhileRunning={undefined}
                  disabled={!!busy || preparing || chatLoading || !pdf}
                  cancelVisible={chatReady && inputBarProps.cancelVisible}
                  onNewChat={() => void newConversation()}
                  onClearChat={undefined}
                  onModeChange={undefined}
                  modeOptions={undefined}
                  presetOptions={undefined}
                  currentAssistant={undefined}
                  onSelectAssistant={undefined}
                  onOpenAssistantCenter={undefined}
                  onChangeReplyModels={undefined}
                  replyModels={[]}
                  onOpenGitPanel={undefined}
                  gitWorkdir={null}
                  onOpenTools={undefined}
                  onReloadSkills={undefined}
                  usesChatRuntime
                  usesExternalRuntime={false}
                  showProjectEntry={false}
                  contextSlot={
                    <div className="reading-context-status">
                      <span className="context-label"><FileText size={12} />{indexing ? `正在读取${edition}…` : `${edition} ${pages.length} 页`}</span>
                      {inputBarProps.usageSlot && <details className="reading-usage"><summary>用量</summary><div>{inputBarProps.usageSlot}</div></details>}
                    </div>
                  }
                />
              </div>
              {side === 'history' && (
                <div className="reading-history custom-scrollbar">
                  <div className="reading-history-heading">
                    <h2>对话历史</h2>
                  </div>
                  {historyLoading ? <p role="status">正在加载对话…</p> : (
                    <>
                      {!history.length && <p>还没有对话。新建一个，开始讨论这篇文献。</p>}
                      {history.some((item) => !item.conversation) && (
                        <div className="reading-history-unavailable" role="status">
                          <span>{history.filter(item => !item.conversation).length} 条对话暂时无法读取</span>
                          <Button size="sm" variant="ghost" onClick={() => setHistoryRevision((value) => value + 1)}>重试</Button>
                        </div>
                      )}
                      {history.filter(item => item.conversation).map(({ id, conversation }) => {
                        const question = conversation?.messages.find((message) => message.role === 'user')?.content.trim()
                        const lastQuestion = conversation?.messages.filter((message) => message.role === 'user').at(-1)?.content.trim()
                        return (
                          <button
                            className="reading-history-item"
                            key={id}
                            disabled={!conversation || preparing}
                            aria-current={id === doc.activeConversationId ? 'true' : undefined}
                            onClick={() => { persist({ activeConversationId: id }); setSide('chat') }}
                          >
                            <span className="reading-history-title">{question || (conversation ? '尚未提问' : '对话加载失败')}</span>
                            {lastQuestion && lastQuestion !== question && <span className="reading-history-preview">最近：{lastQuestion}</span>}
                            <span className="reading-history-meta">
                              {conversation && new Date(conversation.updated_at * 1000).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })}
                              {id === doc.activeConversationId && <span>当前对话</span>}
                            </span>
                          </button>
                        )
                      })}
                    </>
                  )}
                </div>
              )}
              {side === 'notes' && (
                <div className="notes-surface custom-scrollbar" data-chat-density="compact">
                  <div className="notes-intro">
                    <h2>阅读笔记</h2>
                    <p>标注、想法和保存的回答，集中在这里。</p>
                  </div>
                  <TextArea
                    value={noteDraft}
                    onChange={setNoteDraft}
                    rows={3}
                    placeholder="记下一个想法…"
                    aria-label="笔记内容"
                  />
                  <div className="note-compose-actions">
                    <span>{saveStatus}</span>
                    <Button
                      size="sm"
                      disabled={!noteDraft.trim() || noteSaving}
                      onClick={() => void saveDraft()}
                    >
                      保存笔记
                    </Button>
                  </div>
                  {!state.notes.length && (
                    <p className="note-empty">
                      在文档中选中文字并保存高亮，
                      <br />
                      或将一条有用的回答存为笔记。
                    </p>
                  )}
                  {state.notes.map((n) => (
                    <article className="reading-note" key={n.id} aria-current={n.id === initialTarget?.noteId ? 'true' : undefined} ref={n.id === initialTarget?.noteId ? initialNoteRef : undefined}>
                      <div className="note-meta">
                        <span>
                          {n.kind === 'highlight'
                            ? `${n.anchor?.view?.kind === 'dual' ? '双语对照' : n.anchor?.view ? '译文' : '原文'} · ${{ highlight: '高亮', underline: '下划线', strike: '删除线', area: '框选' }[n.anchor?.mark ?? 'highlight']}`
                            : n.kind === 'answer'
                              ? '来自对话'
                              : '我的笔记'}
                        </span>
                        {n.anchor && (
                          <Button
                            variant="ghost"
                            size="sm"
                            onClick={() => cite(n.anchor!)}
                          >
                            第 {n.anchor.page} 页<ArrowUpRight size={12} />
                          </Button>
                        )}
                        <IconButton label="删除这条笔记或标注" variant="ghost" onClick={() => void updateReadingDocument(doc.id, document => ({ ...document, notes: document.notes.filter(note => note.id !== n.id) })).catch(() => notifyError('删除失败，请重试保存'))}><Trash2 size={14} /></IconButton>
                      </div>
                      <div className="note-actions">
                        <Button size="sm" variant="ghost" disabled={capturing} onClick={() => {
                          if (n.anchor?.mark === 'area') void capturePage(n.anchor)
                          else if (n.anchor) attach('解释', { ...n.anchor, text: n.content })
                          else { openSide('chat'); updateComposerDraft(composerId, draft => ({ ...draft, quotes: [...draft.quotes, n.content] })) }
                        }}><MessageSquare size={14} />交给 AI</Button>
                        {n.anchor?.rect && n.anchor.mark !== 'area' && <Button size="sm" variant="ghost" disabled={capturing} onClick={() => void capturePage(n.anchor!)}><Camera size={14} />截图提问</Button>}
                      </div>
                      <div className="note-content">
                        <ChatMarkdown content={n.content} />
                      </div>
                    </article>
                  ))}
                </div>
              )}
            </section>
          </>
        )}
      </main>
      <ErrorNotice message={chatError} />
      <ErrorNotice message={reading.error} />
      <OperationNotice message={notice} tone="success" />
    </div>
  )
}
function rectStyle(rect: Rect, width: number): React.CSSProperties {
  return {
    left: `calc(50% - ${width / 2}px + ${rect.x * width}px)`,
    top: `${rect.y * 100}%`,
    width: rect.w * width,
    height: `${rect.h * 100}%`,
  }
}
