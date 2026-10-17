import { ErrorNotice, OperationNotice } from '../../components/AppNotice'
import { useEffect, useMemo, useRef, useState, type ReactNode, type PointerEvent, type CSSProperties } from 'react'
import { open, save } from '@tauri-apps/plugin-dialog'
import {
  BookOpen,
  FileText,
  Folder,
  FolderPlus,
  Import,
  Plus,
  Search,
  Star,
  Trash2,
  X,
  ArrowUpDown,
  RotateCcw,
  Tag,
  ChevronRight,
  Pencil,
  MoreHorizontal,
  SlidersHorizontal,
} from 'lucide-react'
import * as pdfjs from 'pdfjs-dist'
import { StudyIcon } from '../../settings/public/icons'
import { Button, IconButton } from '../../components/Button'
import { Input, Select, TextArea } from '../../settings/public/controls'
import {
  emptyBibliography,
  readingApi,
  type ReadingCollection,
  type ReadingDocument,
} from '../../api/reading'
import {
  attachReadingPdf,
  createReadingItem,
  importReadingDocument,
  importReadingRis,
  exportReadingRis,
  updateReadingCollections,
  updateReadingDocument,
  useReadingLibrary,
} from './readingLibrary'
import {
  collectionDescendants,
  literatureLayout,
  filterLiterature,
  itemTypes,
  mergeBibliography,
  type LibraryFilter,
} from './literature'
import { PdfThumbnail } from './PdfThumbnail'
import './literature.css'

function LibraryDialog({
  busy,
  title,
  onClose,
  children,
}: {
  busy: boolean
  title: string
  onClose: () => void
  children: React.ReactNode
}) {
  const ref = useRef<HTMLDialogElement>(null)
  useEffect(() => {
    ref.current?.showModal()
  }, [])
  return (
    <dialog
      ref={ref}
      className="kv-modal literature-dialog"
      aria-label={title}
      onCancel={(event) => {
        event.preventDefault()
        onClose()
      }}
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose()
      }}
    >
      <div className="literature-dialog-heading">
        <h2>{title}</h2>
        <IconButton
          label="关闭"
          variant="ghost"
          onClick={onClose}
          disabled={busy}
        >
          <X size={16} />
        </IconButton>
      </div>
      <fieldset disabled={busy} className="literature-dialog-fields">
        {children}
      </fieldset>
    </dialog>
  )
}

export default function LiteratureLibrary({
  onRead,
  headerControls,
}: {
  headerControls?: ReactNode
  onRead: (doc: ReadingDocument) => void
}) {
  const { documents, collections, loading, saving } = useReadingLibrary()
  const bodyRef = useRef<HTMLDivElement>(null)
  const [bodyWidth, setBodyWidth] = useState(0)
  const [navigationWidth, setNavigationWidth] = useState(240)
  const [inspectorWidth, setInspectorWidth] = useState(320)
  const [inspectorOpen, setInspectorOpen] = useState(false)
  const layout = literatureLayout(bodyWidth, navigationWidth, inspectorWidth)
  useEffect(() => {
    const body = bodyRef.current
    if (!body) return
    const observer = new ResizeObserver(() => setBodyWidth(body.clientWidth))
    setBodyWidth(body.clientWidth)
    observer.observe(body)
    return () => observer.disconnect()
  }, [])
  const resizePanel = (side: 'navigation' | 'inspector', event: PointerEvent<HTMLDivElement>) => {
    if (!event.currentTarget.hasPointerCapture(event.pointerId) || !bodyRef.current) return
    const rect = bodyRef.current.getBoundingClientRect()
    // Account for the app's UI scale as well as the actual available container width.
    const scale = bodyRef.current.clientWidth / rect.width
    if (side === 'navigation') setNavigationWidth(Math.max(200, Math.min(layout.navigationMax, (event.clientX - rect.left) * scale)))
    else setInspectorWidth(Math.max(280, Math.min(layout.inspectorMax, (rect.right - event.clientX) * scale)))
  }
  const divider = (side: 'navigation' | 'inspector') => {
    const navigation = side === 'navigation'
    const value = navigation ? layout.navigationWidth : layout.inspectorWidth
    const min = navigation ? 200 : 280
    const max = navigation ? layout.navigationMax : layout.inspectorMax
    const setWidth = navigation ? setNavigationWidth : setInspectorWidth
    return <div
      className="literature-divider"
      role="separator"
      aria-orientation="vertical"
      aria-label={navigation ? '调整文献集栏宽度' : '调整资料栏宽度'}
      aria-valuemin={min} aria-valuemax={max} aria-valuenow={Math.round(value)}
      tabIndex={0}
      onDoubleClick={() => setWidth(navigation ? 240 : 320)}
      onKeyDown={(e) => {
        if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(e.key)) return
        e.preventDefault()
        const delta = (e.key === 'ArrowRight' ? 16 : -16) * (navigation ? 1 : -1)
        setWidth(e.key === 'Home' ? min : e.key === 'End' ? max : Math.max(min, Math.min(max, value + delta)))
      }}
      onPointerDown={(e) => { if (e.button === 0) { e.preventDefault(); e.currentTarget.setPointerCapture(e.pointerId) } }}
      onPointerMove={(e) => resizePanel(side, e)}
      onPointerUp={(e) => { if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId) }}
    />
  }

  const [filter, setFilter] = useState<LibraryFilter>('all')
  const [query, setQuery] = useState('')
  const [tags, setTags] = useState<string[]>([])
  const [sort, setSort] = useState('opened')
  const [showFilters, setShowFilters] = useState(false)
  const [unfiledOnly, setUnfiledOnly] = useState(false)
  const [selected, setSelected] = useState<string[]>([])
  const [busy, setBusy] = useState('')
  const busyRef = useRef(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const [dialog, setDialog] = useState<'item' | 'collection' | null>(null)
  const [title, setTitle] = useState('')
  const [doi, setDoi] = useState('')
  const [editingCollection, setEditingCollection] =
    useState<ReadingCollection | null>(null)
  const [parent, setParent] = useState('')
  const [batchTag, setBatchTag] = useState('')
  const activeCollections = collections.filter((c) => !c.deleted)
  const currentCollection = filter.startsWith('collection:')
    ? filter.slice(11)
    : ''
  const filtered = useMemo(
    () =>
      filterLiterature(documents, collections, filter, query, tags, unfiledOnly)
        .sort(
        (a, b) => {
          if (sort === 'opened') {
            const recent = (b.lastOpenedAt ?? '').localeCompare(a.lastOpenedAt ?? '')
            if (recent) return recent
          }
          if (sort === 'title') return a.name.localeCompare(b.name)
          if (sort === 'year')
            return (
              (b.bibliography?.year ?? '').localeCompare(
                a.bibliography?.year ?? '',
              ) || a.name.localeCompare(b.name)
            )
          if (sort === 'author')
            return (a.bibliography?.authors[0] ?? '').localeCompare(
              b.bibliography?.authors[0] ?? '',
            )
          return (
            (b.addedAt ?? '').localeCompare(a.addedAt ?? '') ||
            a.name.localeCompare(b.name)
          )
        },
      ),
    [documents, collections, filter, query, tags, sort, unfiledOnly],
  )
  const selectedDocs = filtered.filter((d) => selected.includes(d.id))
  const doc = selectedDocs.length === 1 ? selectedDocs[0] : null
  const availableTags = [
    ...new Set(
      documents
        .filter((d) => !d.trashedAt)
        .flatMap((d) => d.tags ?? [])
        .filter((t) => t.trim()),
    ),
  ].sort((a, b) => a.localeCompare(b))
  const names: Record<string, string> = {
    all: '全部文献',
    favorites: '收藏',
    trash: '回收站',
    duplicates: '可能重复',
  }
  const heading =
    names[filter] ??
    activeCollections.find((c) => c.id === currentCollection)?.name ??
    'Kivio Study'
  const run = async (label: string, action: () => Promise<void>) => {
    if (busyRef.current) return
    busyRef.current = true
    setBusy(label)
    setError('')
    setNotice('')
    try {
      await action()
    } catch (e) {
      setError(String(e))
    } finally {
      busyRef.current = false
      setBusy('')
    }
  }
  const change = (
    id: string,
    update: (d: ReadingDocument) => ReadingDocument,
  ) => {
    void updateReadingDocument(id, update).catch((e) => setError(String(e)))
  }
  const chooseFilter = (next: LibraryFilter) => {
    setFilter(next)
    setInspectorOpen(false)
    setSelected([])
    setTags([])
    setUnfiledOnly(false)
  }
  const addToCurrent = async (items: ReadingDocument[]) => {
    for (const item of items) {
      await updateReadingDocument(item.id, (d) => ({
        ...d,
        trashedAt: null,
        collectionIds: currentCollection
          ? [...new Set([...(d.collectionIds ?? []), currentCollection])]
          : d.collectionIds,
      }))
    }
    setSelected(items.map((d) => d.id))
    setQuery('')
    setTags([])
    setUnfiledOnly(false)
    if (!currentCollection) setFilter('all')
  }
  const importPdfs = () =>
    run('正在导入 PDF…', async () => {
      const paths = await open({
        multiple: true,
        filters: [{ name: 'PDF 文献', extensions: ['pdf'] }],
      })
      if (!paths) return
      const results: ReadingDocument[] = [],
        failures: string[] = []
      let duplicates = 0
      const known = new Set(documents.map((d) => d.id))
      for (const path of typeof paths === 'string' ? [paths] : paths) {
        try {
          const item = await importReadingDocument(path)
          if (known.has(item.id)) duplicates++
          known.add(item.id)
          results.push(item)
        } catch (e) {
          failures.push(`${path.split(/[\\/]/).pop()}：${String(e)}`)
        }
      }
      await addToCurrent(results)
      setNotice(
        `已处理 ${results.length} 份 PDF${duplicates ? `，其中 ${duplicates} 份复用已有文献` : ''}。可在资料栏补全作者、年份或通过 DOI 获取资料。`,
      )
      if (failures.length) setError(failures.join('\n'))
    })
  const importRis = () =>
    run('正在导入 RIS…', async () => {
      const path = await open({
        multiple: false,
        filters: [{ name: 'RIS 文献资料（UTF-8）', extensions: ['ris'] }],
      })
      if (typeof path !== 'string') return
      const items = await importReadingRis(path)
      await addToCurrent(items)
      setNotice(
        `已导入 ${items.length} 条文献资料。RIS 不携带 PDF，请在资料栏关联全文。`,
      )
    })
  const exportRis = () =>
    run('正在导出…', async () => {
      const items = selectedDocs.length ? selectedDocs : filtered
      const path = await save({
        defaultPath: 'Kivio文献.ris',
        filters: [{ name: 'RIS 文献资料', extensions: ['ris'] }],
      })
      if (!path) return
      await exportReadingRis(
        items.map((d) => d.id),
        path,
      )
      setNotice(
        `已导出 ${items.length} 条文献资料，可在 Zotero 中导入。PDF 和阅读笔记不包含在 RIS 中。`,
      )
    })
  const createItem = (lookup: boolean) =>
    run(lookup ? '正在获取 DOI 资料…' : '正在创建文献…', async () => {
      const result = lookup
        ? await readingApi.lookup(doi)
        : { name: title.trim(), bibliography: emptyBibliography }
      if (!result.name) throw new Error('请输入文献标题')
      const item = await createReadingItem(result.name, result.bibliography)
      await addToCurrent([item])
      setDialog(null)
      setNotice('文献已加入 Kivio Study。可以继续编辑资料或关联 PDF。')
    })
  const attachPdf = (item: ReadingDocument) =>
    run('正在关联 PDF…', async () => {
      const path = await open({
        multiple: false,
        filters: [{ name: 'PDF', extensions: ['pdf'] }],
      })
      if (typeof path === 'string') await attachReadingPdf(item.id, path)
    })
  const identify = (item: ReadingDocument) =>
    run('正在识别文献资料…', async () => {
      let identifier = item.bibliography?.doi ?? ''
      let pdfTitle = '',
        pdfAuthor = ''
      if (!identifier && item.hasPdf !== false) {
        const pdf = await pdfjs.getDocument({
          data: await readingApi.pdf(item.id),
          isEvalSupported: false,
        }).promise
        try {
          const metadata = await pdf.getMetadata()
          const info = metadata.info as Record<string, unknown>
          pdfTitle = typeof info.Title === 'string' ? info.Title : ''
          pdfAuthor = typeof info.Author === 'string' ? info.Author : ''
          let text = ''
          for (let p = 1; p <= Math.min(2, pdf.numPages); p++) {
            const page = await pdf.getPage(p)
            text +=
              (await page.getTextContent()).items
                .map((i) => ('str' in i ? i.str : ''))
                .join(' ') + '\n'
          }
          identifier =
            text
              .match(/10\.\d{4,9}\/[a-zA-Z0-9._;()/:+-]+/)?.[0]
              .replace(/[.,;]+$/, '') ?? ''
          if (!identifier) {
            const arxiv = text.match(/arXiv:\s*(\d{4}\.\d{4,5})/i)?.[1]
            if (arxiv) identifier = `10.48550/arXiv.${arxiv}`
          }
        } finally {
          await pdf.destroy()
        }
      }
      if (identifier) {
        const result = await readingApi.lookup(identifier)
        await updateReadingDocument(item.id, (d) => ({
          ...d,
          name: result.name,
          bibliography: mergeBibliography(
            { ...emptyBibliography, ...d.bibliography },
            result.bibliography,
          ),
        }))
        setNotice('已通过 DOI 更新资料，请核对标题、作者和年份。')
      } else if (pdfTitle || pdfAuthor) {
        await updateReadingDocument(item.id, (d) => ({
          ...d,
          name: pdfTitle || d.name,
          bibliography: {
            ...emptyBibliography,
            ...d.bibliography,
            authors: pdfAuthor ? [pdfAuthor] : (d.bibliography?.authors ?? []),
          },
        }))
        setNotice('已读取 PDF 内嵌标题和作者；未找到 DOI，其余资料请手动补齐。')
      } else
        throw new Error(
          '未找到可识别的 DOI 或内嵌资料。请填写 DOI 后再获取，或直接编辑资料。',
        )
    })
  const bulk = (
    update: (d: ReadingDocument) => ReadingDocument,
    message: string,
  ) =>
    run('正在保存…', async () => {
      await Promise.all(
        selectedDocs.map((d) => updateReadingDocument(d.id, update)),
      )
      setNotice(message)
    })
  const openCollectionDialog = (item?: ReadingCollection) => {
    setEditingCollection(item ?? null)
    setTitle(item?.name ?? '')
    setParent(item?.parentId ?? currentCollection)
    setDialog('collection')
  }
  const saveCollection = () =>
    run('正在保存文献集…', async () => {
      if (!title.trim()) throw new Error('请输入文献集名称')
      const item = {
        id: editingCollection?.id ?? crypto.randomUUID(),
        name: title.trim(),
        parentId: parent || null,
      }
      await updateReadingCollections((items) =>
        editingCollection
          ? items.map((c) => (c.id === item.id ? item : c))
          : [...items, item],
      )
      setDialog(null)
      chooseFilter(`collection:${item.id}`)
    })
  const deleteCollection = () =>
    run('正在移除文献集…', async () => {
      if (!editingCollection) return
      const ids = collectionDescendants(collections, editingCollection.id)
      await updateReadingCollections((items) =>
        items.map((c) => (ids.has(c.id) ? { ...c, deleted: true } : c)),
      )
      chooseFilter('all')
      setDialog(null)
      setNotice('文献集及子文献集已移除，文献、PDF、笔记和对话均保留。')
    })
  const collectionRows = (
    parentId: string | null,
    depth = 0,
  ): React.ReactNode =>
    activeCollections
      .filter(
        (c) =>
          c.parentId === parentId ||
          (!parentId &&
            c.parentId &&
            !activeCollections.some((p) => p.id === c.parentId)),
      )
      .map((c) => (
        <div key={c.id}>
          <div
            className="literature-collection-row"
            style={{ paddingLeft: Math.min(depth, 5) * 12 }}
          >
            <button
              className="literature-nav-row"
              aria-current={currentCollection === c.id ? 'page' : undefined}
              onClick={() => chooseFilter(`collection:${c.id}`)}
            >
              <Folder size={15} />
              <span>{c.name}</span>
              <small>
                {
                  documents.filter(
                    (d) => !d.trashedAt && d.collectionIds?.includes(c.id),
                  ).length
                }
              </small>
            </button>
            <IconButton
              label={`编辑文献集 ${c.name}`}
              size="xs"
              variant="ghost"
              onClick={() => openCollectionDialog(c)}
            >
              <Pencil size={12} />
            </IconButton>
          </div>
          {collectionRows(c.id, depth + 1)}
        </div>
      ))
  const b = { ...emptyBibliography, ...doc?.bibliography }
  const patchBibliography = (
    field: keyof typeof b,
    value: string | string[],
  ) => {
    if (doc)
      change(doc.id, (d) => ({
        ...d,
        bibliography: {
          ...emptyBibliography,
          ...d.bibliography,
          [field]: value,
        },
      }))
  }
  return (
    <div className="literature-library">
      <header className={`literature-header ${headerControls ? 'has-window-controls' : ''}`} data-tauri-drag-region>
        {headerControls}
        <h1>
          <StudyIcon size={17} />
          Kivio Study
        </h1>
        <div className="literature-search">
          <Search size={15} />
          <Input
            aria-label="搜索文献"
            placeholder="搜索文献、作者、标签或笔记"
            value={query}
            onChange={setQuery}
          />
        </div>
        <div className="literature-actions">
          <Button
            size="sm"
            variant="ghost"
            onClick={() => void importRis()}
            disabled={!!busy}
          >
            <Import size={15} />
            导入 RIS
          </Button>
          <Button
            size="sm"
            onClick={() => {
              setTitle('')
              setDoi('')
              setDialog('item')
            }}
            disabled={!!busy}
          >
            <Plus size={15} />
            新建文献
          </Button>
          <Button
            size="sm"
            variant="primary"
            onClick={() => void importPdfs()}
            disabled={!!busy || loading}
          >
            <Import size={15} />
            导入 PDF
          </Button>
        </div>
      </header>
      <ErrorNotice message={error} />
      <OperationNotice message={busy || notice} tone={busy ? 'info' : 'success'} duration={busy ? 0 : 3500} />
      <div ref={bodyRef} className="literature-body" style={{
        gridTemplateColumns: [layout.showNavigation ? `${layout.navigationWidth}px 6px` : '', 'minmax(0, 1fr)', layout.showInspector ? `6px ${layout.inspectorWidth}px` : ''].filter(Boolean).join(' '),
      } as CSSProperties}>
        {layout.showNavigation && <aside
          className="literature-navigation custom-scrollbar"
          aria-label="文献导航"
        >
          {(['all', 'favorites'] as const).map((key) => (
            <button
              key={key}
              className="literature-nav-row"
              aria-current={filter === key ? 'page' : undefined}
              onClick={() => chooseFilter(key)}
            >
              {key === 'favorites' ? <Star size={15} /> : <BookOpen size={15} />}
              <span>{names[key]}</span>
              <small>{filterLiterature(documents, collections, key, '', []).length}</small>
            </button>
          ))}
          <div className="literature-collections">
            <div className="literature-section-heading">
              <h2>文献集</h2>
              <IconButton
                label="新建文献集"
                variant="ghost"
                size="xs"
                onClick={() => openCollectionDialog()}
              >
                <FolderPlus size={15} />
              </IconButton>
            </div>
            {collectionRows(null)}
            {!activeCollections.length && (
              <p className="literature-muted">按主题、课程或研究任务收集文献。</p>
            )}
          </div>
          <button
            className="literature-nav-row literature-trash"
            aria-current={filter === 'trash' ? 'page' : undefined}
            onClick={() => chooseFilter('trash')}
          >
            <Trash2 size={15} />
            <span>回收站</span>
          </button>
        </aside>}
        {layout.showNavigation && divider('navigation')}
        <main className="literature-main">
          <div className="literature-list-toolbar">
            <div className="literature-list-heading">
              {!layout.showNavigation && <Select
                ariaLabel="切换文献集" size="sm" value={filter} onChange={(value) => { if (value === 'new-collection') openCollectionDialog(); else chooseFilter(value as LibraryFilter) }}
                triggerIcon={<Folder size={15} />}
                options={[
                  { value: 'all', label: '全部文献' }, { value: 'favorites', label: '收藏' },
                  ...activeCollections.map((c) => ({ value: `collection:${c.id}`, label: c.name })),
                  { value: 'trash', label: '回收站' },
                  { value: 'new-collection', label: '新建文献集…' },
                ]}
              />}
              <h2>
                <span title={heading}>{heading}</span>
                <small>{filtered.length}</small>
              </h2>
              <Select
                ariaLabel="文献排序"
                size="sm"
                value={sort}
                onChange={setSort}
                triggerIcon={<ArrowUpDown size={14} />}
                options={[
                  { value: 'opened', label: '最近打开' },
                  { value: 'added', label: '最近加入' },
                  { value: 'title', label: '标题' },
                  { value: 'author', label: '作者' },
                  { value: 'year', label: '年份' },
                ]}
              />
              {!layout.showInspector && selectedDocs.length > 0 && <Button size="sm" variant="ghost" onClick={() => setInspectorOpen(true)}>
                <FileText size={15} />资料
              </Button>}
              <Button
                size="sm"
                variant="ghost"
                aria-expanded={showFilters}
                aria-controls="literature-filters"
                onClick={() => setShowFilters((value) => !value)}
              >
                <SlidersHorizontal size={15} />
                筛选{tags.length + Number(unfiledOnly) > 0 ? ` ${tags.length + Number(unfiledOnly)}` : ''}
              </Button>
              <Select
                ariaLabel="文献管理"
                size="sm"
                value=""
                triggerIcon={<MoreHorizontal size={16} />}
                title="文献管理"
                disabled={!!busy}
                onChange={(value) => {
                  if (value === 'duplicates') chooseFilter('duplicates')
                  if (value === 'export') void exportRis()
                }}
                options={[
                  { value: 'duplicates', label: '查找可能重复的文献' },
                  ...(filtered.length ? [{ value: 'export', label: selectedDocs.length ? '导出选中文献 RIS' : '导出当前列表 RIS' }] : []),
                ]}
              />
            </div>
            {selectedDocs.length > 0 && (
              <div className="literature-bulk">
                <span>已选 {selectedDocs.length} 篇</span>
                <Select
                  ariaLabel="批量加入文献集"
                  size="sm"
                  value=""
                  disabled={!!busy || !activeCollections.length}
                  onChange={(id) =>
                    void bulk(
                      (d) => ({
                        ...d,
                        collectionIds: [
                          ...new Set([...(d.collectionIds ?? []), id]),
                        ],
                      }),
                      '已加入文献集',
                    )
                  }
                  options={[
                    { value: '', label: '加入文献集…' },
                    ...activeCollections.map((c) => ({
                      value: c.id,
                      label: c.name,
                    })),
                  ]}
                />
                {currentCollection && (
                  <Button
                    size="sm"
                    variant="ghost"
                    disabled={!!busy}
                    onClick={() =>
                      void bulk(
                        (d) => ({
                          ...d,
                          collectionIds: d.collectionIds?.filter(
                            (id) => id !== currentCollection,
                          ),
                        }),
                        '已移出文献集，文献仍在 Kivio Study 中',
                      )
                    }
                  >
                    移出文献集
                  </Button>
                )}
                <IconButton
                  label={filter === 'trash' ? '恢复选中文献' : '移入回收站'}
                  variant="ghost"
                  disabled={!!busy}
                  onClick={() =>
                    void bulk(
                      (d) => ({
                        ...d,
                        trashedAt:
                          filter === 'trash' ? null : new Date().toISOString(),
                      }),
                      filter === 'trash'
                        ? '已恢复文献'
                        : '已移入回收站，可随时恢复',
                    )
                  }
                >
                  {filter === 'trash' ? (
                    <RotateCcw size={15} />
                  ) : (
                    <Trash2 size={15} />
                  )}
                </IconButton>
              </div>
            )}
          </div>
          {showFilters && (
            <div className="literature-filters" id="literature-filters" aria-label="文献筛选">
              <div className="literature-filter-heading">
                <label className="literature-membership">
                  <input type="checkbox" checked={unfiledOnly} onChange={(e) => setUnfiledOnly(e.target.checked)} />
                  尚未加入文献集
                </label>
                {(tags.length > 0 || unfiledOnly) && (
                  <Button size="sm" variant="ghost" onClick={() => { setTags([]); setUnfiledOnly(false) }}>清除筛选</Button>
                )}
              </div>
              {availableTags.length > 0 && (
                <div className="literature-tag-list" aria-label="按标签筛选">
                  {availableTags.map((tag) => (
                    <Button key={tag} size="sm" variant={tags.includes(tag) ? 'default' : 'ghost'} aria-pressed={tags.includes(tag)}
                      onClick={() => setTags((old) => old.includes(tag) ? old.filter((t) => t !== tag) : [...old, tag])}>
                      <Tag size={12} />{tag}
                    </Button>
                  ))}
                </div>
              )}
            </div>
          )}
          <div className="literature-table-wrap custom-scrollbar">
            <table className="literature-table">
              <thead>
                <tr>
                  <th>
                    <input
                      type="checkbox"
                      aria-label="全选当前列表"
                      checked={
                        filtered.length > 0 &&
                        selectedDocs.length === filtered.length
                      }
                      onChange={(e) =>
                        setSelected(
                          e.target.checked ? filtered.map((d) => d.id) : [],
                        )
                      }
                    />
                  </th>
                  <th>标题 / 作者</th>
                  <th>年份</th>
                </tr>
              </thead>
              <tbody>
                {filtered.map((item) => (
                  <tr
                    key={item.id}
                    aria-selected={selected.includes(item.id)}
                    onClick={() => setSelected([item.id])}
                    onDoubleClick={() => {
                      if (!item.trashedAt && item.hasPdf !== false) onRead(item)
                    }}
                  >
                    <td onClick={(e) => e.stopPropagation()}>
                      <input
                        type="checkbox"
                        aria-label={`选择 ${item.name}`}
                        checked={selected.includes(item.id)}
                        onChange={(e) =>
                          setSelected((old) =>
                            e.target.checked
                              ? [...old, item.id]
                              : old.filter((id) => id !== item.id),
                          )
                        }
                      />
                    </td>
                    <td>
                      <div className="literature-item">
                        {item.hasPdf !== false && (
                          <PdfThumbnail
                            id={item.id}
                            fingerprint={item.fingerprint}
                            name={item.name}
                          />
                        )}
                        <div className="literature-item-copy">
                          <button
                            className="literature-title"
                            onClick={() => setSelected([item.id])}
                          >
                            {item.hasPdf === false && <BookOpen size={15} />}
                            <span>{item.name}</span>
                            {item.favorite && <Star size={12} />}
                          </button>
                          <p>
                            {item.bibliography?.authors.join('; ') ||
                              '作者待补全'}
                            {item.bibliography?.publication
                              ? ` · ${item.bibliography.publication}`
                              : ''}
                          </p>
                          {!!item.tags?.length && (
                            <small>{item.tags.join(' · ')}</small>
                          )}
                        </div>
                      </div>
                    </td>
                    <td>{item.bibliography?.year || '—'}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            {!filtered.length && (
              <div className="literature-empty">
                <StudyIcon size={30} />
                <h3>
                  {loading
                    ? '正在加载 Kivio Study…'
                    : query || tags.length || unfiledOnly
                      ? '没有匹配的文献'
                      : filter === 'trash'
                        ? '回收站为空'
                        : '这里还没有文献'}
                </h3>
                <p>
                  {query || tags.length || unfiledOnly
                    ? '试试其他关键词，或清除筛选。'
                    : filter === 'trash'
                      ? '移入回收站的文献可在这里恢复。'
                      : '导入 PDF、RIS，或用 DOI 新建文献，再按主题整理。'}
                </p>
                {(query || tags.length || unfiledOnly) && (
                  <Button
                    size="sm"
                    onClick={() => {
                      setQuery('')
                      setTags([])
                      setUnfiledOnly(false)
                    }}
                  >
                    清除筛选
                  </Button>
                )}
              </div>
            )}
          </div>
          <footer className="literature-list-footer">
            {filter === 'duplicates'
              ? '按 DOI、文件指纹或标题与年份查找；请核对后整理。'
              : `${documents.filter((d) => !d.trashedAt).length} 篇文献 · ${saving ? '正在保存…' : '保存在本机'} · 双击打开全文`}
          </footer>
        </main>
        {layout.showInspector && divider('inspector')}
        {(layout.showInspector || inspectorOpen) && <aside
          className={`literature-inspector custom-scrollbar ${!layout.showInspector ? 'is-overlay' : ''}`}
          aria-label="文献资料"
          onKeyDown={(event) => { if (event.key === 'Escape') setInspectorOpen(false) }}
        >
          {!layout.showInspector && <div className="literature-overlay-close"><IconButton autoFocus label="关闭资料栏" variant="ghost" onClick={() => setInspectorOpen(false)}><X size={16} /></IconButton></div>}
          <fieldset disabled={!!busy} className="literature-inspector-fields">
            {doc ? (
              <>
                <div className="literature-detail-heading">
                  <h2>文献资料</h2>
                  <IconButton
                    label={doc.favorite ? '取消收藏' : '收藏文献'}
                    variant="ghost"
                    onClick={() =>
                      change(doc.id, (d) => ({ ...d, favorite: !d.favorite }))
                    }
                  >
                    <Star
                      size={16}
                      fill={doc.favorite ? 'currentColor' : 'none'}
                    />
                  </IconButton>
                </div>
                {doc.trashedAt ? (
                  <Button
                    onClick={() =>
                      void bulk(
                        (d) => ({ ...d, trashedAt: null }),
                        '已恢复文献',
                      )
                    }
                  >
                    <RotateCcw size={15} />
                    恢复到 Kivio Study
                  </Button>
                ) : doc.hasPdf !== false ? (
                  <Button variant="primary" onClick={() => onRead(doc)}>
                    <BookOpen size={15} />
                    阅读与对话
                    <ChevronRight size={14} />
                  </Button>
                ) : (
                  <Button onClick={() => void attachPdf(doc)} disabled={!!busy}>
                    <Plus size={15} />
                    关联 PDF 全文
                  </Button>
                )}
                <label className="literature-field">
                  标题
                  <Input
                    aria-label="文献标题"
                    value={doc.name}
                    onChange={(value) =>
                      change(doc.id, (d) => ({ ...d, name: value }))
                    }
                  />
                </label>
                <label className="literature-field">
                  类型
                  <Select
                    ariaLabel="文献类型"
                    value={b.itemType || 'document'}
                    onChange={(value) => patchBibliography('itemType', value)}
                    options={itemTypes}
                  />
                </label>
                <label className="literature-field">
                  作者
                  <TextArea
                    aria-label="文献作者"
                    placeholder="每行一位作者"
                    value={b.authors.join('\n')}
                    onChange={(value) =>
                      patchBibliography('authors', value.split('\n'))
                    }
                    rows={2}
                  />
                </label>
                <label className="literature-field">
                  年份
                  <Input aria-label="发表年份" value={b.year} onChange={(value) => patchBibliography('year', value)} />
                </label>
                <label className="literature-field">
                  期刊 / 出版物
                  <Input
                    aria-label="期刊或出版物"
                    value={b.publication}
                    onChange={(value) =>
                      patchBibliography('publication', value)
                    }
                  />
                </label>
                <label className="literature-field">
                  DOI
                  <Input
                    aria-label="文献 DOI"
                    value={b.doi}
                    placeholder="10.…"
                    onChange={(value) => patchBibliography('doi', value)}
                  />
                </label>
                <Button
                  size="sm"
                  disabled={!!busy || (!b.doi && doc.hasPdf === false)}
                  onClick={() => void identify(doc)}
                >
                  {b.doi ? '通过 DOI 更新资料' : '识别 PDF 资料'}
                </Button>
                <p className="literature-muted">
                  DOI 查询使用 Crossref / DataCite；请核对识别结果。
                </p>
                <label className="literature-field">
                  网址
                  <Input
                    aria-label="文献网址"
                    value={b.url}
                    onChange={(value) => patchBibliography('url', value)}
                  />
                </label>
                <label className="literature-field">
                  摘要
                  <TextArea
                    aria-label="文献摘要"
                    value={b.abstractText}
                    onChange={(value) =>
                      patchBibliography('abstractText', value)
                    }
                    rows={4}
                  />
                </label>
                <label className="literature-field">
                  标签
                  <TextArea
                    aria-label="文献标签"
                    placeholder="每行一个标签"
                    value={(doc.tags ?? []).join('\n')}
                    onChange={(value) =>
                      change(doc.id, (d) => ({ ...d, tags: value.split('\n') }))
                    }
                    rows={2}
                  />
                </label>
                <div className="literature-section-heading">
                  <h2>所属文献集</h2>
                </div>
                {activeCollections.map((c) => (
                  <label key={c.id} className="literature-membership">
                    <input
                      type="checkbox"
                      checked={doc.collectionIds?.includes(c.id) ?? false}
                      onChange={(e) =>
                        change(doc.id, (d) => ({
                          ...d,
                          collectionIds: e.target.checked
                            ? [...new Set([...(d.collectionIds ?? []), c.id])]
                            : d.collectionIds?.filter((id) => id !== c.id),
                        }))
                      }
                    />
                    {c.name}
                  </label>
                ))}
                {!activeCollections.length && (
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => openCollectionDialog()}
                  >
                    创建第一个文献集
                  </Button>
                )}
                <div className="literature-detail-summary">
                  <span>{doc.notes.length} 条笔记</span>
                  <span>{doc.conversationIds.length} 个对话</span>
                  <span>
                    {doc.hasPdf === false
                      ? '尚未关联 PDF'
                      : `PDF · 上次读到第 ${doc.page} 页`}
                  </span>
                </div>
              </>
            ) : selectedDocs.length > 1 ? (
              <>
                <h2>批量整理</h2>
                <p>已选择 {selectedDocs.length} 篇文献</p>
                <label className="literature-field">
                  添加标签
                  <Input
                    value={batchTag}
                    onChange={setBatchTag}
                    aria-label="批量标签"
                    placeholder="输入一个标签"
                  />
                </label>
                <Button
                  size="sm"
                  disabled={!batchTag.trim() || !!busy}
                  onClick={() =>
                    void bulk(
                      (d) => ({
                        ...d,
                        tags: [
                          ...new Set([...(d.tags ?? []), batchTag.trim()]),
                        ],
                      }),
                      '已添加标签',
                    )
                  }
                >
                  添加到所选文献
                </Button>
                <Button
                  size="sm"
                  onClick={() =>
                    void bulk(
                      (d) => ({ ...d, favorite: true }),
                      '已收藏所选文献',
                    )
                  }
                  disabled={!!busy}
                >
                  收藏所选文献
                </Button>
                <p className="literature-muted">
                  文献集和回收站操作位于列表上方。批量操作不会覆盖原文、笔记和对话。
                </p>
              </>
            ) : (
              <div className="literature-empty">
                <FileText size={28} />
                <h3>选择一篇文献</h3>
                <p>在这里整理书目信息、标签与文献集，或打开全文继续阅读。</p>
              </div>
            )}
          </fieldset>
        </aside>}
      </div>
      {dialog === 'item' && (
        <LibraryDialog
          busy={!!busy}
          title="新建文献"
          onClose={() => {
            if (!busy) setDialog(null)
          }}
        >
          <form
            onSubmit={(e) => {
              e.preventDefault()
              void createItem(true)
            }}
          >
            <label className="literature-field">
              通过 DOI 获取资料
              <Input
                autoFocus
                aria-label="新文献 DOI"
                value={doi}
                onChange={setDoi}
                placeholder="粘贴 DOI 或 https://doi.org/…"
              />
            </label>
            <Button
              type="submit"
              variant="primary"
              disabled={!doi.trim() || !!busy}
            >
              获取并加入 Kivio Study
            </Button>
          </form>
          <div className="literature-dialog-divider">也可以手动录入</div>
          <form
            onSubmit={(e) => {
              e.preventDefault()
              void createItem(false)
            }}
          >
            <label className="literature-field">
              文献标题
              <Input
                aria-label="新文献标题"
                value={title}
                onChange={setTitle}
                placeholder="文章、书籍或其他资料的标题"
              />
            </label>
            <Button type="submit" disabled={!title.trim() || !!busy}>
              创建条目
            </Button>
          </form>
          {(error || busy) && <p role="alert">{error || busy}</p>}
          <p className="literature-muted">
            先保存文献资料，之后可关联 PDF。相同 DOI 会复用已有条目。
          </p>
        </LibraryDialog>
      )}
      {dialog === 'collection' && (
        <LibraryDialog
          busy={!!busy}
          title={editingCollection ? '编辑文献集' : '新建文献集'}
          onClose={() => {
            if (!busy) setDialog(null)
          }}
        >
          <form
            onSubmit={(e) => {
              e.preventDefault()
              void saveCollection()
            }}
          >
            <label className="literature-field">
              文献集名称
              <Input
                autoFocus
                aria-label="文献集名称"
                value={title}
                onChange={setTitle}
              />
            </label>
            <label className="literature-field">
              上级文献集
              <Select
                ariaLabel="上级文献集"
                value={parent}
                onChange={setParent}
                options={[
                  { value: '', label: 'Kivio Study（顶层）' },
                  ...activeCollections
                    .filter(
                      (c) =>
                        !editingCollection ||
                        !collectionDescendants(
                          collections,
                          editingCollection.id,
                        ).has(c.id),
                    )
                    .map((c) => ({ value: c.id, label: c.name })),
                ]}
              />
            </label>
            <div className="literature-actions">
              <Button
                variant="primary"
                type="submit"
                disabled={!!busy || !title.trim()}
              >
                保存文献集
              </Button>
              {editingCollection && (
                <Button
                  variant="danger"
                  disabled={!!busy}
                  onClick={() => void deleteCollection()}
                >
                  移除文献集
                </Button>
              )}
            </div>
          </form>
          {editingCollection && (
            <p className="literature-muted">
              移除文献集及其子文献集，不会删除其中的文献。
            </p>
          )}
          {error && <p role="alert">{error}</p>}
        </LibraryDialog>
      )}
    </div>
  )
}
