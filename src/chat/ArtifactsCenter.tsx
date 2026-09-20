import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { save } from '@tauri-apps/plugin-dialog'
import { Code2, Download, ExternalLink, File, FileText, FolderOpen, Image, Layers, LayoutGrid, List, MessageSquare, Music2, Presentation, RefreshCw, Search, Table2, X } from 'lucide-react'
import { api, type ArtifactLibraryItem, type ArtifactLibraryPage } from '../api/tauri'
import { Button, IconButton } from '../components/Button'
import { Input, Select } from '../settings/public/controls'
import { useLang } from '../components/i18n'
import { artifactDataUrl, artifactMimeType } from './artifacts'
import './ArtifactsCenter.css'

const MarkdownPreview = lazy(() => import('./ChatMarkdown').then((module) => ({ default: module.ChatMarkdown })))
type Kind = 'all' | 'image' | 'document' | 'spreadsheet' | 'presentation' | 'code' | 'media' | 'other'
const kindIcons = { all: Layers, image: Image, document: FileText, spreadsheet: Table2, presentation: Presentation, code: Code2, media: Music2, other: File }
const kindNames: Record<Kind, [string, string]> = { all: ['全部', 'All'], document: ['文档', 'Documents'], image: ['图片', 'Images'], spreadsheet: ['表格', 'Spreadsheets'], presentation: ['演示稿', 'Presentations'], code: ['代码与网页', 'Code & web'], media: ['音视频', 'Media'], other: ['其他', 'Other'] }
const kinds: Kind[] = ['all', 'document', 'image', 'spreadsheet', 'presentation', 'code', 'media', 'other']

function extension(item: ArtifactLibraryItem) { return item.artifact.name.split('.').pop()?.toUpperCase() || 'FILE' }
function sizeLabel(item: ArtifactLibraryItem) {
  const bytes = item.artifact.sizeBytes ?? item.artifact.size_bytes
  return bytes == null ? '' : bytes < 1024 ? `${bytes} B` : bytes < 1024 * 1024 ? `${Math.round(bytes / 1024)} KB` : `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}
function displayTitle(item: ArtifactLibraryItem) {
  const stem = item.artifact.name.replace(/\.[^.]+$/, '')
  return /^generated[-_ ]?(image|file|document)[-_ \d]*$/i.test(stem) && item.title && item.title !== item.artifact.name && !/^(新对话|新聊天|New chat|Untitled)$/i.test(item.title) ? item.title : stem
}
function isText(item: ArtifactLibraryItem) {
  return artifactMimeType(item.artifact).startsWith('text/') || /\.(md|markdown|txt|csv|tsv|log)$/i.test(item.artifact.name) || kindOf(item) === 'code'
}
function decodeText(data: string) {
  try { return new TextDecoder().decode(Uint8Array.from(atob(data.slice(data.indexOf(',') + 1)), (c) => c.charCodeAt(0))) } catch { return '' }
}

function kindOf(item: ArtifactLibraryItem): Kind {
  const mime = artifactMimeType(item.artifact)
  if (mime.startsWith('image/') || /\.(png|jpe?g|gif|webp|svg|avif|bmp|tiff?)$/i.test(item.artifact.name)) return 'image'
  if (/\.(xlsx?|xlsm|csv|tsv|ods)$/i.test(item.artifact.name) || /spreadsheet|excel/.test(mime)) return 'spreadsheet'
  if (/\.(pptx?|odp|key)$/i.test(item.artifact.name) || /presentation|powerpoint/.test(mime)) return 'presentation'
  if (/^(audio|video)\//.test(mime) || /\.(mp[34]|wav|m4a|ogg|flac|webm|mov)$/i.test(item.artifact.name)) return 'media'
  if (/\.(html?|jsx?|tsx?|py|rs|css|json|ya?ml|sh|sql)$/i.test(item.artifact.name)) return 'code'
  if (/\.(pdf|docx?|md|markdown|txt|odt|rtf)$/i.test(item.artifact.name) || mime.startsWith('text/') || /wordprocessing|msword/.test(mime)) return 'document'
  return 'other'
}

export function ArtifactsCenter({ onOpenConversation }: { onOpenConversation: (id: string) => void }) {
  const zh = useLang() === 'zh'
  const [page, setPage] = useState<ArtifactLibraryPage>({ items: [], warnings: 0 })
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')
  const [refresh, setRefresh] = useState(0)
  const [query, setQuery] = useState('')
  const [kind, setKind] = useState<Kind>('all')
  const [list, setList] = useState(false)
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [sort, setSort] = useState('recent')
  // Immutable IDs let card excerpts and the reader share a read during this visit.
  const previews = useRef(new Map<string, Promise<string | null>>())
  const loadPreview = useCallback((id: string) => {
    let pending = previews.current.get(id)
    if (!pending) {
      pending = api.chatArtifactAction(id, 'preview').then((value) => {
        // Retain small text covers, not every full-resolution image opened here.
        if (value && value.length > 512 * 1024 && previews.current.get(id) === pending) previews.current.delete(id)
        return value
      })
      previews.current.set(id, pending)
      void pending.catch(() => { if (previews.current.get(id) === pending) previews.current.delete(id) })
    }
    return pending
  }, [])

  useEffect(() => {
    let active = true
    setLoading(true)
    setError('')
    api.chatArtifactsList().then((result) => { if (active) setPage(result) })
      .catch((e) => { if (active) setError(String(e)) })
      .finally(() => { if (active) setLoading(false) })
    return () => { active = false }
  }, [refresh])

  const groups = useMemo(() => {
    const result = new Map<string, ArtifactLibraryItem[]>()
    for (const item of page.items) {
      const versions = result.get(item.workId) ?? []
      versions.push(item)
      result.set(item.workId, versions)
    }
    for (const versions of result.values()) versions.sort((a, b) => b.createdAt - a.createdAt || b.id.localeCompare(a.id))
    return [...result.values()].sort((a, b) => b[0].createdAt - a[0].createdAt)
  }, [page.items])
  const filtered = groups.filter(([item]) => (kind === 'all' || kindOf(item) === kind)
    && `${item.title} ${item.artifact.name}`.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase()))
    .sort((a, b) => sort === 'name' ? displayTitle(a[0]).localeCompare(displayTitle(b[0]), zh ? 'zh-CN' : 'en') : sort === 'oldest' ? a[0].createdAt - b[0].createdAt : b[0].createdAt - a[0].createdAt)
  const selected = page.items.find((item) => item.id === selectedId)
  const versions = selected ? groups.find((items) => items[0].workId === selected.workId) ?? [] : []

  return <section className="kv-works custom-scrollbar" aria-label={zh ? '作品' : 'Works'}>
    <header className="kv-works-header">
      <div><h1>{zh ? '作品' : 'Works'} <span>{groups.length}</span></h1>
        <p>{zh ? '把灵感留下，把作品接着做好。' : 'Keep what you create. Pick up where you left off.'}</p></div>
      <div className="kv-works-search"><Search size={16} /><Input aria-label={zh ? '搜索作品' : 'Search works'} placeholder={zh ? '搜索作品或聊天…' : 'Search works or chats…'} value={query} onChange={setQuery} />{query && <IconButton label={zh ? '清除搜索' : 'Clear search'} onClick={() => setQuery('')}><X size={14} /></IconButton>}</div>
    </header>
    <nav className="kv-works-categories custom-scrollbar" aria-label={zh ? '作品类型' : 'Work types'}>{kinds.map((value) => {
      const Icon = kindIcons[value]
      const count = value === 'all' ? groups.length : groups.filter(([item]) => kindOf(item) === value).length
      return <Button key={value} variant={kind === value ? 'default' : 'ghost'} aria-pressed={kind === value} onClick={() => setKind(value)}><Icon size={16} /><span>{kindNames[value][zh ? 0 : 1]}</span><small>{count}</small></Button>
    })}</nav>
    <div className="kv-works-toolbar">
      <span className="kv-works-count">{zh ? `${filtered.length} 件作品` : `${filtered.length} works`}</span>
      <Select className="w-32" ariaLabel={zh ? '作品排序' : 'Sort works'} value={sort} onChange={setSort} options={[{ value: 'recent', label: zh ? '最近创建' : 'Newest first' }, { value: 'oldest', label: zh ? '最早创建' : 'Oldest first' }, { value: 'name', label: zh ? '按名称' : 'Name' }]} />
      <div className="kv-works-layout"><Button size="sm" variant={list ? 'ghost' : 'default'} aria-label={zh ? '网格视图' : 'Grid view'} title={zh ? '网格视图' : 'Grid view'} aria-pressed={!list} onClick={() => setList(false)}><LayoutGrid size={16} /></Button><Button size="sm" variant={list ? 'default' : 'ghost'} aria-label={zh ? '列表视图' : 'List view'} title={zh ? '列表视图' : 'List view'} aria-pressed={list} onClick={() => setList(true)}><List size={16} /></Button></div>
      <IconButton label={zh ? '刷新作品' : 'Refresh works'} disabled={loading} onClick={() => { previews.current.clear(); setRefresh((v) => v + 1) }}><RefreshCw size={15} className={loading ? 'animate-spin' : ''} /></IconButton>
    </div>
    {error && <div role="alert" className="kv-works-notice">{error}<Button size="sm" onClick={() => setRefresh((v) => v + 1)}>{zh ? '重试' : 'Retry'}</Button></div>}
    {loading && !page.items.length ? <div className="kv-works-empty" role="status">{zh ? '正在整理作品…' : 'Loading works…'}</div>
        : !filtered.length ? <div className="kv-works-empty"><Layers size={36} strokeWidth={1.2} /><h2>{query || kind !== 'all' ? (zh ? '没有找到匹配的作品' : 'No matching works') : (zh ? '你的创作，从这里开始' : 'Your creations belong here')}</h2><p>{zh ? '聊天中交付的文档、图片、表格等会自动收录。' : 'Documents, images, spreadsheets and other files delivered in chats appear here.'}</p></div>
        : <div className={`kv-works-grid ${list ? 'is-list' : ''}`} aria-busy={loading}>
          {filtered.map((items) => <WorkCard key={items[0].id} item={items[0]} versions={items.length} zh={zh} list={list} loadPreview={loadPreview} onOpen={() => setSelectedId(items[0].id)} />)}
        </div>}
    {page.warnings > 0 && <details className="kv-works-import-note"><summary>{zh ? `${page.warnings} 项历史内容未导入` : `${page.warnings} historical items not imported`}</summary><p>{zh ? '部分历史附件暂时无法读取。原聊天记录保留，你可以回到聊天查看，恢复文件后刷新重试。' : 'Some historical attachments could not be read. Their chats are unchanged. Restore the files and refresh to retry.'}</p></details>}
    {selected && <WorkPreview key={selected.id} item={selected} versions={versions} zh={zh} loadPreview={loadPreview} onVersion={setSelectedId} onClose={() => setSelectedId(null)} onSource={() => onOpenConversation(selected.conversationId)} />}
  </section>
}

function WorkCard({ item, versions, zh, list, loadPreview, onOpen }: { item: ArtifactLibraryItem; versions: number; zh: boolean; list: boolean; loadPreview: (id: string) => Promise<string | null>; onOpen: () => void }) {
  const root = useRef<HTMLButtonElement>(null)
  const [excerpt, setExcerpt] = useState('')
  const kind = kindOf(item)
  const Icon = kindIcons[kind]
  const thumb = artifactDataUrl(item.artifact)
  const title = displayTitle(item)
  const sourceIsTitle = title === item.title
  const [imageFailed, setImageFailed] = useState(false)
  useEffect(() => {
    if (list || !item.available || !isText(item) || (item.artifact.sizeBytes ?? item.artifact.size_bytes ?? 0) > 256 * 1024) return
    let active = true
    const read = () => { void loadPreview(item.id).then((data) => {
      if (active && data) setExcerpt(decodeText(data).slice(0, 1600).replace(/^#{1,6}\s+/gm, '').replace(/\*\*|__|```[^\n]*/g, '').trim())
    }).catch(() => { /* File identity remains useful when a cover cannot be read. */ }) }
    const observer = typeof IntersectionObserver === 'undefined' ? null : new IntersectionObserver((entries) => {
      if (entries.some((entry) => entry.isIntersecting)) { observer?.disconnect(); read() }
    }, { rootMargin: '100px' })
    if (observer && root.current) observer.observe(root.current)
    else read()
    return () => { active = false; observer?.disconnect() }
  }, [item, list, loadPreview])
  return <button ref={root} type="button" className={`kv-work-card kind-${kind}`} aria-label={item.artifact.name} onClick={onOpen}>
    <div className="kv-work-cover">
      {kind === 'image' && thumb && !imageFailed ? <img src={thumb} alt="" loading="lazy" onError={() => setImageFailed(true)} /> : <div className={`kv-work-file-cover ${excerpt ? 'has-excerpt' : ''}`}>
        <div className="kv-work-file-format"><Icon size={22} strokeWidth={1.5} /><span>{extension(item)}</span></div>
        <strong>{displayTitle(item)}</strong>
        {excerpt ? <p className={kind === 'code' ? 'is-code' : ''}>{excerpt}</p> : <span className="kv-work-file-caption">{kindNames[kind][zh ? 0 : 1]}{sizeLabel(item) && ` · ${sizeLabel(item)}`}</span>}
      </div>}
      {versions > 1 && <span className="kv-work-versions"><Layers size={12} />{versions} {zh ? '个版本' : 'versions'}</span>}
    </div>
    <div className="kv-work-info"><div className="kv-work-title"><Icon size={15} /><h2 title={item.artifact.name}>{title}</h2><span className="kv-work-format">{extension(item)}</span></div><p title={item.title}>{sourceIsTitle ? <File size={12} /> : <MessageSquare size={12} />}<span>{sourceIsTitle ? item.artifact.name : item.title || (zh ? '来源聊天' : 'Source chat')}</span></p></div>
    <div className="kv-work-meta"><span>{new Date(item.createdAt * 1000).toLocaleDateString(zh ? 'zh-CN' : 'en-US', { month: 'short', day: 'numeric', year: 'numeric' })}</span><span>{item.available ? sizeLabel(item) : (zh ? '文件缺失' : 'File missing')}</span></div>
  </button>
}

function WorkPreview({ item, versions, zh, loadPreview, onVersion, onClose, onSource }: {
  item: ArtifactLibraryItem; versions: ArtifactLibraryItem[]; zh: boolean
  loadPreview: (id: string) => Promise<string | null>
  onVersion: (id: string) => void; onClose: () => void; onSource: () => void
}) {
  const dialog = useRef<HTMLDialogElement>(null)
  const [data, setData] = useState('')
  const [loading, setLoading] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const alive = useRef(true)
  const mime = artifactMimeType(item.artifact)
  const previewable = kindOf(item) === 'image' || isText(item) || mime === 'application/pdf'
  const Icon = kindIcons[kindOf(item)]

  useEffect(() => {
    alive.current = true
    dialog.current?.showModal()
    return () => { alive.current = false }
  }, [])
  useEffect(() => {
    let active = true
    if (item.available && previewable) {
      setLoading(true)
      loadPreview(item.id).then((value) => { if (active) setData(value ?? '') })
        .catch((e) => { if (active) setError(String(e)) })
        .finally(() => { if (active) setLoading(false) })
    }
    return () => { active = false }
  }, [item.id, item.available, previewable, loadPreview])

  async function action(operation: 'open' | 'reveal' | 'export') {
    setBusy(true); setError(''); setNotice('')
    try {
      const destination = operation === 'export' ? await save({ defaultPath: item.artifact.name, title: zh ? '作品另存为' : 'Save work as' }) : undefined
      if (operation === 'export' && !destination) return
      await api.chatArtifactAction(item.id, operation, destination ?? undefined)
      if (alive.current && operation === 'export') setNotice(zh ? '已保存' : 'Saved')
    } catch (e) { if (alive.current) setError(String(e)) }
    finally { if (alive.current) setBusy(false) }
  }
  const text = data && isText(item) ? decodeText(data) : ''
  return <dialog ref={dialog} className="kv-modal kv-work-dialog custom-scrollbar" aria-label={item.artifact.name} onCancel={onClose} onClick={(e) => { if (e.target === e.currentTarget) onClose() }}>
    <div className="kv-work-dialog-inner">
      <header><span className={`kv-work-dialog-icon kind-${kindOf(item)}`}><Icon size={22} /></span><div className="kv-work-dialog-title"><h2>{displayTitle(item)}</h2><p>{item.artifact.name} · {sizeLabel(item) || extension(item)}</p></div><IconButton label={zh ? '关闭预览' : 'Close preview'} onClick={onClose}><X size={18} /></IconButton></header>
      <div className="kv-work-preview custom-scrollbar">
        {!item.available ? <p>{zh ? '原文件已丢失。你仍可返回来源聊天查看记录。' : 'The original file is missing. Its source chat is still available if it has not been deleted.'}</p>
          : loading ? <span role="status">{zh ? '正在加载原文件…' : 'Loading original…'}</span>
            : data && kindOf(item) === 'image' ? <img src={data} alt={item.artifact.name} />
              : data && mime === 'application/pdf' ? <iframe title={item.artifact.name} src={data} sandbox="" />
                : text ? /\.(md|markdown)$/i.test(item.artifact.name) ? <article className="kv-work-document"><Suspense fallback={<pre>{text}</pre>}><MarkdownPreview content={text} /></Suspense></article> : <pre className="kv-work-text">{text}</pre>
                  : <div className={`kv-work-file-detail kind-${kindOf(item)}`}><Icon size={48} strokeWidth={1.2} /><span>{extension(item)}</span><h3>{displayTitle(item)}</h3><p>{zh ? '用默认应用打开，查看完整内容与排版。' : 'Open in the default app to see the full content and layout.'}</p><Button onClick={() => void action('open')} disabled={busy}><ExternalLink size={15} />{zh ? '打开文件' : 'Open file'}</Button></div>}
      </div>
      <footer>
        {versions.length > 1 && <Select className="w-36" ariaLabel={zh ? '历史版本' : 'Version history'} value={item.id} onChange={onVersion} options={versions.map((v, i) => ({ value: v.id, label: `${zh ? '版本' : 'Version'} ${versions.length - i}${i === 0 ? (zh ? ' · 最新' : ' · Latest') : ''}` }))} />}
        <Button size="sm" disabled={!item.sourceAvailable} title={item.sourceAvailable ? item.title : (zh ? '来源聊天已删除' : 'Source chat deleted')} onClick={onSource}><MessageSquare size={14} />{zh ? '回到聊天修改' : 'Continue in chat'}</Button>
        <div className="grow" />
        <IconButton label={zh ? '在文件夹中显示' : 'Show in folder'} disabled={!item.available || busy} onClick={() => void action('reveal')}><FolderOpen size={16} /></IconButton>
        <Button size="sm" disabled={!item.available || busy} title={zh ? '用默认应用打开副本，保留作品原版本' : 'Open a copy in the default app, preserving this version'} onClick={() => void action('open')}><ExternalLink size={14} />{zh ? '打开副本' : 'Open copy'}</Button>
        <Button size="sm" variant="primary" disabled={!item.available || busy} onClick={() => void action('export')}><Download size={14} />{zh ? '另存为' : 'Save as'}</Button>
      </footer>
      {error && <p role="alert" className="kv-works-notice">{error}</p>}
      {notice && <p role="status" className="kv-works-notice">{notice}</p>}
    </div>
  </dialog>
}
