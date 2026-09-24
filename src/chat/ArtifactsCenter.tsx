import { useEffect, useMemo, useRef, useState, type MouseEvent } from 'react'
import { open, save } from '@tauri-apps/plugin-dialog'
import { CheckSquare, Code2, Download, ExternalLink, File, FileText, FolderOpen, Image, Layers, LayoutGrid, List, MessageSquare, MoreHorizontal, Music2, Pencil, Presentation, RefreshCw, Search, Table2, Trash2, X } from 'lucide-react'
import { api, type ArtifactLibraryItem, type ArtifactLibraryPage } from '../api/tauri'
import { Button, IconButton } from '../components/Button'
import { Input, Select } from '../settings/public/controls'
import { useLang } from '../components/i18n'
import { WorksIcon } from '../settings/public/icons'
import { artifactDataUrl, artifactMimeType } from './artifacts'
import { DockContextMenu, type DockMenuAnchor } from './dock/DockContextMenu'
import './ArtifactsCenter.css'

let cachedPage: ArtifactLibraryPage | null = null
// Allows an explicit cold read in regression tests.
// eslint-disable-next-line react-refresh/only-export-components
export function clearArtifactsPageCache() { cachedPage = null }
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
  const [page, setPage] = useState<ArtifactLibraryPage>(() => cachedPage ?? { items: [], warnings: 0 })
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')
  const [refresh, setRefresh] = useState(0)
  const [query, setQuery] = useState('')
  const [kind, setKind] = useState<Kind>('all')
  const [list, setList] = useState(false)
  const [sort, setSort] = useState('recent')
  const [picking, setPicking] = useState(false)
  const [chosen, setChosen] = useState<Set<string>>(new Set())
  const [busy, setBusy] = useState(false)
  const [rename, setRename] = useState<{ id: string; name: string } | null>(null)
  const [menu, setMenu] = useState<{ workId: string; anchor: DockMenuAnchor } | null>(null)
  useEffect(() => {
    let active = true
    setLoading(true)
    api.chatArtifactsList(false).then((result) => {
      if (active) {
        cachedPage = { ...result, warnings: cachedPage?.warnings ?? result.warnings }
        setPage(cachedPage)
      }
      return api.chatArtifactsList(true)
    }).then((result) => { if (active) { cachedPage = result; setPage(result) } })
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
  const kindCounts = useMemo(() => {
    const counts = new Map<Kind, number>()
    for (const [item] of groups) counts.set(kindOf(item), (counts.get(kindOf(item)) ?? 0) + 1)
    return counts
  }, [groups])
  const visibleKinds = kinds.filter((value) => value === 'all' || (kindCounts.get(value) ?? 0) > 0)
  const activeKind = visibleKinds.includes(kind) ? kind : 'all'
  const filtered = groups.filter(([item]) => (activeKind === 'all' || kindOf(item) === activeKind)
    && `${item.title} ${item.artifact.name}`.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase()))
    .sort((a, b) => sort === 'name' ? displayTitle(a[0]).localeCompare(displayTitle(b[0]), zh ? 'zh-CN' : 'en') : sort === 'oldest' ? a[0].createdAt - b[0].createdAt : b[0].createdAt - a[0].createdAt)
  const managing = picking || chosen.size > 0
  const chosenWorks = filtered.filter(([item]) => chosen.has(item.workId))

  const reload = () => { setError(''); setRefresh((v) => v + 1) }
  const toggleChosen = (workId: string) => {
    setChosen((prev) => {
      const next = new Set(prev)
      if (next.has(workId)) next.delete(workId)
      else next.add(workId)
      return next
    })
  }
  async function run(action: () => Promise<void>) {
    if (busy) return
    setBusy(true); setError('')
    try { await action() }
    catch (e) { setError(String(e)) }
    finally { setBusy(false) }
  }
  function openWork(item: ArtifactLibraryItem) {
    if (!item.available) { setError(zh ? '文件缺失，无法打开原文件。' : 'File missing. Cannot open the original.'); return }
    void run(async () => { await api.chatArtifactAction(item.id, 'open') })
  }
  function recordIds(workIds: string[]) {
    return groups.filter(([item]) => workIds.includes(item.workId)).flatMap((items) => items.map((item) => item.id))
  }
  function deleteWorks(workIds: string[]) {
    const ids = recordIds(workIds)
    if (!ids.length) return
    if (!window.confirm(zh ? `删除 ${workIds.length} 件作品？原聊天记录仍会保留。` : `Delete ${workIds.length} work${workIds.length === 1 ? '' : 's'}? Source chats stay unchanged.`)) return
    void run(async () => {
      const failures: string[] = []
      for (const id of ids) {
        try { await api.chatArtifactAction(id, 'delete') }
        catch (error) { failures.push(String(error)) }
      }
      setChosen(new Set())
      setPicking(false)
      reload()
      if (failures.length) throw new Error(zh ? `${failures.length} 个版本删除失败：${failures[0]}` : `${failures.length} versions could not be deleted: ${failures[0]}`)
    })
  }
  function exportWorks(items: ArtifactLibraryItem[]) {
    void run(async () => {
      if (items.length === 1) {
        const destination = await save({ defaultPath: items[0].artifact.name, title: zh ? '作品另存为' : 'Save work as' })
        if (!destination) return
        await api.chatArtifactAction(items[0].id, 'export', destination)
        return
      }
      const dir = await open({ directory: true, multiple: false, title: zh ? '选择保存目录' : 'Choose a folder' })
      if (typeof dir !== 'string') return
      for (const item of items) {
        if (!item.available) continue
        await api.chatArtifactAction(item.id, 'export_unique', dir)
      }
    })
  }
  async function submitRename() {
    if (!rename || busy) return
    const name = rename.name.trim()
    if (!name) return
    await run(async () => {
      await api.chatArtifactAction(rename.id, 'rename', undefined, name)
      setRename(null)
      reload()
    })
  }
  const menuWork = menu ? filtered.find(([item]) => item.workId === menu.workId) : undefined

  useEffect(() => {
    if (!managing) return
    const onKey = (e: KeyboardEvent) => { if (e.key === 'Escape') { setChosen(new Set()); setPicking(false) } }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [managing])

  return <section className={`kv-works custom-scrollbar${managing ? ' is-selecting' : ''}`} aria-label={zh ? '作品' : 'Works'}>
    <header className="kv-works-header">
      <div><h1>{zh ? '作品' : 'Works'} <span>{groups.length}</span></h1>
        <p>{zh ? '这里只展示聊天里交付的文件，仍留在原来的位置。' : 'A gallery of files delivered in chats. They stay where they were created.'}</p></div>
      <div className="kv-works-search"><Search size={16} /><Input aria-label={zh ? '搜索作品' : 'Search works'} placeholder={zh ? '搜索作品或聊天…' : 'Search works or chats…'} value={query} onChange={setQuery} /><IconButton className={query ? '' : 'is-idle'} label={zh ? '清除搜索' : 'Clear search'} disabled={!query} onClick={() => setQuery('')}><X size={14} /></IconButton></div>
    </header>
    <div className="kv-works-toolbar">
      {visibleKinds.length > 1 && <nav className="kv-works-kinds custom-scrollbar kv-scrollbar-autohide" aria-label={zh ? '作品类型' : 'Work types'}>{visibleKinds.map((value) => {
        const count = value === 'all' ? groups.length : (kindCounts.get(value) ?? 0)
        return <button key={value} type="button" className="kv-works-kind" aria-label={kindNames[value][zh ? 0 : 1]} aria-pressed={activeKind === value} onClick={() => setKind(value)}>{kindNames[value][zh ? 0 : 1]}<small>{count}</small></button>
      })}</nav>}
      <div className="kv-works-tools">
        <span className="kv-works-count">{managing ? (zh ? `已选 ${chosen.size} 件` : `${chosen.size} selected`) : (zh ? `${filtered.length} 件作品` : `${filtered.length} works`)}</span>
        <div className="kv-works-tools-pane" hidden={managing}>
          <Select className="w-32" ariaLabel={zh ? '作品排序' : 'Sort works'} value={sort} onChange={setSort} options={[{ value: 'recent', label: zh ? '最近创建' : 'Newest first' }, { value: 'oldest', label: zh ? '最早创建' : 'Oldest first' }, { value: 'name', label: zh ? '按名称' : 'Name' }]} />
          <div className="kv-works-layout"><Button size="sm" variant={list ? 'ghost' : 'default'} aria-label={zh ? '网格视图' : 'Grid view'} title={zh ? '网格视图' : 'Grid view'} aria-pressed={!list} onClick={() => setList(false)}><LayoutGrid size={16} /></Button><Button size="sm" variant={list ? 'default' : 'ghost'} aria-label={zh ? '列表视图' : 'List view'} title={zh ? '列表视图' : 'List view'} aria-pressed={list} onClick={() => setList(true)}><List size={16} /></Button></div>
          <Button size="sm" variant="ghost" disabled={!filtered.length} onClick={() => setPicking(true)}><CheckSquare size={14} />{zh ? '选择' : 'Select'}</Button>
          <IconButton label={zh ? '刷新作品' : 'Refresh works'} disabled={loading} onClick={reload}><RefreshCw size={15} className={loading ? 'animate-spin' : ''} /></IconButton>
        </div>
        <div className="kv-works-tools-pane" hidden={!managing}>
          <Button size="sm" disabled={busy || !filtered.length} onClick={() => setChosen(new Set(filtered.map(([item]) => item.workId)))}>{zh ? '全选' : 'Select all'}</Button>
          <Button size="sm" disabled={busy || !chosenWorks.some(([item]) => item.available)} onClick={() => exportWorks(chosenWorks.map(([item]) => item).filter((item) => item.available))}><Download size={14} />{zh ? '另存为' : 'Save as'}</Button>
          <Button size="sm" variant="danger" disabled={busy || !chosen.size} onClick={() => deleteWorks([...chosen])}><Trash2 size={14} />{zh ? '删除' : 'Delete'}</Button>
          <Button size="sm" variant="ghost" disabled={busy} onClick={() => { setChosen(new Set()); setPicking(false) }}>{zh ? '完成' : 'Done'}</Button>
        </div>
      </div>
    </div>
    {error && <div role="alert" className="kv-works-notice">{error}<Button size="sm" onClick={reload}>{zh ? '重试' : 'Retry'}</Button></div>}
    {loading && !page.items.length ? <div className="kv-works-empty" role="status">{zh ? '正在整理作品…' : 'Loading works…'}</div>
        : !filtered.length ? <div className="kv-works-empty"><span className="kv-works-mark-well"><WorksIcon size={34} strokeWidth={1.6} /></span><h2>{query || activeKind !== 'all' ? (zh ? '没有找到匹配的作品' : 'No matching works') : (zh ? '你的创作，从这里开始' : 'Your creations belong here')}</h2><p>{zh ? '聊天里交付的文档、图片、表格会显示在这里，文件仍留在项目或对话原来的位置。删除对话后，对应作品会一起消失。' : 'Documents, images and spreadsheets delivered in chats appear here. The files stay in the project or conversation. Deleting a chat removes its works from this gallery.'}</p></div>
        : <div className={`kv-works-grid ${list ? 'is-list' : ''}`} aria-busy={loading}>
          {filtered.map((items) => <WorkCard key={items[0].id} item={items[0]} versions={items.length} zh={zh} checked={chosen.has(items[0].workId)} managing={managing}
            onOpen={() => managing ? toggleChosen(items[0].workId) : openWork(items[0])}
            onToggle={() => toggleChosen(items[0].workId)}
            onMenu={(anchor) => setMenu({ workId: items[0].workId, anchor })} />)}
        </div>}
    {page.warnings > 0 && <details className="kv-works-import-note"><summary>{zh ? `${page.warnings} 项历史内容未导入` : `${page.warnings} historical items not imported`}</summary><p>{zh ? '部分历史附件暂时无法读取。原聊天记录保留，你可以回到聊天查看，恢复文件后刷新重试。' : 'Some historical attachments could not be read. Their chats are unchanged. Restore the files and refresh to retry.'}</p></details>}
    {menu && menuWork && <DockContextMenu anchor={menu.anchor} onClose={() => setMenu(null)} items={[
      { key: 'open', label: zh ? '打开文件' : 'Open file', icon: <ExternalLink size={16} />, disabled: !menuWork[0].available, onSelect: () => openWork(menuWork[0]) },
      ...menuWork.slice(1).map((version, index) => ({ key: version.id, label: `${zh ? '版本' : 'Version'} ${menuWork.length - index - 1}`, icon: <Layers size={16} />, disabled: !version.available, onSelect: () => openWork(version) })),
      { key: 'reveal', label: zh ? '打开所在位置' : 'Show in folder', icon: <FolderOpen size={16} />, disabled: !menuWork[0].available, onSelect: () => { void run(async () => { await api.chatArtifactAction(menuWork[0].id, 'reveal') }) } },
      { key: 'rename', label: zh ? '重命名' : 'Rename', icon: <Pencil size={16} />, onSelect: () => setRename({ id: menuWork[0].id, name: menuWork[0].artifact.name }) },
      { key: 'source', label: zh ? '回到聊天' : 'Open chat', icon: <MessageSquare size={16} />, disabled: !menuWork[0].sourceAvailable, onSelect: () => onOpenConversation(menuWork[0].conversationId) },
      { key: 'export', label: zh ? '另存为' : 'Save as', icon: <Download size={16} />, disabled: !menuWork[0].available, onSelect: () => exportWorks([menuWork[0]]) },
      { key: 'delete', label: zh ? '删除' : 'Delete', icon: <Trash2 size={16} />, danger: true, onSelect: () => deleteWorks([menuWork[0].workId]) },
    ]} />}
    {rename && <RenameDialog zh={zh} name={rename.name} busy={busy} onChange={(name) => setRename({ ...rename, name })} onCancel={() => setRename(null)} onSave={() => void submitRename()} />}
  </section>
}

function RenameDialog({ zh, name, busy, onChange, onCancel, onSave }: { zh: boolean; name: string; busy: boolean; onChange: (name: string) => void; onCancel: () => void; onSave: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null)
  useEffect(() => { dialog.current?.showModal() }, [])
  return <dialog ref={dialog} className="kv-modal kv-work-rename" aria-label={zh ? '重命名作品' : 'Rename work'} onCancel={onCancel} onClick={(e) => { if (e.target === e.currentTarget) onCancel() }}>
    <h2>{zh ? '重命名' : 'Rename'}</h2>
    <Input autoFocus aria-label={zh ? '作品名称' : 'Work name'} value={name} onChange={onChange} onKeyDown={(e) => { if (e.key === 'Enter') onSave(); if (e.key === 'Escape') onCancel() }} />
    <div className="kv-work-rename-actions"><Button size="sm" variant="ghost" onClick={onCancel}>{zh ? '取消' : 'Cancel'}</Button><Button size="sm" variant="primary" disabled={busy || !name.trim()} onClick={onSave}>{zh ? '保存' : 'Save'}</Button></div>
  </dialog>
}

function WorkCard({ item, versions, zh, checked, managing, onOpen, onToggle, onMenu }: {
  item: ArtifactLibraryItem; versions: number; zh: boolean; checked: boolean; managing: boolean
  onOpen: () => void; onToggle: () => void; onMenu: (anchor: DockMenuAnchor) => void
}) {
  const kind = kindOf(item)
  const Icon = kindIcons[kind]
  const thumbnail = kind === 'image' ? artifactDataUrl(item.artifact) : ''
  const title = displayTitle(item)
  const sourceIsTitle = title === item.title
  const [imageFailed, setImageFailed] = useState(false)
  const openMenu = (e: MouseEvent) => { e.preventDefault(); e.stopPropagation(); onMenu({ left: e.clientX, top: e.clientY }) }
  return <article className={`kv-work-card kind-${kind}${checked ? ' is-selected' : ''}`}>
    <label className="kv-work-check" onClick={(e) => e.stopPropagation()}>
      <input type="checkbox" checked={checked} onChange={onToggle} aria-label={zh ? `选择 ${title}` : `Select ${title}`} />
    </label>
    <IconButton className="kv-work-more" label={zh ? '更多操作' : 'More actions'} onClick={(e) => { e.stopPropagation(); onMenu({ left: e.currentTarget.getBoundingClientRect().right - 168, top: e.currentTarget.getBoundingClientRect().bottom + 4 }) }}><MoreHorizontal size={15} /></IconButton>
    <button type="button" className="kv-work-open" aria-label={item.artifact.name} onClick={onOpen} onContextMenu={managing ? undefined : openMenu}>
      <div className="kv-work-cover">
        {thumbnail && !imageFailed ? <img src={thumbnail} alt="" loading="lazy" onError={() => setImageFailed(true)} /> : <div className="kv-work-file-cover">
          <div className="kv-work-file-format"><Icon size={18} strokeWidth={1.5} /><span>{extension(item)}</span></div>
          <strong>{displayTitle(item)}</strong>
          <span className="kv-work-file-caption">{kindNames[kind][zh ? 0 : 1]}{sizeLabel(item) && ` · ${sizeLabel(item)}`}</span>
        </div>}
        {versions > 1 && <span className="kv-work-versions"><Layers size={11} />{versions} {zh ? '个版本' : 'versions'}</span>}
      </div>
      <div className="kv-work-info"><div className="kv-work-title"><Icon size={14} /><h2 title={item.artifact.name}>{title}</h2><span className="kv-work-format">{extension(item)}</span></div><p title={item.title}>{sourceIsTitle ? <File size={11} /> : <MessageSquare size={11} />}<span>{sourceIsTitle ? item.artifact.name : item.title || (zh ? '来源聊天' : 'Source chat')}</span></p></div>
      <div className="kv-work-meta"><span>{new Date(item.createdAt * 1000).toLocaleDateString(zh ? 'zh-CN' : 'en-US', { month: 'short', day: 'numeric', year: 'numeric' })}</span><span>{item.available ? sizeLabel(item) : (zh ? '文件缺失' : 'File missing')}</span></div>
    </button>
  </article>
}
