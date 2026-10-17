import { ErrorNotice, OperationNotice } from '../../components/AppNotice'
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
  History,
  Minus,
  NotebookPen,
  Highlighter,
  PanelRightClose,
  PanelRightOpen,
  Loader2,
} from 'lucide-react'
import { Button, IconButton } from '../../components/Button'
import { ComposeIcon } from '../../settings/public/icons'
import { Input, TextArea } from '../../settings/public/controls'
import { ChatMarkdown } from '../ChatMarkdown'
import { MessageList, type MessageListProps } from '../MessageList'
import { InputBar, type InputBarProps } from '../InputBar'
import { insertTextIntoComposer } from '../composerInsert'
import { updateComposerDraft, migrateComposerDraft } from '../composerDraft'
import * as pdfjs from 'pdfjs-dist'
import worker from 'pdfjs-dist/build/pdf.worker.min.mjs?url'
import './pdfTextLayer.css'
import './reading.css'
pdfjs.GlobalWorkerOptions.workerSrc = worker

import LiteratureLibrary from './LiteratureLibrary'
import {
  readingApi,
  type ReadingDocument,
  type ReadingAnchor as Anchor,
  type ReadingRect as Rect,
  type ReadingNote as Note,
} from '../../api/reading'
import {
  useReadingLibrary,
  loadReadingLibrary,
  updateReadingDocument,
  retryReadingSaves,
} from './readingLibrary'
import { readingContext, readingCitation } from './readingContext'
import { chatApi } from '../api'
import type { Conversation, PendingAttachment } from '../types'
function PdfPage({
  pdf,
  number,
  width,
  zoom,
  onReady,
}: {
  pdf: pdfjs.PDFDocumentProxy
  number: number
  width: number
  zoom: number
  onReady: () => void
}) {
  const onReadyRef = useRef(onReady)
  onReadyRef.current = onReady
  const el = useRef<HTMLDivElement>(null)
  const [near, setNear] = useState(false),
    [ratio, setRatio] = useState(792 / 612),
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
}

export interface ReadingCenterProps {
  headerControls?: React.ReactNode
  inputBarProps: InputBarProps
  messageListProps: MessageListProps
  modelControls: React.ReactNode
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
  useEffect(() => {
    void loadReadingLibrary()
  }, [])
  const select = (document: ReadingDocument) => {
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
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <ErrorNotice
        message={error || library.error}
        actionLabel="重试保存 / 加载"
        onAction={() => {
          setError('')
          void retryReadingSaves().then(() => loadReadingLibrary()).catch((error) => setError(String(error)))
        }}
      />
      <div className={doc ? 'hidden' : 'flex min-h-0 flex-1 flex-col'}>
        <LiteratureLibrary onRead={select} headerControls={props.headerControls} />
      </div>
      {doc ? (
        <ReadingWorkspace
          key={doc.id}
          {...props}
          doc={doc}
          openPdf={async () => setActiveId(null)}
          saving={library.saving > 0}
        />
      ) : null}
    </div>
  )
}
function ReadingWorkspace({
  doc,
  openPdf,
  saving,
  inputBarProps,
  messageListProps,
  modelControls,
  headerControls,
  onCreateConversation,
  onActivateConversation,
  onSend,
}: ReadingCenterProps & {
  doc: ReadingDocument
  openPdf: () => Promise<void>
  saving: boolean
}) {
  const [pdf, setPdf] = useState<pdfjs.PDFDocumentProxy | null>(null),
    [error, setError] = useState('')
  const [pages, setPages] = useState<string[]>([]),
    [indexing, setIndexing] = useState(true)
  const [page, setPage] = useState(doc.page),
    [width, setWidth] = useState(720),
    [zoom, setZoom] = useState(1)
  const [side, setSide] = useState<'chat' | 'notes' | 'history'>('chat'),
    [sideOpen, setSideOpen] = useState(true),
    [mobile, setMobile] = useState('reader')
  const [nav, setNav] = useState<'outline' | 'search' | null>(null),
    [toc, setToc] = useState<{ title: string; page: number; depth: number }[]>(
      [],
    )
  const [query, setQuery] = useState(''),
    [notice, setNotice] = useState('')
  const [selection, setSelection] = useState<
      (Anchor & { left: number; top: number }) | null
    >(null),
    [highlight, setHighlight] = useState<Anchor | null>(null)
  const [noteDraft, setNoteDraft] = useState(doc.noteDraft),
    [returnTo, setReturnTo] = useState<{ page: number; offset: number } | null>(
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
    void Promise.all(doc.conversationIds.map(async (id) => {
      try { return { id, conversation: await chatApi.getConversation(id) } }
      catch { return { id, conversation: null } }
    })).then((items) => {
      if (!live) return
      setHistory(items.reverse())
      setHistoryLoading(false)
    })
    return () => { live = false }
  }, [side, doc.conversationIds, historyRevision])
  const host = useRef<HTMLDivElement>(null),
    reader = useRef<HTMLElement>(null),
    alive = useRef(true)
  const position = useRef({ page: doc.page, offset: doc.offset }),
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
      (error) => {
        if (live) {
          setChatLoading(false)
          setChatError(String(error))
        }
      },
    )
    return () => {
      live = false
    }
  }, [doc.activeConversationId, onActivateConversation])
  useEffect(() => {
    let live = true
    let task: pdfjs.PDFDocumentLoadingTask | undefined
    void (async () => {
      try {
        const data = await readingApi.pdf(doc.id)
        if (!live) return
        task = pdfjs.getDocument({ data })
        const pdf = await task.promise
        if (!live) return
        setPdf(pdf)
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
        }
      } catch (error) {
        if (live) {
          setError(`无法读取 PDF：${String(error)}`)
          setIndexing(false)
        }
      }
    })()
    return () => {
      live = false
      void task?.destroy()
    }
  }, [doc.id])
  useEffect(() => {
    const el = host.current
    if (!el) return
    const observer = new ResizeObserver(() => {
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
        host.current.scrollTo({
          top: target.parentElement!.offsetTop + offset,
          behavior: 'instant',
        })
        setPage(number)
      }
    },
    [frame],
  )
  useEffect(() => {
    if (pdf)
      requestAnimationFrame(() =>
        jump(position.current.page, position.current.offset),
      )
  }, [pdf, jump])
  const locate = useCallback(() => {
    if (!highlight || !host.current) return
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
  }, [highlight, frame])
  useEffect(locate, [locate])
  useEffect(() => {
    if (!highlight) return
    const timer = setTimeout(() => setHighlight(null), 8000)
    return () => clearTimeout(timer)
  }, [highlight])
  const cite = (anchor: Anchor) => {
    setReturnTo({ ...position.current })
    setHighlight(anchor)
    setMobile('reader')
    jump(anchor.page)
    setSelection(null)
  }
  const onScroll = () => {
    if (!host.current) return
    setSelection(null)
    const top = host.current.scrollTop + 100
    const frames = Array.from(
      host.current.querySelectorAll<HTMLElement>('[data-page]'),
    )
    const visible = frames
      .reverse()
      .find((frame) => frame.parentElement!.offsetTop <= top)
    if (visible) {
      const page = Number(visible.dataset.page)
      setPage(page)
      position.current = {
        page,
        offset: host.current.scrollTop - visible.parentElement!.offsetTop,
      }
      clearTimeout(pendingPosition.current)
      pendingPosition.current = setTimeout(() => persist(position.current), 400)
    }
  }
  const selectText = () => {
    const selected = window.getSelection()
    if (!selected || selected.isCollapsed || !host.current || !reader.current)
      return
    const frame =
      selected.anchorNode?.parentElement?.closest<HTMLElement>('[data-page]')
    if (
      !frame ||
      frame !== selected.focusNode?.parentElement?.closest('[data-page]')
    )
      return
    const text = selected.toString().trim()
    if (!text) return
    const r = selected.getRangeAt(0).getBoundingClientRect(),
      s = frame.getBoundingClientRect(),
      bounds = reader.current.getBoundingClientRect()
    setSelection({
      page: Number(frame.dataset.page),
      text: text.slice(0, 6000),
      rect: {
        x: (r.x - s.x) / s.width,
        y: (r.y - s.y) / s.height,
        w: r.width / s.width,
        h: r.height / s.height,
      },
      left: Math.max(12, Math.min(r.x - bounds.x, bounds.width - 300)),
      top: Math.max(48, Math.min(r.bottom - bounds.y + 8, bounds.height - 56)),
    })
  }
  const composerId = doc.activeConversationId ?? `reading-${doc.id}`
  const attach = (action: string) => {
    if (!selection) return
    setSide('chat')
    setSideOpen(true)
    setMobile('chat')
    updateComposerDraft(composerId, (draft) => ({
      ...draft,
      input: [
        draft.input,
        action === '解释'
          ? '请用通俗的话解释这段内容。'
          : action === '翻译'
            ? '请把这段内容翻译成中文。'
            : '',
      ]
        .filter(Boolean)
        .join('\n'),
      quotes: [
        ...draft.quotes,
        `[第 ${selection.page} 页](#reading-page-${selection.page})\n${selection.text}`,
      ],
    }))
    setSelection(null)
    window.getSelection()?.removeAllRanges()
  }
  const addNote = async (
    content: string,
    kind: Note['kind'],
    anchor?: Anchor,
  ) => {
    if (!content.trim()) return
    try {
      await updateReadingDocument(doc.id, (document) => ({
        ...document,
        notes: [
          { id: crypto.randomUUID(), content, kind, anchor },
          ...document.notes,
        ],
      }))
      setNotice(kind === 'highlight' ? '已保存高亮' : '已保存到阅读笔记')
      return true
    } catch {
      setChatError('保存失败，请通过操作提示重试保存')
      return false
    }
  }
  const saveDraft = async () => {
    if (noteSaveLock.current) return
    const submitted = noteDraft
    noteSaveLock.current = true
    setNoteSaving(true)
    try {
      if (await addNote(submitted, 'note'))
        setNoteDraft((current) => (current === submitted ? '' : current))
    } finally {
      noteSaveLock.current = false
      if (alive.current) setNoteSaving(false)
    }
  }
  const createConversation = async () => {
    const conversation = await onCreateConversation(doc.name)
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
    if (sendLock.current || indexing || chatLoading) return false
    sendLock.current = true
    const sequence = ++sendSequence.current
    setPreparing(true)
    setChatError('')
    try {
      const context = readingContext(doc.name, pages, content, page)
      const conversation = doc.activeConversationId
        ? await chatApi.getConversation(doc.activeConversationId)
        : await createConversation()
      if (!alive.current) return false
      await onActivateConversation(conversation.id)
      if (!alive.current) return false
      return await onSend(
        conversation,
        content,
        [context, ...attachments],
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
    const citation = /\(#reading-page-(\d+)\)/.exec(message.content)
    return (
      (await addNote(
        message.content,
        'answer',
        citation ? { page: Number(citation[1]), text: '' } : undefined,
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
            {saving ? '正在保存…' : '已保存到本机'}
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
          原文
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
              <span className="page-count">
                {page} <span>/ {pdf?.numPages || '—'}</span>
              </span>
              <IconButton
                label="下一页"
                variant="ghost"
                disabled={!pdf || page === pdf.numPages}
                onClick={() => jump(page + 1)}
              >
                <ChevronRight size={15} />
              </IconButton>
            </div>
            <div className="tool-group">
              <IconButton
                label="缩小"
                variant="ghost"
                disabled={zoom <= 0.8}
                onClick={() => setZoom((z) => Math.max(0.8, z - 0.1))}
              >
                <Minus size={15} />
              </IconButton>
              <Button variant="ghost" size="sm" onClick={() => setZoom(1)}>
                {zoom === 1 ? '适合宽度' : `${Math.round(zoom * 100)}%`}
              </Button>
              <IconButton
                label="放大"
                variant="ghost"
                disabled={zoom >= 1.8}
                onClick={() => setZoom((z) => Math.min(1.8, z + 0.1))}
              >
                <Plus size={15} />
              </IconButton>
            </div>
          </div>
          {nav && (
            <aside className="navigation-panel custom-scrollbar">
              <div className="navigation-heading">
                <strong>{nav === 'outline' ? '文档导航' : '搜索原文'}</strong>
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
                        cite({ page: r.page, text: query })
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
            className="document-scroll custom-scrollbar"
            ref={host}
            onScroll={onScroll}
            onMouseUp={selectText}
          >
            {!pdf && !error && (
              <div className="document-loading">
                <Loader2 size={20} />
                正在打开原文…
              </div>
            )}
            {error && (
              <div role="alert" className="document-loading">
                {error}
              </div>
            )}
            {pdf &&
              Array.from({ length: pdf.numPages }, (_, i) => (
                <div className="page-container" key={`${doc.id}-${i}`}>
                  <PdfPage
                    pdf={pdf}
                    number={i + 1}
                    width={width}
                    zoom={zoom}
                    onReady={locate}
                  />
                  {state.notes
                    .filter(
                      (n) =>
                        n.kind === 'highlight' &&
                        n.anchor?.page === i + 1 &&
                        n.anchor.rect,
                    )
                    .map((n) => (
                      <div
                        key={n.id}
                        className="source-mark saved"
                        style={rectStyle(n.anchor!.rect!, width * zoom)}
                      />
                    ))}
                  {highlight?.page === i + 1 && highlight.rect && (
                    <div
                      className="source-mark"
                      style={rectStyle(highlight.rect, width * zoom)}
                    />
                  )}
                </div>
              ))}
          </div>
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
              <IconButton
                label="保存高亮"
                variant="ghost"
                onClick={() => {
                  addNote(selection.text, 'highlight', selection)
                  setSelection(null)
                  window.getSelection()?.removeAllRanges()
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
                  jump(returnTo.page, returnTo.offset)
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
                <div className="reading-chat-model">{modelControls}</div>
                <div className="reading-chat-actions">
                  <Button
                    size="sm"
                    variant="ghost"
                    aria-pressed={side === 'notes'}
                    onClick={() => setSide(side === 'notes' ? 'chat' : 'notes')}
                  >
                    {side === 'notes' ? <ArrowLeft size={14} /> : <NotebookPen size={14} />}
                    {side === 'notes' ? '返回对话' : `笔记${state.notes.length ? ` ${state.notes.length}` : ''}`}
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    aria-pressed={side === 'history'}
                    onClick={() => setSide(side === 'history' ? 'chat' : 'history')}
                  >
                    <History size={14} />
                    {side === 'history' ? '返回对话' : '历史'}
                  </Button>
                  <IconButton
                    label="新建对话"
                    variant="ghost"
                    disabled={preparing || !!busy}
                    onClick={() => void newConversation()}
                  >
                    <ComposeIcon size={16} strokeWidth={1.75} />
                  </IconButton>
                </div>
              </div>
              <div
                className="chat-surface"
                style={{ display: side === 'chat' ? 'flex' : 'none' }}
                onClickCapture={(e) => {
                  const href = (e.target as HTMLElement)
                    .closest('a')
                    ?.getAttribute('href')
                  const page = readingCitation(href ?? '', pdf?.numPages ?? 0)
                  if (!page) return
                  e.preventDefault()
                  e.stopPropagation()
                  cite({ page, text: '' })
                }}
              >
                {!messages.length && (
                  <div className="reading-empty custom-scrollbar">
                    <div className="reading-empty-content">
                      <h2>一起读懂这篇文献</h2>
                      <p>选中原文深入提问，或从一个问题开始。</p>
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
                  <MessageList
                    {...messageListProps}
                    onSaveMessageToNote={saveAnswer}
                  />
                )}
                <InputBar
                  {...inputBarProps}
                  conversationId={composerId}
                  onSend={send}
                  onSendWhileRunning={undefined}
                  disabled={!!busy || preparing || chatLoading || indexing}
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
                    <span className="context-label">
                      <FileText size={12} />
                      {indexing ? '正在提取原文…' : `原文 ${pages.length} 页`}
                    </span>
                  }
                />
              </div>
              {side === 'history' && (
                <div className="reading-history custom-scrollbar">
                  <div className="reading-history-heading">
                    <h2>这篇文献的对话</h2>
                  </div>
                  {historyLoading ? <p role="status">正在加载对话…</p> : (
                    <>
                      {!history.length && <p>还没有对话。新建一个，开始讨论这篇文献。</p>}
                      {history.some((item) => !item.conversation) && (
                        <Button size="sm" variant="ghost" onClick={() => setHistoryRevision((value) => value + 1)}>
                          重试加载失败的对话
                        </Button>
                      )}
                      {history.map(({ id, conversation }) => {
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
                <div className="notes-surface custom-scrollbar">
                  <div className="notes-intro">
                    <h2>留下读懂的部分</h2>
                    <p>高亮、想法和保存的回答，集中在这里。</p>
                  </div>
                  <TextArea
                    value={noteDraft}
                    onChange={setNoteDraft}
                    rows={3}
                    placeholder="记下一个想法…"
                    aria-label="笔记内容"
                  />
                  <div className="note-compose-actions">
                    <span>{saving ? '正在保存…' : '自动保存在本机'}</span>
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
                      在原文中选中文字并保存高亮，
                      <br />
                      或将一条有用的回答存为笔记。
                    </p>
                  )}
                  {state.notes.map((n) => (
                    <article className="reading-note" key={n.id}>
                      <div className="note-meta">
                        <span>
                          {n.kind === 'highlight'
                            ? '原文摘录'
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
