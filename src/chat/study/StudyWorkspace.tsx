import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from 'react'
import { BookOpen, FileText, History, Lightbulb, MessageCircle, PanelLeftClose, PanelLeftOpen, Plus, RotateCcw, Send, Square, Trash2 } from 'lucide-react'
import { Button, IconButton } from '../../components/Button'
import { confirmDialog } from '../../components/dialogQueue'
import { useLang } from '../../components/i18n'
import { Select, TextArea } from '../../settings/public/controls'
import { getSettingsCached, subscribeSettings } from '../../api/settingsCache'
import { type ModelProvider, type Settings } from '../../api/tauri'
import { resolveModelInfo } from '../../data/modelMatching'
import { copyToClipboard } from '../../utils/clipboard'
import { useWindowStore } from '../../utils/windowStore'
import { ChatMarkdown } from '../ChatMarkdown'
import { StudyReader } from './StudyReader'
import type { StudyReaderContext } from './studyMaterial'
import { readStudyDocumentBlob, type StudyTurn } from './studyStorage'
import { cancelStudyHelp, cancelStudyImport, editStudyPage, importStudyFile, initializeStudy, openStudyPage, removeStudyDocument, reloadStudyWorkspace, retryStudySave, sameStudyRegion, sendStudyHelp, studyPage, studyWorkspace } from './studyWorkspaceStore'
import './StudyWorkspace.css'

type Pane = 'library' | 'reader' | 'help'
const MODES: StudyTurn['mode'][] = ['hint', 'explain', 'check', 'solution']
const modeNames = { hint: ['提示一步', 'One hint'], explain: ['讲清概念', 'Explain'], check: ['检查我的解答', 'Check my attempt'], solution: ['完整解答', 'Full solution'] } as const

export function StudyWorkspace({ onOpenSettings }: { onOpenSettings: () => void }) {
  const zh = useLang() === 'zh'
  const text = useCallback((cn: string, en: string) => zh ? cn : en, [zh])
  const [state] = useWindowStore(studyWorkspace)
  const [providers, setProviders] = useState<ModelProvider[]>([])
  const [providerId, setProviderId] = useState('')
  const [model, setModel] = useState('')
  const [settingsError, setSettingsError] = useState('')
  const [mode, setMode] = useState<StudyTurn['mode']>('hint')
  const [pane, setPane] = useState<Pane>('reader')
  const [libraryOpen, setLibraryOpen] = useState(true)
  const [blobState, setBlobState] = useState<{ id: string; blob?: Blob; error?: string }>({ id: '' })
  const [context, setContext] = useState<{ documentId: string; value: StudyReaderContext } | null>(null)
  const [includeImage, setIncludeImage] = useState(true)
  const [actionError, setActionError] = useState('')
  const [actionNotice, setActionNotice] = useState('')
  const [retryTurn, setRetryTurn] = useState<StudyTurn | null>(null)
  const fileInput = useRef<HTMLInputElement>(null)
  const historyEnd = useRef<HTMLDivElement>(null)
  const doc = state.documents.find((item) => item.id === state.selectedDocumentId)
  const documentId = doc?.id
  const page = doc ? studyPage(doc) : null
  const provider = providers.find((item) => item.id === providerId)
  const vision = Boolean(provider && resolveModelInfo(model, provider.modelOverrides, provider).capabilities?.vision)
  const currentContext = doc && context?.documentId === doc.id && context.value.page === doc.lastPage && sameStudyRegion(context.value.region, page?.region) ? context.value : null
  const sendingImage = retryTurn ? Boolean(retryTurn.sourceImageUsed) : vision && includeImage
  const activeHere = state.activeRequest?.documentId === doc?.id && state.activeRequest?.page === doc?.lastPage
  const canSend = Boolean(doc && currentContext?.status === 'ready' && provider && model && !state.activeRequest)
  const allTurns = doc ? Object.values(doc.pages).flatMap((item) => item.history).sort((a, b) => b.createdAt - a.createdAt) : []

  useEffect(() => { void initializeStudy() }, [])
  useEffect(() => {
    let active = true
    const apply = (settings: Settings) => {
      if (!active) return
      const enabled = settings.providers.filter((item) => item.enabled && item.enabledModels.length)
      setProviders(enabled)
      const preferred = settings.defaultModels?.chat
      setProviderId((current) => enabled.some((item) => item.id === current) ? current : enabled.find((item) => item.id === preferred?.providerId)?.id ?? enabled[0]?.id ?? '')
      setModel((current) => current || preferred?.model || '')
      setSettingsError('')
    }
    void getSettingsCached().then(apply).catch((error) => { if (active) setSettingsError(String(error)) })
    const unsubscribe = subscribeSettings(apply)
    return () => { active = false; unsubscribe() }
  }, [])
  useEffect(() => {
    if (provider && !provider.enabledModels.includes(model)) setModel(provider.enabledModels[0] ?? '')
  }, [provider, model])
  useEffect(() => {
    let active = true
    setRetryTurn(null); setActionError('')
    if (documentId) {
      const id = documentId
      setBlobState({ id })
      void readStudyDocumentBlob(id).then((blob) => { if (active) setBlobState({ id, blob }) }).catch((error) => { if (active) setBlobState({ id, error: String(error) }) })
    }
    return () => { active = false }
  }, [documentId])
  useEffect(() => {
    const guard = (event: BeforeUnloadEvent) => { if (studyWorkspace.getSnapshot().dirtyIds.length) { event.preventDefault(); event.returnValue = '' } }
    window.addEventListener('beforeunload', guard)
    return () => window.removeEventListener('beforeunload', guard)
  }, [])
  useEffect(() => { if (state.selectedTurnId) document.getElementById(`study-turn-${state.selectedTurnId}`)?.scrollIntoView({ block: 'nearest' }) }, [state.selectedTurnId])

  const handleContext = useCallback((value: StudyReaderContext) => {
    if (documentId) setContext({ documentId, value })
  }, [documentId])
  const edit = (patch: Parameters<typeof editStudyPage>[2]) => { if (doc) editStudyPage(doc.id, doc.lastPage, patch) }
  const upload = (file?: File) => { if (file) { setActionError(''); void importStudyFile(file); setPane('reader') } }
  const selectHistory = (turn: StudyTurn) => {
    if (!doc) return
    openStudyPage(doc.id, turn.page, turn.region ?? null, turn.id)
    setRetryTurn(null); setPane('help')
  }
  const prepareRetry = (turn: StudyTurn) => {
    if (!doc) return
    openStudyPage(doc.id, turn.page, turn.region ?? null, turn.id)
    setMode(turn.mode); setRetryTurn(turn); setPane('help'); setActionError('')
  }
  const send = async () => {
    if (!doc || !currentContext) return
    setActionError('')
    try {
      if (retryTurn?.sourceImageUsed && !vision) throw new Error(text('原问题使用了页面图片，请选择视觉模型后重试。', 'The original question included a page image. Choose an image-capable model to retry.'))
      await sendStudyHelp({ documentId: doc.id, page: doc.lastPage, mode, providerId, model, context: currentContext, includeImage: vision && includeImage, retry: retryTurn ?? undefined })
      setRetryTurn(null)
    } catch (error) { setActionError(error instanceof Error ? error.message : String(error)) }
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
    const unsaved = current.documents.filter((item) => current.dirtyIds.includes(item.id))
    const copied = await copyToClipboard(JSON.stringify(unsaved, null, 2))
    if (copied) setActionNotice(text('已复制未保存的文字记录，请粘贴到安全位置后再重新读取。', 'Unsaved text records copied. Paste them somewhere safe before reloading.'))
    else setActionError(text('复制失败，请手动保留草稿后再重新读取。', 'Copy failed. Preserve your draft manually before reloading.'))
  }
  const reloadSaved = async () => {
    if (!(await confirmDialog({ title: text('重新读取已保存版本', 'Reload saved version'), message: text('这会替换当前窗口中全部未保存的草稿。请先复制未保存记录。其他窗口保存的内容不会被改动。', 'This replaces all unsaved drafts in this window. Copy your unsaved work first. Work saved by another window stays unchanged.'), confirmLabel: text('重新读取', 'Reload saved version'), danger: true }))) return
    try { await reloadStudyWorkspace(); setRetryTurn(null); setActionError(''); setActionNotice('') }
    catch (error) { setActionError(String(error)) }
  }
  const remove = async () => {
    if (!doc || !(await confirmDialog({ title: text('移除学习材料', 'Remove study material'), message: text(`移除“${doc.name}”及其全部问题、草稿与笔记？此操作无法撤销，原始文件不受影响。`, `Remove “${doc.name}” and all its questions, drafts and notes? This cannot be undone. Your original file is unchanged.`), confirmLabel: text('移除', 'Remove'), danger: true }))) return
    try { await removeStudyDocument(doc.id) } catch (error) { setActionError(String(error)) }
  }

  return <section className="kv-study" aria-label="Kivio Study">
    <header className="kv-study-header">
      <div className="kv-study-heading"><BookOpen size={23} strokeWidth={1.65} /><div><h1>Kivio Study</h1><p>{text('读自己的材料，把卡住的一步弄明白', 'Your material. Your next step.')}</p></div></div>
      <div className="kv-study-header-actions">
        <span className="kv-study-save-status" role="status">{state.saveError ? text('尚未保存', 'Not saved') : state.dirtyIds.length ? text('正在保存…', 'Saving…') : text('保存在此设备', 'Saved on this device')}</span>
        <Button size="sm" onClick={() => fileInput.current?.click()} disabled={state.importing}><Plus size={15} />{text('导入材料', 'Import material')}</Button>
        <input ref={fileInput} className="sr-only" type="file" accept="application/pdf,image/png,image/jpeg,image/webp,.pdf,.png,.jpg,.jpeg,.webp" aria-label={text('导入 PDF 或图片', 'Import PDF or image')} onChange={(event) => { upload(event.target.files?.[0]); event.target.value = '' }} />
      </div>
    </header>
    {(state.error || actionError || settingsError) && <div className="kv-panel warn kv-study-banner" role="alert">{actionError || state.error || settingsError}{state.error && !state.loaded && <Button size="sm" onClick={() => void initializeStudy()}>{text('重试读取', 'Retry loading')}</Button>}</div>}
    {state.saveError && <div className="kv-panel warn kv-study-banner" role="alert"><span>{text('保存失败，草稿仍在当前窗口。关闭前请重试：', 'Save failed. Your draft is still in this window. Retry before closing: ')}{state.saveError}</span><Button size="sm" onClick={retryStudySave}>{text('重试保存', 'Retry save')}</Button><Button size="sm" onClick={() => void copyUnsaved()}>{text('复制未保存记录', 'Copy unsaved work')}</Button><Button size="sm" onClick={() => void reloadSaved()} disabled={Boolean(state.activeRequest || state.importing)}>{text('重新读取已保存版本', 'Reload saved version')}</Button></div>}
    {actionNotice && <div className="kv-study-banner" role="status">{actionNotice}</div>}
    {state.notice && <div className="kv-study-banner" role="status">{state.notice}</div>}
    {state.importing && <div className="kv-study-banner" role="status">{text('正在检查并导入材料…', 'Checking and importing material…')}<Button size="sm" onClick={cancelStudyImport}>{text('取消', 'Cancel')}</Button></div>}
    <nav className="kv-study-tabs" role="tablist" aria-label={text('学习面板', 'Study panes')}>
      {(['library', 'reader', 'help'] as Pane[]).map((item, index) => <button id={`study-tab-${item}`} key={item} type="button" role="tab" aria-selected={pane === item} aria-controls={`study-pane-${item}`} tabIndex={pane === item ? 0 : -1} onClick={() => setPane(item)} onKeyDown={(event) => keyTabs(event, item)}>{[text('材料与历史', 'Library'), text('阅读', 'Read'), text('学习助手', 'Help')][index]}</button>)}
    </nav>
    {!state.loaded ? <div className="kv-study-empty" role="status">{text('正在恢复学习记录…', 'Restoring your study space…')}</div> : !state.documents.length ? <div className="kv-study-empty" onDragOver={(event) => event.preventDefault()} onDrop={(event) => { event.preventDefault(); upload(event.dataTransfer.files[0]) }}>
      <div className="kv-study-hero-icon"><BookOpen size={38} strokeWidth={1.3} /></div>
      <span className="kv-study-eyebrow">KIVIO STUDY</span>
      <h2>{text('从卡住的那一页开始', 'Start with the page that has you stuck')}</h2>
      <p>{text('导入讲义、习题或手写笔记。先自己试一试，再要一步提示。', 'Bring a handout, a problem set or a photo of your notes. Try it first, then ask for one hint.')}</p>
      <Button variant="primary" onClick={() => fileInput.current?.click()} disabled={state.importing}><Plus size={16} />{text('打开 PDF 或图片', 'Open a PDF or image')}</Button>
      <small>{text('PDF · PNG · JPEG · WebP，单份不超过 25 MB。也可以拖到这里。', 'PDF · PNG · JPEG · WebP, up to 25 MB each. Or drop a file here.')}</small>
      <div className="kv-study-steps"><span><FileText size={17} />{text('读一页', 'Read a page')}</span><span><Lightbulb size={17} />{text('走一步', 'Take one step')}</span><span><History size={17} />{text('下次接着学', 'Pick up where you left off')}</span></div>
      <p className="kv-study-privacy">{text('材料与笔记保存在此设备的应用存储中。只有点击发送，选中的页面文字或图片才会交给你配置的模型服务。清除应用数据会删除学习记录。', 'Materials and notes are saved in this device’s app storage. Selected page text or images go to your configured model service only when you send. Clearing app data removes this work.')}</p>
    </div> : <div className={`kv-study-layout ${libraryOpen ? '' : 'is-library-closed'}`} data-pane={pane}>
      <aside id="study-pane-library" className="kv-study-library custom-scrollbar" aria-label={text('材料与问题历史', 'Materials and question history')}>
        <div className="kv-study-pane-heading"><h2>{text('我的材料', 'My materials')}</h2><IconButton label={text('收起材料栏', 'Collapse library')} variant="ghost" onClick={() => setLibraryOpen(false)}><PanelLeftClose size={16} /></IconButton></div>
        <div className="kv-study-document-list">{state.documents.map((item) => <button className="kv-study-list-row" type="button" key={item.id} aria-current={item.id === doc?.id ? 'true' : undefined} onClick={() => { openStudyPage(item.id, item.lastPage); setPane('reader') }}><FileText size={16} /><span><strong>{item.name}</strong><small>{text(`第 ${item.lastPage} / ${item.pageCount} 页`, `Page ${item.lastPage} of ${item.pageCount}`)}</small></span></button>)}</div>
        <div className="kv-study-pane-heading"><h2>{text('问题足迹', 'Questions')}</h2><History size={15} /></div>
        {!allTurns.length && <p className="kv-study-muted">{text('每次提问都留在对应页面，随时回来接着想。', 'Each question stays with its page. Come back when you need it.')}</p>}
        {allTurns.map((turn) => <button type="button" key={turn.id} className="kv-study-list-row kv-study-history-row" aria-current={turn.id === state.selectedTurnId ? 'true' : undefined} onClick={() => selectHistory(turn)}><span><small>{text(`第 ${turn.page} 页`, `Page ${turn.page}`)} · {modeNames[turn.mode][zh ? 0 : 1]}</small><strong>{turn.question}</strong><small>{new Date(turn.createdAt).toLocaleDateString(zh ? 'zh-CN' : 'en-US')}{turn.status !== 'complete' && ` · ${turn.status}`}</small></span></button>)}
      </aside>
      <main id="study-pane-reader" className="kv-study-reading" aria-label={text('材料阅读器', 'Material reader')}>
        <div className="kv-study-pane-heading"><div className="kv-study-reader-title">{!libraryOpen && <IconButton label={text('展开材料栏', 'Expand library')} variant="ghost" onClick={() => setLibraryOpen(true)}><PanelLeftOpen size={16} /></IconButton>}<h2 title={doc?.name}>{doc?.name}</h2></div><IconButton label={text('移除当前材料', 'Remove current material')} variant="ghost" disabled={Boolean(state.activeRequest?.documentId === doc?.id)} onClick={() => void remove()}><Trash2 size={15} /></IconButton></div>
        {doc && blobState.id === doc.id && blobState.error && <p role="alert">{blobState.error}</p>}
        {doc && blobState.id === doc.id && blobState.blob ? <StudyReader key={doc.id} blob={blobState.blob} page={doc.lastPage} onPageChange={(next) => { openStudyPage(doc.id, next); setRetryTurn(null) }} region={page?.region ?? null} onRegionChange={(region) => { edit({ region }); setRetryTurn(null) }} onContextChange={handleContext} /> : <p className="kv-study-muted" role="status">{text('正在打开材料…', 'Opening material…')}</p>}
        {doc && page && <details className="kv-study-notes"><summary>{text(`第 ${doc.lastPage} 页笔记`, `Notes for page ${doc.lastPage}`)}{page.notes && ' ·'}</summary><TextArea aria-label={text('本页笔记', 'Page notes')} value={page.notes} onChange={(notes) => edit({ notes })} autoSize={{ minRows: 2, maxRows: 6 }} maxLength={12000} placeholder={text('用自己的话记下关键一步，或者下次还想问的问题', 'Explain the key step in your own words, or leave a question for next time')} /></details>}
      </main>
      <section id="study-pane-help" className="kv-study-help" aria-label={text('学习助手', 'Study helper')}>
        <div className="kv-study-pane-heading"><h2><MessageCircle size={16} />{text('一起想明白', 'Work it through')}</h2><span className="kv-study-page-label">{text(`第 ${doc?.lastPage ?? 1} 页`, `Page ${doc?.lastPage ?? 1}`)}</span></div>
        <div className="kv-study-answers custom-scrollbar">
          {!page?.history.length && <div className="kv-study-help-empty"><Lightbulb size={25} strokeWidth={1.6} /><h3>{text('先给你一步，不急着给答案', 'One step before the answer')}</h3><p>{text('选中一道题或一段话，说说卡在哪里。也可以写出你的思路，让我检查第一处问题。', 'Select a problem or passage and say where you got stuck. Add your attempt to check the first step that needs attention.')}</p></div>}
          {page?.history.map((turn) => <article className="kv-study-turn" key={turn.id} id={`study-turn-${turn.id}`} aria-label={`${modeNames[turn.mode][zh ? 0 : 1]}: ${turn.question}`}>
            <div className="kv-study-question"><span className="kv-study-eyebrow">{modeNames[turn.mode][zh ? 0 : 1]}</span><p>{turn.question}</p>{turn.attempt && <details><summary>{text('我的思路', 'My attempt')}</summary><p>{turn.attempt}</p></details>}</div>
            <div className="kv-study-turn-meta"><Button size="sm" variant="ghost" onClick={() => { if (doc) { openStudyPage(doc.id, turn.page, turn.region ?? null, turn.id); setPane('reader') } }}>{text(`来源：第 ${turn.page} 页${turn.region ? ' · 选区' : ''}`, `Source: page ${turn.page}${turn.region ? ' · region' : ''}`)}</Button><small>{turn.model}</small></div>
            {turn.sourceWarning && <small className="kv-study-muted">{turn.sourceWarning}</small>}
            {turn.answer && (turn.mode === 'solution' ? <details className="kv-study-solution"><summary>{text('展开完整解答', 'Reveal full solution')}</summary><ChatMarkdown content={turn.answer} readOnly /></details> : <ChatMarkdown content={turn.answer} readOnly />)}
            {turn.status === 'streaming' && <p role="status" className="kv-study-muted">{text('正在思考并回答…', 'Thinking and responding…')}</p>}
            {turn.status !== 'complete' && turn.status !== 'streaming' && <div className="kv-study-turn-error" role="status"><span>{turn.error || text('回答已停止，草稿与已生成内容已保留。', 'Stopped. Your draft and partial reply are preserved.')}</span><Button size="sm" onClick={() => prepareRetry(turn)} disabled={Boolean(state.activeRequest)}><RotateCcw size={13} />{text('重试此问题', 'Retry this question')}</Button></div>}
            <details className="kv-study-source"><summary>{text('查看发送的原文', 'View source text sent')}</summary><pre>{turn.sourceText || text('已发送页面图片，没有提取文字。', 'Page image sent; no text extracted.')}</pre>{turn.sourceImageUsed && <small>{text('同时发送了此页或选区的图片。', 'Also included an image of this page or region.')}</small>}</details>
          </article>)}
          <div ref={historyEnd} />
        </div>
        {doc && page && <div className="kv-study-composer">
          <div className="kv-study-compose-heading"><span>{text('问这一页', 'Ask about this page')}</span><small>{page.region ? text('已选区域', 'Selected area') : text('整页上下文', 'Whole-page context')}</small></div>
          <div className="kv-study-modes" role="radiogroup" aria-label={text('帮助方式', 'Help mode')}>
            {MODES.map((item, index) => <button key={item} type="button" role="radio" aria-checked={mode === item} tabIndex={mode === item ? 0 : -1} onClick={() => { setMode(item); setRetryTurn(null) }} onKeyDown={(event) => { const offset = ['ArrowRight', 'ArrowDown'].includes(event.key) ? 1 : ['ArrowLeft', 'ArrowUp'].includes(event.key) ? -1 : 0; if (offset) { event.preventDefault(); const next = MODES[(index + offset + MODES.length) % MODES.length]; setMode(next); setRetryTurn(null); document.getElementById(`study-mode-${next}`)?.focus() } }} id={`study-mode-${item}`}>{modeNames[item][zh ? 0 : 1]}</button>)}
          </div>
          <div className="kv-study-compose-fields custom-scrollbar">
            {state.activeRequest && !activeHere && <p role="status" className="kv-study-muted">{text('另一页正在回答，你可以继续阅读或写草稿。', 'Another page is getting a reply. You can keep reading or drafting.')}</p>}
            {retryTurn ? <div className="kv-panel kv-study-retry"><p>{text('将用原问题、原思路和原文重新回答：', 'Retrying the original question, attempt and source: ')}{retryTurn.question}</p><Button size="sm" variant="ghost" onClick={() => setRetryTurn(null)}>{text('取消重试', 'Cancel retry')}</Button></div> : <>
              <div className="kv-study-compose-field">
                <label htmlFor="study-question" className="kv-study-field-label">{text('关于本页的问题', 'Question about this page')}</label>
                <TextArea id="study-question" aria-required aria-label={text('关于本页的问题', 'Question about this page')} value={page.question} onChange={(question) => edit({ question })} autoSize={{ minRows: 2, maxRows: 6 }} maxLength={6000} placeholder={text('哪一步不明白？例如：这里为什么要换元？', 'Where are you stuck? For example: why change the variable here?')} />
              </div>
              <div className="kv-study-compose-field">
                <div className="kv-study-field-heading"><label htmlFor="study-attempt" className="kv-study-field-label">{text('我的解答或思路', 'My attempt')}</label><small>{mode === 'check' ? text('检查解答时必填', 'Required for checking') : text('可选', 'Optional')}</small></div>
                <TextArea id="study-attempt" aria-required={mode === 'check'} aria-label={text('我的解答或思路', 'My attempt')} value={page.attempt} onChange={(attempt) => edit({ attempt })} autoSize={{ minRows: mode === 'check' ? 3 : 2, maxRows: 6 }} maxLength={12000} placeholder={mode === 'check' ? text('写下你已经做到哪一步，或者你的推导过程', 'Write the steps you tried or the reasoning you used') : text('我试了……，但在这里卡住了', 'I tried… and got stuck when…')} />
              </div>
            </>}
            <details className="kv-study-correction"><summary>{text('补充或修正题目文字', 'Paste or correct problem text')}{page.correctedText.trim() && <span className="kv-study-source-active">{text('已使用', 'In use')}</span>}</summary><p>{text('会替代自动提取文字。扫描件、公式和手写识别可能不准确，请核对；这里不会自动 OCR。', 'Replaces extracted text. Scans, equations and handwriting may be unreadable or inaccurate; check the source. No automatic OCR is performed.')}</p><TextArea aria-label={text('修正后的题目文字', 'Corrected problem text')} value={page.correctedText} onChange={(correctedText) => edit({ correctedText })} autoSize={{ minRows: 3, maxRows: 7 }} maxLength={16000} /></details>
            {mode === 'check' && !page.attempt.trim() && !retryTurn && <small className="kv-study-mode-note">{text('先写下自己的思路，再检查第一处需要调整的地方。', 'Add your thinking first, then check the first step that needs attention.')}</small>}
            {mode === 'solution' && <small className="kv-study-mode-note">{text('完整解答会揭示答案。想继续自己试，可以切回“提示一步”。', 'A full solution reveals the answer. Choose One hint to keep trying yourself.')}</small>}
          </div>
          <div className="kv-study-compose-footer">
            <div className="kv-study-models"><Select ariaLabel={text('模型服务', 'Model provider')} value={providerId} onChange={setProviderId} options={providers.map((item) => ({ value: item.id, label: item.name }))} disabled={!providers.length} /><Select ariaLabel={text('学习模型', 'Study model')} value={model} onChange={setModel} options={(provider?.enabledModels ?? []).map((item) => ({ value: item, label: item }))} disabled={!provider} /></div>
            {(vision || retryTurn?.sourceImageUsed) && <label className="kv-study-vision"><input type="checkbox" checked={sendingImage} disabled={Boolean(retryTurn)} onChange={(event) => setIncludeImage(event.target.checked)} />{text('同时发送页面 / 选区图片', 'Include page / region image')}</label>}
            {!providers.length && <div className="kv-study-no-provider"><span>{text('配置模型后即可提问。仍可阅读和记笔记。', 'Configure a model to ask for help. Reading and notes still work.')}</span><Button size="sm" onClick={onOpenSettings}>{text('打开设置', 'Open settings')}</Button></div>}
            <div className="kv-study-send-row"><small>{text('AI 可能出错，请对照原文检查。', 'AI can make mistakes. Check against the source.')}</small>{state.activeRequest ? <Button onClick={cancelStudyHelp}><Square size={13} />{text('停止回答', 'Stop reply')}</Button> : <Button variant="primary" disabled={!canSend || (!retryTurn && (!page.question.trim() || (mode === 'check' && !page.attempt.trim())))} onClick={() => void send()}><Send size={14} />{retryTurn ? text('重新发送', 'Send retry') : mode === 'solution' ? text('获取完整解答', 'Get full solution') : text('发送', 'Send')}</Button>}</div>
            <p className="kv-study-disclosure">{provider ? text(`发送时，你的问题、思路、此页历史与${sendingImage ? '页面文字及图片' : '页面文字'}将交给 ${provider.name}（${model}）。`, `Sending shares your question, attempt, this page’s history and ${sendingImage ? 'page text and image' : 'page text'} with ${provider.name} (${model}).`) : ''}</p>
            <details className="kv-study-privacy-details"><summary>{text('本机存储与调试信息', 'Local storage and diagnostics')}</summary><p>{text('草稿与笔记保存在此设备。如已启用请求调试，请求内容也会记录在本地调试日志。', 'Drafts and notes stay on this device. If Request Debug is enabled, request content is also recorded in local debug logs.')}</p></details>
          </div>
        </div>}
      </section>
    </div>}
  </section>
}
