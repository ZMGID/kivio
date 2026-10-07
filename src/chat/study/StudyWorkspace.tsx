import { useCallback, useContext, useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from 'react'
import { BookOpen, FileText, History, Lightbulb, MessageCircle, PanelLeftClose, PanelLeftOpen, Plus, Trash2 } from 'lucide-react'
import { Button, IconButton } from '../../components/Button'
import { confirmDialog } from '../../components/dialogQueue'
import { useLang } from '../../components/i18n'
import { Select, TextArea } from '../../settings/public/controls'
import { copyToClipboard } from '../../utils/clipboard'
import { useWindowStore } from '../../utils/windowStore'
import { ChatSurfaceActivityContext } from '../chatSurfaceActivity'
import type { Conversation, StudyMessageSource } from '../types'
import { StudyReader } from './StudyReader'
import type { StudyReaderContext } from './studyMaterial'
import { readStudyDocumentBlob, type StudyDocument, type StudyTurn } from './studyStorage'
import { cancelStudyImport, editStudyPage, importStudyFile, initializeStudy, openStudyPage, removeStudyDocument, reloadStudyWorkspace, retryStudySave, sameStudyRegion, studyPage, studyWorkspace } from './studyWorkspaceStore'
import './StudyWorkspace.css'

type Pane = 'library' | 'reader' | 'help'
const MODES: StudyTurn['mode'][] = ['read', 'explain', 'hint', 'check', 'solution']
const modeNames = { read: ['阅读问答', 'Reading Q&A'], hint: ['提示一步', 'One hint'], explain: ['讲清概念', 'Explain'], check: ['检查我的解答', 'Check my attempt'], solution: ['完整解答', 'Full solution'] } as const

/** Material context for the existing Chat owner; no conversation or request state lives here. */
export interface StudyReadingSurface {
  document: StudyDocument
  page: number
  context: StudyReaderContext | null
  source: StudyMessageSource
  focusRequest: number
  controls: ReactNode
  emptyState: ReactNode
  showSource: (source: StudyMessageSource) => void
}
interface StudyWorkspaceProps {
  conversation: Conversation | null
  busy: boolean
  onReloadSaved?: () => void
  onMaterialRemoved?: (materialId: string) => void
  onOpenPage: (document: StudyDocument, page: number) => Promise<void>
  renderChat: (surface: StudyReadingSurface) => ReactNode
}

export function StudyWorkspace({ conversation, busy, onOpenPage, onReloadSaved, onMaterialRemoved, renderChat }: StudyWorkspaceProps) {
  const surfaceActive = useContext(ChatSurfaceActivityContext)
  const zh = useLang() === 'zh'
  const text = useCallback((cn: string, en: string) => zh ? cn : en, [zh])
  const [state] = useWindowStore(studyWorkspace)
  const [mode, setMode] = useState<StudyTurn['mode']>('read')
  const [pane, setPane] = useState<Pane>('reader')
  const [libraryPreference, setLibraryPreference] = useState<boolean | null>(null)
  const [focusRequest, setFocusRequest] = useState(0)
  const [blobState, setBlobState] = useState<{ id: string; blob?: Blob; error?: string }>({ id: '' })
  const [context, setContext] = useState<{ documentId: string; value: StudyReaderContext } | null>(null)
  const [opening, setOpening] = useState<{ key: string; error?: string; ready?: boolean }>({ key: '' })
  const [openAttempt, setOpenAttempt] = useState<string | null>(null)
  const [openRevision, setOpenRevision] = useState(0)
  const [actionError, setActionError] = useState('')
  const [actionNotice, setActionNotice] = useState('')
  const fileInput = useRef<HTMLInputElement>(null)
  const doc = state.documents.find(item => item.id === state.selectedDocumentId)
  const documentId = doc?.id
  const page = doc ? studyPage(doc) : null
  const pageNumber = doc?.lastPage ?? 1
  const pageKey = `${documentId ?? ''}:${pageNumber}`
  const libraryOpen = libraryPreference ?? state.documents.length > 1
  const readingMode = mode === 'read' || mode === 'explain'
  const attemptOpen = openAttempt === pageKey || mode === 'check'
  const currentContext = doc && context?.documentId === doc.id && context.value.page === pageNumber && sameStudyRegion(context.value.region, page?.region) ? context.value : null
  const conversationHere = conversation?.study_context?.materialId === documentId && conversation?.study_context?.page === pageNumber
  const currentDoc = useRef(doc)
  currentDoc.current = doc

  useEffect(() => { void initializeStudy() }, [])
  useEffect(() => { setFocusRequest(0) }, [pageKey])
  useEffect(() => { setMode('read'); setOpenAttempt(null) }, [documentId])
  useEffect(() => {
    let active = true
    if (!documentId) return
    const id = documentId
    setBlobState({ id })
    void readStudyDocumentBlob(id).then(blob => { if (active) setBlobState({ id, blob }) }).catch(error => { if (active) setBlobState({ id, error: String(error) }) })
    return () => { active = false }
  }, [documentId])
  useEffect(() => {
    const selected = currentDoc.current
    if (!selected || !surfaceActive) return
    let active = true
    setOpening({ key: pageKey })
    void onOpenPage(selected, pageNumber).then(() => { if (active) setOpening({ key: pageKey, ready: true }) }).catch(error => { if (active) setOpening({ key: pageKey, error: String(error) }) })
    return () => { active = false }
  }, [pageKey, pageNumber, onOpenPage, openRevision, surfaceActive])
  useEffect(() => {
    const guard = (event: BeforeUnloadEvent) => { if (studyWorkspace.getSnapshot().dirtyIds.length) { event.preventDefault(); event.returnValue = '' } }
    window.addEventListener('beforeunload', guard)
    return () => window.removeEventListener('beforeunload', guard)
  }, [])

  const handleContext = useCallback((value: StudyReaderContext) => { if (documentId) setContext({ documentId, value }) }, [documentId])
  const edit = (patch: Parameters<typeof editStudyPage>[2]) => { if (doc) editStudyPage(doc.id, pageNumber, patch) }
  const upload = (file?: File) => { if (file) { setActionError(''); void importStudyFile(file); setPane('reader') } }
  const ask = () => { setPane('help'); setFocusRequest(value => value + 1) }
  const showSource = (source: StudyMessageSource) => {
    if (!doc) return
    openStudyPage(doc.id, source.page, source.region ?? null)
    setPane('reader')
    requestAnimationFrame(() => document.querySelector<HTMLElement>('.kv-study-reader-paper')?.focus({ preventScroll: true }))
  }
  const keyTabs = (event: KeyboardEvent<HTMLButtonElement>, current: Pane) => {
    const tabs: Pane[] = ['library', 'reader', 'help']
    const step = event.key === 'ArrowRight' ? 1 : event.key === 'ArrowLeft' ? -1 : 0
    if (!step) return
    event.preventDefault(); const next = tabs[(tabs.indexOf(current) + step + tabs.length) % tabs.length]
    setPane(next); document.getElementById(`study-tab-${next}`)?.focus()
  }
  const copyUnsaved = async () => {
    const current = studyWorkspace.getSnapshot()
    const copied = await copyToClipboard(JSON.stringify(current.documents.filter(item => current.dirtyIds.includes(item.id)), null, 2))
    if (copied) setActionNotice(text('已复制未保存记录，请粘贴到安全位置后再重新读取。', 'Unsaved records copied. Paste them somewhere safe before reloading.'))
    else setActionError(text('复制失败，请手动保留草稿后再重新读取。', 'Copy failed. Preserve your draft before reloading.'))
  }
  const reloadSaved = async () => {
    if (!(await confirmDialog({ title: text('重新读取已保存版本', 'Reload saved version'), message: text('这会替换当前窗口中未保存的材料草稿。请先复制未保存记录。聊天记录不会被替换。', 'This replaces unsaved material drafts in this window. Copy them first. Chat history is not replaced.'), confirmLabel: text('重新读取', 'Reload saved version'), danger: true }))) return
    try { await reloadStudyWorkspace(); onReloadSaved?.(); setOpenRevision(value => value + 1); setActionError(''); setActionNotice('') } catch (error) { setActionError(String(error)) }
  }
  const remove = async () => {
    if (!doc || busy || !(await confirmDialog({ title: text('移除学习材料', 'Remove study material'), message: text(`移除“${doc.name}”及其材料草稿、笔记和旧记录备份？此操作无法撤销。原始文件和已迁移的聊天记录保留。`, `Remove “${doc.name}”, its material drafts, notes and legacy backup? This cannot be undone. The original file and migrated Chat conversations are retained.`), confirmLabel: text('移除', 'Remove'), danger: true }))) return
    try { await removeStudyDocument(doc.id); onMaterialRemoved?.(doc.id) } catch (error) { setActionError(String(error)) }
  }
  const source: StudyMessageSource = { page: pageNumber, region: page?.region ?? null, mode, attempt: page?.attempt ?? '' }
  const controls = <div className="kv-study-reading-controls">
    <div className="kv-study-compose-tools"><Select ariaLabel={text('帮助方式', 'Help mode')} value={mode} onChange={value => setMode(value as StudyTurn['mode'])} triggerLabel={modeNames[mode][zh ? 0 : 1]} options={MODES.map(item => ({ value: item, label: modeNames[item][zh ? 0 : 1] }))} />
      <Button size="sm" variant="ghost" onClick={() => setOpenAttempt(attemptOpen ? null : pageKey)}>{readingMode ? text('补充说明', 'Additional context') : text('我的思路', 'My attempt')}</Button></div>
    {attemptOpen && page && <TextArea aria-label={readingMode ? text('补充说明', 'Additional context') : text('我的解答或思路', 'My attempt')} aria-required={mode === 'check'} value={page.attempt} onChange={attempt => edit({ attempt })} autoSize={{ minRows: 2, maxRows: 5 }} maxLength={12000} placeholder={text('可补充你的理解、尝试或阅读目标', 'Add your interpretation, attempt or reading goal')} />}
  </div>
  const emptyState = <div className="kv-study-help-empty"><Lightbulb size={25} /><h3>{text('读懂这一页', 'Read this page your way')}</h3><p>{text('选中段落或图表，直接问你想了解的内容。发给模型的是当前页或选区。', 'Select a passage or figure and ask what you want to understand. The model receives the current page or selection.')}</p></div>

  return <section className="kv-study" aria-label="Kivio Study">
    <header className="kv-study-header"><div className="kv-study-heading"><BookOpen size={23} /><div><h1>Kivio Study</h1><p>{text('读自己的材料，把不懂的地方弄明白', 'Read your material. Make sense of it.')}</p></div></div>
      <div className="kv-study-header-actions"><span className="kv-study-save-status" role="status">{state.saveError ? text('材料草稿尚未保存', 'Material draft not saved') : state.dirtyIds.length ? text('正在保存…', 'Saving…') : text('材料草稿保存在此设备', 'Material drafts saved on this device')}</span><Button size="sm" onClick={() => fileInput.current?.click()} disabled={state.importing}><Plus size={15} />{text('导入材料', 'Import material')}</Button><input ref={fileInput} className="sr-only" type="file" accept="application/pdf,image/png,image/jpeg,image/webp,.pdf,.png,.jpg,.jpeg,.webp" aria-label={text('导入 PDF 或图片', 'Import PDF or image')} onChange={event => { upload(event.target.files?.[0]); event.target.value = '' }} /></div>
    </header>
    {(state.error || actionError) && <div className="kv-panel warn kv-study-banner" role="alert">{actionError || state.error}{!state.loaded && <Button size="sm" onClick={() => void initializeStudy()}>{text('重试读取', 'Retry loading')}</Button>}</div>}
    {state.saveError && <div className="kv-panel warn kv-study-banner" role="alert"><span>{text('材料草稿保存失败，关闭前请重试：', 'Material drafts could not be saved. Retry before closing: ')}{state.saveError}</span><Button size="sm" onClick={retryStudySave}>{text('重试保存', 'Retry save')}</Button><Button size="sm" onClick={() => void copyUnsaved()}>{text('复制未保存记录', 'Copy unsaved work')}</Button><Button size="sm" onClick={() => void reloadSaved()} disabled={busy || state.importing}>{text('重新读取已保存版本', 'Reload saved version')}</Button></div>}
    {(actionNotice || state.notice) && <div className="kv-study-banner" role="status">{actionNotice || state.notice}</div>}
    {state.importing && <div className="kv-study-banner" role="status">{text('正在检查并导入材料…', 'Checking and importing material…')}<Button size="sm" onClick={cancelStudyImport}>{text('取消', 'Cancel')}</Button></div>}
    <nav className="kv-study-tabs" role="tablist" aria-label={text('学习面板', 'Study panes')}>{(['library', 'reader', 'help'] as Pane[]).map((item, index) => <button id={`study-tab-${item}`} key={item} type="button" role="tab" aria-selected={pane === item} aria-controls={`study-pane-${item}`} tabIndex={pane === item ? 0 : -1} onClick={() => setPane(item)} onKeyDown={event => keyTabs(event, item)}>{[text('材料与页面', 'Library'), text('阅读', 'Read'), text('学习助手', 'Help')][index]}</button>)}</nav>
    {!state.loaded ? <div className="kv-study-empty" role="status">{text('正在恢复学习记录…', 'Restoring your study space…')}</div> : !state.documents.length ? <div className="kv-study-empty" onDragOver={event => event.preventDefault()} onDrop={event => { event.preventDefault(); upload(event.dataTransfer.files[0]) }}><div className="kv-study-hero-icon"><BookOpen size={38} /></div><h2>{text('从想读懂的那一页开始', 'Start with the page you want to understand')}</h2><p>{text('导入论文、英文文章、讲义或习题，读一页，选中想了解的部分，直接提问。', 'Bring a paper, article, handout or exercise. Read a page, select what matters, and ask.')}</p><Button variant="primary" onClick={() => fileInput.current?.click()}><Plus size={16} />{text('打开 PDF 或图片', 'Open a PDF or image')}</Button><small>PDF · PNG · JPEG · WebP · 25 MB</small><p className="kv-study-privacy">{text('材料保存在此设备。发送时，选中的图片会交给所选模型服务。', 'Materials stay on this device. Sending shares the selected image with your chosen model service.')}</p></div> : <div className={`kv-study-layout ${libraryOpen ? '' : 'is-library-closed'}`} data-pane={pane}>
      <aside id="study-pane-library" className="kv-study-library custom-scrollbar" aria-label={text('材料与页面', 'Materials and pages')}><div className="kv-study-pane-heading"><h2>{text('我的材料', 'My materials')}</h2><IconButton className="kv-study-library-toggle" label={text('收起材料栏', 'Collapse library')} variant="ghost" onClick={() => setLibraryPreference(false)}><PanelLeftClose size={16} /></IconButton></div><div className="kv-study-document-list">{state.documents.map(item => <button className="kv-study-list-row" type="button" key={item.id} aria-current={item.id === doc?.id ? 'true' : undefined} onClick={() => { openStudyPage(item.id, item.lastPage); setPane('reader') }}><FileText size={16} /><span><strong>{item.name}</strong><small>{text(`第 ${item.lastPage} / ${item.pageCount} 页`, `Page ${item.lastPage} of ${item.pageCount}`)}</small></span></button>)}</div><div className="kv-study-pane-heading"><h2><History size={15} />{text('读过的页面', 'Visited pages')}</h2></div>{doc && [...new Set([...Object.keys(doc.pages).map(Number), pageNumber])].sort((a, b) => a - b).map(number => <button type="button" key={number} className="kv-study-list-row" aria-current={number === pageNumber ? 'true' : undefined} onClick={() => { openStudyPage(doc.id, number); setPane('help') }}><span>{text(`第 ${number} 页`, `Page ${number}`)}</span></button>)}</aside>
      <main id="study-pane-reader" className="kv-study-reading" aria-label={text('材料阅读器', 'Material reader')}><div className="kv-study-pane-heading"><div className="kv-study-reader-title">{!libraryOpen && <IconButton className="kv-study-library-toggle" label={text('展开材料栏', 'Expand library')} variant="ghost" onClick={() => setLibraryPreference(true)}><PanelLeftOpen size={16} /></IconButton>}<h2 title={doc?.name}>{doc?.name}</h2></div><IconButton label={text('移除当前材料', 'Remove current material')} variant="ghost" disabled={busy} onClick={() => void remove()}><Trash2 size={15} /></IconButton></div>
        {doc && blobState.id === doc.id && blobState.error && <p role="alert">{blobState.error}</p>}
        {doc && blobState.id === doc.id && blobState.blob ? <StudyReader key={doc.id} blob={blobState.blob} page={pageNumber} onPageChange={next => openStudyPage(doc.id, next)} region={page?.region ?? null} onRegionChange={region => edit({ region })} onContextChange={handleContext} onAsk={ask} /> : <p role="status">{text('正在打开材料…', 'Opening material…')}</p>}
        {doc && page && <details className="kv-study-notes"><summary>{text(`第 ${pageNumber} 页笔记`, `Notes for page ${pageNumber}`)}</summary><TextArea aria-label={text('本页笔记', 'Page notes')} value={page.notes} onChange={notes => edit({ notes })} autoSize={{ minRows: 2, maxRows: 6 }} maxLength={12000} placeholder={text('用自己的话记下关键观点', 'Note a key idea in your own words')} /></details>}
        {page && (page.history.length > 0 || page.correctedText) && <details className="kv-study-notes"><summary>{text('旧记录备份（只读，可能含完整答案）', 'Legacy backup (read-only, may include full answers)')}</summary><p className="kv-study-muted">{text('原始旧记录保留在此处，不会作为材料发送给模型。', 'Original legacy records are retained here and are not sent as source material.')}</p><pre className="kv-study-legacy-backup custom-scrollbar">{JSON.stringify({ manuallyAddedText: page.correctedText, history: page.history }, null, 2)}</pre></details>}
      </main>
      <section id="study-pane-help" className="kv-study-help" aria-label={text('学习助手', 'Study helper')}><div className="kv-study-pane-heading"><h2><MessageCircle size={16} />{text('一起想明白', 'Work it through')}</h2><Button size="sm" variant="ghost" aria-label={text(`查看第 ${pageNumber} 页${page?.region ? '选区' : ''}`, `View page ${pageNumber}${page?.region ? ' selection' : ''}`)} onClick={() => showSource(source)}>{page?.region && currentContext?.imageDataUrl && <img className="kv-study-context-thumbnail" src={currentContext.imageDataUrl} alt="" />}<span className="kv-study-page-label">{text(`第 ${pageNumber} 页`, `Page ${pageNumber}`)}{page?.region && text(' · 选区', ' · selection')}</span></Button></div>
        {opening.key === pageKey && opening.error ? <div className="kv-study-banner" role="alert">{opening.error}<Button onClick={() => setOpenRevision(value => value + 1)}>{text('重新打开对话', 'Retry opening conversation')}</Button></div> : doc && opening.key === pageKey && opening.ready && conversationHere ? renderChat({ document: doc, page: pageNumber, context: currentContext, source, focusRequest, controls, emptyState, showSource }) : <p role="status" className="kv-study-muted">{text('正在打开本页对话…', 'Opening this page’s conversation…')}</p>}
      </section>
    </div>}
  </section>
}
