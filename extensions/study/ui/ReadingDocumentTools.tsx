import { useEffect, useRef, useState } from 'react'
import { save } from '@tauri-apps/plugin-dialog'
import { Check, Download, FileDown, Image, Languages, Loader2, Settings2, X } from 'lucide-react'
import { Button, IconButton } from '../../sdk/ui'
import { ErrorNotice } from '../../sdk/ui'
import { Select } from '../../sdk/ui'
import { readingOperationsApi, type ReadingExportFormat, type ReadingSession } from '../api/readingOperations'
import { readingSetupApi, type StudySetupStatus } from '../api/readingSetup'
import type { ReadingDocument } from '../api/reading'
import { readingLanguages as languages } from './readingLanguages'

const stages: Record<string, string> = {
  'Parse PDF and Create Intermediate Representation': '正在解析文档结构',
  'Detect Scanned Pages': '正在检查文字层', 'Parse Page Layout': '正在分析版面',
  'Parse Paragraphs': '正在整理段落', 'Translate Paragraphs': '正在翻译段落',
  'Typesetting': '正在排版译文', 'Generate PDF': '正在生成 PDF',
  'Font subsetting': '正在处理字体', 'Save PDF': '正在保存 PDF',
  'Subset font': '正在处理字体', 'Add Fonts': '正在准备字体',
  'DetectScannedFile': '正在检查文字层', 'Parse Formulas and Styles': '正在识别公式与样式',
  'Generate drawing instructions': '正在生成版面',
}

/** A document's tool surface; the backend keeps its job alive when this unmounts. */
export default function ReadingDocumentTools({ doc, page, ready, noteDraft, model, modelControls,
  onConfigure, onPageImage, capturing, session, onTranslate, onCancel, onRead }: {
  doc: ReadingDocument
  page: number
  ready: boolean
  noteDraft: string
  model?: { providerId: string; model: string }
  modelControls: React.ReactNode
  onConfigure: () => void
  onPageImage: () => void
  capturing: boolean
  session: ReadingSession | null
  onTranslate: (providerId: string, model: string, source: string, target: string, page: number | null) => Promise<unknown>
  onCancel: () => Promise<void>
  onRead: (kind: 'source' | 'mono' | 'dual') => void
}) {
  const [panel, setPanel] = useState<'translate' | 'export' | null>(null)
  const [configuring, setConfiguring] = useState(false)
  const [setup, setSetup] = useState<StudySetupStatus | null>(null)
  const translation = session?.translation
  const output = session?.view.latest
  const [source, setSource] = useState('en'), [target, setTarget] = useState('zh')
  const [scope, setScope] = useState('all')
  const [busy, setBusy] = useState(false), [error, setError] = useState(''), [notice, setNotice] = useState('')
  const alive = useRef(true)
  const previousPhase = useRef(translation?.phase)
  const running = translation?.phase === 'running'
  useEffect(() => { alive.current = true; return () => { alive.current = false } }, [])
  useEffect(() => {
    if (previousPhase.current === 'running' && translation?.phase === 'ready') setPanel(null)
    previousPhase.current = translation?.phase
  }, [translation?.phase])
  useEffect(() => {
    if (!panel) return
    let live = true
    void readingSetupApi.status().then(value => { if (live) setSetup(value) }).catch(error => { if (live) setError(String(error)) })
    const escape = (event: KeyboardEvent) => { if (event.key === 'Escape' && !event.defaultPrevented) setPanel(null) }
    window.addEventListener('keydown', escape)
    return () => { live = false; window.removeEventListener('keydown', escape) }
  }, [panel])
  const act = async (action: () => Promise<void>) => {
    setBusy(true); setError(''); setNotice('')
    try { await action() } catch (error) { if (alive.current) setError(String(error)) }
    finally { if (alive.current) setBusy(false) }
  }
  const exportDocument = (format: ReadingExportFormat) => void act(async () => {
    const ext = ['source', 'mono', 'dual'].includes(format) ? 'pdf' : format
    const suffix = format === 'source' ? '' : format === 'mono' ? ' · 译文' : format === 'dual' ? ' · 双语' : ' · 学习笔记'
    const name = doc.name.replace(/[/\\:*?"<>|\n\r]/g, '_').replace(/\.pdf$/i, '')
    const path = await save({ title: '导出文档', defaultPath: `${name}${suffix}.${ext}`, filters: [{ name: ext.toUpperCase(), extensions: [ext] }] })
    if (!path) return
    await readingOperationsApi.export(doc.id, path, format, doc.notes, noteDraft, session?.view.output?.runId || output?.runId)
    if (alive.current) setNotice('已导出到所选位置')
  })
  const toggle = (value: typeof panel) => { setPanel(panel === value ? null : value); setConfiguring(false); setNotice('') }
  const translated = !!output
  const hasNotes = doc.notes.length > 0 || !!noteDraft.trim()
  return <>
    <ErrorNotice message={error} />
    <nav className="reader-document-tools" aria-label="文档快捷工具">
      <div className="tool-group">
        <IconButton label="论文翻译" variant={panel === 'translate' ? 'default' : 'ghost'} size="sm" aria-pressed={panel === 'translate'} onClick={() => toggle('translate')}>
          {running ? <Loader2 size={18} className="reading-spin" /> : <Languages size={18} />}
        </IconButton>
        <IconButton label="将当前页作为图片提问" variant="ghost" size="sm" disabled={!ready || capturing} onClick={onPageImage}>
          {capturing ? <Loader2 size={18} className="reading-spin" /> : <Image size={18} />}
        </IconButton>
        <IconButton label="导出文档与笔记" variant={panel === 'export' ? 'default' : 'ghost'} size="sm" aria-pressed={panel === 'export'} onClick={() => toggle('export')}><Download size={18} /></IconButton>
      </div>
      <IconButton label="Study 组件配置" variant="ghost" size="sm" onClick={onConfigure}><Settings2 size={17} /></IconButton>
    </nav>
    {panel && <aside className="reader-tool-panel custom-scrollbar" aria-label={panel === 'translate' ? '论文翻译设置' : '导出设置'}>
      <div className="reader-tool-heading"><h2>{panel === 'translate' ? '论文翻译' : '导出'}</h2><IconButton label="关闭文档工具" variant="ghost" onClick={() => setPanel(null)}><X size={16} /></IconButton></div>
      {panel === 'translate' ? <>
        <section className="reader-version-section" aria-label="选择阅读版本">
          <h3>阅读版本</h3>
          <div className="reader-version-actions">
            {([{ kind: 'source', label: '原文', name: '阅读原文' }, { kind: 'mono', label: '译文', name: '阅读译文' }, { kind: 'dual', label: '双语对照', name: '双语对照' }] as const).map(version =>
              <Button key={version.kind} size="sm" variant={session?.view.kind === version.kind ? 'default' : 'ghost'} aria-label={version.name} aria-pressed={session?.view.kind === version.kind}
                disabled={busy || !session || (version.kind !== 'source' && !translated)} onClick={() => { onRead(version.kind); setPanel(null) }}>
                {session?.view.kind === version.kind && <Check size={13} />}{version.label}
              </Button>)}
          </div>
          <p className="reader-version-caption">{output ? `${languages.find(l => l.value === output.targetLang)?.label || output.targetLang}译文 · ${output.page ? `第 ${output.page} 页` : '整份文档'}` : '原文可直接阅读，翻译后可在这里切换版本。'}</p>
        </section>
        {!running && <div className="reader-tool-field"><span>翻译模型</span><div className="reader-translation-model">{modelControls}</div></div>}
        {!running && <div className="reader-translation-heading">
          <h3>{translated ? '翻译设置' : '生成译文'}</h3>
          {translated && <Button size="sm" variant="ghost" aria-expanded={configuring} onClick={() => setConfiguring(!configuring)}>{configuring ? '收起设置' : '重新翻译…'}</Button>}
        </div>}
        {!running && (!translated || configuring) && <div className="reader-translation-form">
        <div className="reader-language-pair">
          <div className="reader-tool-field"><span>原文语言</span><Select ariaLabel="原文语言" value={source} onChange={setSource} options={languages} disabled={running || busy} /></div>
          <div className="reader-tool-field"><span>译文语言</span><Select ariaLabel="译文语言" value={target} onChange={setTarget} options={languages} disabled={running || busy} /></div>
        </div>
        <div className="reader-tool-field"><span>翻译范围</span><Select ariaLabel="翻译范围" value={scope} onChange={setScope} disabled={running || busy} options={[{ value: 'page', label: `当前页 · 第 ${page} 页` }, { value: 'all', label: '整份文档' }]} /></div>
          {translation?.error && <p className="reader-tool-description" role="status">{translation.phase === 'cancelled' ? '上次翻译已停止，可以重新开始。' : '上次翻译未完成。'}<Button size="sm" variant="ghost" onClick={() => setError(translation.error || '')}>查看详情</Button></p>}
          {setup && !setup.installed ? <Button variant="primary" onClick={onConfigure}>配置 PDF2zh</Button> : <Button variant="primary" disabled={!setup || !ready || !model?.providerId || !model.model || source === target || busy} onClick={() => void act(async () => {
            await onTranslate(model!.providerId, model!.model, source, target, scope === 'page' ? page : null)
          })}>{busy ? '正在开始…' : translated ? '重新翻译' : '开始翻译'}</Button>}
          {source === target && <p className="reader-tool-description">原文与译文请选择不同语言。</p>}
          <p className="reader-tool-footnote">翻译后自动显示译文。内容由所选模型处理并计费。</p>
        </div>}
        {running && <div className="reader-translation-progress" role="status">
          <progress aria-label="论文翻译进度" max={100} value={translation.percent} />
          <div><span>{stages[translation.stage] || '正在处理文档'}</span><span>{Math.floor(translation.percent)}%</span></div>
          <small>使用 {translation.model}。离开页面后仍会继续。</small>
          <Button size="sm" disabled={busy} onClick={() => void act(() => onCancel())}>停止翻译</Button>
        </div>}
        {translated && !running && !configuring && translation?.error && <p className="reader-tool-description" role="status">上次翻译未完成，已有版本仍可阅读。<Button size="sm" variant="ghost" onClick={() => setError(translation.error || '')}>查看详情</Button></p>}
      </> : <>
        <p className="reader-tool-description">带走原文，或将阅读中的理解整理成文档。</p>
        <div className="reader-export-section"><h3>文档</h3><Button variant="ghost" disabled={busy} onClick={() => exportDocument('source')}><FileDown size={16} />原文 PDF</Button>
          {translated && <><Button variant="ghost" disabled={busy} onClick={() => exportDocument('dual')}>双语 PDF</Button><Button variant="ghost" disabled={busy} onClick={() => exportDocument('mono')}>译文 PDF</Button></>}
        </div>
        <p className="reader-tool-footnote">PDF 包含对应版本的标注，原文件保持不变。</p>
        <div className="reader-export-section"><h3>学习笔记 <span>{doc.notes.length} 条{noteDraft.trim() ? ' · 含草稿' : ''}</span></h3>
          {!hasNotes && <p className="reader-tool-description">先保存一段高亮、回答，或写下笔记。</p>}
          <Button variant="ghost" disabled={busy || !hasNotes} onClick={() => exportDocument('md')}>Markdown <span>.md</span></Button>
          <Button variant="ghost" disabled={busy || !hasNotes || !setup?.pandocInstalled} onClick={() => exportDocument('docx')}>Word <span>.docx</span></Button>
          <Button variant="ghost" disabled={busy || !hasNotes || !setup?.pandocInstalled} onClick={() => exportDocument('epub')}>电子书 <span>.epub</span></Button>
          {setup && !setup.pandocInstalled && <Button size="sm" variant="ghost" onClick={onConfigure}>配置 Pandoc，解锁 Word 与 EPUB</Button>}
        </div>
      </>}
      {busy && panel === 'export' && <p role="status" className="reader-tool-description">正在导出…</p>}
      {notice && <p role="status" className="reader-tool-success"><Check size={14} />{notice}</p>}
    </aside>}
  </>
}
