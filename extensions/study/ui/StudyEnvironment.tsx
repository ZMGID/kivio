import { useEffect, useRef, useState, type ReactNode } from 'react'
import { Check, Settings2, X } from 'lucide-react'
import { readingSetupApi, type StudySetupStatus } from '../api/readingSetup'
import { Button, IconButton } from '../../sdk/ui'
import { ErrorNotice } from '../../sdk/ui'
import { Toggle } from '../../sdk/ui'
import { KivioBlob } from '../../sdk/chat'
import './studyEnvironment.css'

const phases: Record<string, string> = {
  download: '正在下载运行环境',
  python: '正在准备独立 Python 环境',
  packages: '正在下载并安装 PDF2zh',
  assets: '正在准备版面模型与字体',
  verify: '正在检查组件是否可用',
  ready: '配置完成，可以开始学习了',
  error: '配置未完成，可以重试',
}
const pandocPhases: Record<string, string> = { download: '正在下载 Pandoc', extract: '正在解包 Pandoc', verify: '正在检查 Pandoc 是否可用' }
const size = (bytes: number) => `${(bytes / 1024 / 1024).toFixed(1)} MB`

function ComponentOptions({ status, busy, onClose, onSave }: {
  status: StudySetupStatus
  busy: boolean
  onClose: () => void
  onSave: (selection: { pdf2zh: boolean; pandoc: boolean }) => Promise<void>
}) {
  const dialog = useRef<HTMLDialogElement>(null)
  const [selected, setSelected] = useState(status.pdf2zh)
  const [pandoc, setPandoc] = useState(status.pandoc)
  const [error, setError] = useState('')
  useEffect(() => { dialog.current?.showModal() }, [])
  return <dialog ref={dialog} className="kv-modal study-options custom-scrollbar" aria-labelledby="study-options-title"
    onCancel={e => { e.preventDefault(); if (!busy) onClose() }}
    onClick={e => { if (e.target === e.currentTarget && !busy) onClose() }}>
    <div className="study-options-heading">
      <h2 id="study-options-title">配置选项</h2>
      <IconButton label="关闭" variant="ghost" disabled={busy} onClick={onClose}><X size={16} /></IconButton>
    </div>
    <p className="study-options-intro">选择要安装的组件，之后也可以随时补装。</p>
    <div className="study-component">
      <div className="study-component-heading">
        <div><h3 id="study-pdf2zh-label">PDF2zh 论文翻译</h3><span>{status.installed ? '已安装' : '待安装'} · v{status.version}</span></div>
        <Toggle checked={selected} disabled={busy} onChange={setSelected} ariaLabel="安装 PDF2zh 论文翻译" />
      </div>
      <p>翻译整篇论文，保留公式与排版，生成译文或双语 PDF。</p>
      <details className="study-dependencies" aria-label="必要依赖">
        <summary>{selected ? '随组件自动配置' : '未选择，不下载依赖'}</summary>
        <ul><li>独立 Python 运行环境</li><li>PDF2zh 与 BabelDOC</li><li>版面分析模型、字体和字符映射</li></ul>
      </details>
    </div>
    <div className="study-component">
      <div className="study-component-heading">
        <div><h3>Pandoc 文档转换</h3><span>{status.pandocInstalled ? '已安装' : '待安装'} · v{status.pandocVersion}{status.pandocDownloadBytes && !status.pandocInstalled ? ` · 下载 ${size(status.pandocDownloadBytes)}` : ''}</span></div>
        <Toggle checked={pandoc} disabled={busy} onChange={setPandoc} ariaLabel="安装 Pandoc 文档转换" />
      </div>
      <p>提供 Word、EPUB、Markdown 等文档格式转换能力。</p>
      <div className="study-dependencies">独立组件，无需 Python。PDF 导出还需要额外的排版引擎，本次不安装。</div>
    </div>
    <p className="study-options-note">{!selected && !pandoc ? '不安装也可以阅读 PDF、管理文献和记笔记。' : '只安装勾选且尚未就绪的组件，配置时显示实际进度。'}<br />取消选择不会删除已安装的文件。PDF2zh 的翻译模型需另行配置。</p>
    {error && <p role="alert" className="study-options-error">{error}</p>}
    <div className="study-options-actions"><Button disabled={busy} onClick={onClose}>取消</Button><Button variant="primary" disabled={busy} onClick={() => {
      setError('')
      void onSave({ pdf2zh: selected, pandoc }).catch(e => setError(String(e)))
    }}>{busy ? '正在保存…' : '确定'}</Button></div>
  </dialog>
}

/** The backend owns installation; leaving this surface does not interrupt its job. */
export default function StudyEnvironment({ children, headerControls }: {
  children: (openSetup: () => void) => ReactNode
  headerControls?: ReactNode
}) {
  const [status, setStatus] = useState<StudySetupStatus | null>(null)
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const [options, setOptions] = useState(false)
  const [managing, setManaging] = useState(false)
  const revision = useRef(0)
  const alive = useRef(true)
  useEffect(() => { alive.current = true; return () => { alive.current = false } }, [])
  const progress = status?.progress
  const running = !!progress && progress.phase !== 'ready' && progress.phase !== 'error'
  useEffect(() => {
    let live = true
    let timer: ReturnType<typeof setTimeout> | undefined
    const refresh = async () => {
      const current = revision.current
      try {
        const next = await readingSetupApi.status()
        if (!live || current !== revision.current) return
        setStatus(next)
        setError('')
      } catch (e) { if (live && current === revision.current) setError(String(e)) }
      if (live && running) timer = setTimeout(() => void refresh(), 1000)
    }
    void refresh()
    return () => { live = false; clearTimeout(timer) }
  }, [running])
  const act = async (action: () => Promise<StudySetupStatus>, reportError = true) => {
    ++revision.current
    setBusy(true); setError('')
    try {
      const next = await action()
      if (alive.current) setStatus(next)
    } catch (e) { if (alive.current && reportError) setError(String(e)); throw e }
    finally { if (alive.current) setBusy(false) }
  }
  const enter = () => void act(() => readingSetupApi.save({ pdf2zh: status!.pdf2zh, pandoc: status!.pandoc }, true))
    .then(() => { if (alive.current) setManaging(false) }).catch(() => {})
  if (status?.acknowledged && !managing) return <>{children(() => { setManaging(true); setOptions(!running) })}</>
  const allReady = !!status && (!status.pdf2zh || status.installed) && (!status.pandoc || status.pandocInstalled)
  const ready = allReady && !running
  const selectedNames = status ? [status.pdf2zh ? 'PDF2zh' : '', status.pandoc ? 'Pandoc' : ''].filter(Boolean).join('、') : ''
  const anyInstalled = !!status && (status.installed || status.pandocInstalled)
  const downloaded = progress?.phase === 'download' && !!progress.total
  const message = progress ? (progress.component === 'pandoc' ? pandocPhases[progress.phase] : undefined) || phases[progress.phase] : allReady && selectedNames ? phases.ready : status ? '准备你的学习空间' : '正在检查配置…'
  return <div className="study-setup">
    <ErrorNotice message={error || progress?.error} actionLabel={!status ? '重试' : undefined} onAction={() => { void act(readingSetupApi.status).catch(() => {}) }} />
    <header className="study-setup-header" data-tauri-drag-region>{headerControls}
      {status?.acknowledged && <Button variant="ghost" onClick={() => setManaging(false)}>返回 Study</Button>}
    </header>
    <main className="study-setup-main custom-scrollbar">
      <section className="study-setup-content" aria-labelledby="study-setup-title">
        <div className="study-setup-brand" data-momo-region>
          <KivioBlob size={88} mood={running ? 'work' : 'idle'} />
          <h1 id="study-setup-title">Kivio Study</h1>
        </div>
        <p className="study-setup-description">为论文阅读与学习，准备好所需的工具。</p>
        <div className="study-setup-progress">
          <progress aria-label="Study 配置进度" max={downloaded ? progress!.total! : 1}
            value={running ? downloaded ? progress!.downloaded : undefined : allReady && !!selectedNames ? 1 : 0} />
          <div className="study-setup-status" role="status" aria-live="polite"><span>{message}</span><span>
            {running ? downloaded ? `${size(progress!.downloaded)} / ${size(progress!.total!)}` : `${progress!.step} / ${progress!.steps}` : allReady && selectedNames ? <Check size={15} aria-label="组件已就绪" /> : ''}
          </span></div>
        </div>
        <div className="study-setup-actions">
          <Button variant="ghost" disabled={!status || busy || running} onClick={() => setOptions(true)}><Settings2 size={15} />配置选项</Button>
          <Button variant="primary" disabled={!status || busy || running} onClick={ready ? enter : () => void act(() => readingSetupApi.start()).catch(() => {})}>
            {running ? '正在配置…' : busy ? '请稍候…' : ready ? '进入 Kivio Study' : progress?.phase === 'error' ? '重试配置' : '开始配置'}
          </Button>
        </div>
        <p className="study-setup-footnote">{running ? '首次配置可能需要几分钟。离开页面后仍会继续。' : !selectedNames && status ? '已跳过组件安装，基础阅读功能可直接使用。' : ready ? `${selectedNames} 已就绪。` : status ? `所选组件：${selectedNames}。可在配置选项中调整。` : '正在检查已安装的组件。'}</p>
        {anyInstalled && selectedNames && !running && <Button variant="ghost" size="sm" disabled={busy} onClick={() => void act(() => readingSetupApi.start(true)).catch(() => {})}>检查并修复组件</Button>}
      </section>
    </main>
    {options && status && <ComponentOptions status={status} busy={busy} onClose={() => setOptions(false)} onSave={async selected => {
      await act(() => readingSetupApi.save(selected), false)
      if (alive.current) setOptions(false)
    }} />}
  </div>
}
