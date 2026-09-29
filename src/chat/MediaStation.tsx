import { useEffect, useRef, useState } from 'react'
import { open, save } from '@tauri-apps/plugin-dialog'
import { Clapperboard, Download, ImagePlus, Images, LoaderCircle, Play, RefreshCw, Settings2, Sparkles, X } from 'lucide-react'
import { mediaStationApi, type MediaJob, type MediaKind, type MediaRequest } from '../api/mediaStation'
import { getSettingsCached } from '../api/settingsCache'
import { isTauriRuntime, type ModelProvider } from '../api/tauri'
import { Button, IconButton } from '../components/Button'
import { Input, SuggestInput, Label, Select, TextArea } from '../settings/public/controls'
import { useLang } from '../components/i18n'
import { resolveModelInfo } from '../data/modelMatching'
import './MediaStation.css'

// Unsubmitted form state survives navigation in this window; credentials remain in settings.
let draft: MediaRequest = { kind: 'image', providerId: '', model: '', prompt: '', aspectRatio: '1:1', duration: 5, referencePaths: [] }

function OutputPreview({ job, index }: { job: MediaJob; index: number }) {
  const [url, setUrl] = useState('')
  const [error, setError] = useState('')
  const output = job.outputs[index]
  useEffect(() => {
    let active = true
    let objectUrl = ''
    setUrl(''); setError('')
    mediaStationApi.read(job.id, index).then((bytes) => {
      if (!active) return
      objectUrl = URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type: output.mimeType }))
      setUrl(objectUrl)
    }).catch((e) => { if (active) setError(String(e)) })
    return () => { active = false; if (objectUrl) URL.revokeObjectURL(objectUrl) }
  }, [job.id, index, output.mimeType])
  if (error) return <p role="alert">{error}</p>
  if (!url) return <LoaderCircle className="animate-spin" aria-label="Loading" />
  return output.mimeType.startsWith('video/')
    ? <video src={url} controls preload="metadata" />
    : <img src={url} alt={job.request.prompt} />
}

export function MediaStation({ onOpenSettings }: { onOpenSettings: () => void }) {
  const zh = useLang() === 'zh'
  const text = (cn: string, en: string) => zh ? cn : en
  const desktop = isTauriRuntime()
  const [form, setForm] = useState<MediaRequest>(() => draft)
  const [providers, setProviders] = useState<ModelProvider[]>([])
  const [jobs, setJobs] = useState<MediaJob[]>([])
  const [selectedId, setSelectedId] = useState('')
  const [filter, setFilter] = useState('all')
  const [query, setQuery] = useState('')
  const [error, setError] = useState('')
  const [loading, setLoading] = useState(desktop)
  const [submitting, setSubmitting] = useState(false)
  const [refresh, setRefresh] = useState(0)
  const sending = useRef(false)
  const mounted = useRef(true)
  const selected = jobs.find((j) => j.id === selectedId)
  const provider = providers.find((p) => p.id === form.providerId)
  const suggestedModels = provider?.enabledModels.filter((model) => form.kind === 'image'
    ? resolveModelInfo(model, provider.modelOverrides, provider).capabilities?.imageGeneration
    : /grok.*video/i.test(model)) ?? []
  const running = jobs.filter((j) => j.status === 'running').length
  const visible = jobs.filter((j) => (filter === 'all' || j.request.kind === filter) && j.request.prompt.toLowerCase().includes(query.toLowerCase()))
  const statusLabel = (job: MediaJob) => ({ running: text('生成中', 'Generating'), completed: text('已完成', 'Completed'), failed: text('失败', 'Failed'), cancelled: text('已停止等待', 'Stopped waiting'), interrupted: text('已中断', 'Interrupted') })[job.status]

  function update(patch: Partial<MediaRequest>) {
    setForm((previous) => { draft = { ...previous, ...patch }; return draft })
  }
  useEffect(() => {
    mounted.current = true
    return () => { mounted.current = false }
  }, [])
  useEffect(() => {
    if (!desktop) return
    let active = true
    getSettingsCached().then((settings) => {
      if (!active) return
      setProviders(settings.providers.filter((p) => p.enabled))
      if (!draft.providerId) {
        const initial = settings.defaultModels.imageGeneration
        if (initial.providerId) update({ providerId: initial.providerId, model: initial.model })
      }
    }).catch((e) => { if (active) setError(String(e)) })
    return () => { active = false }
  }, [desktop])
  useEffect(() => {
    if (!desktop) return
    let active = true
    let timer: ReturnType<typeof setTimeout>
    const load = async () => {
      try {
        const result = await mediaStationApi.list()
        if (!active) return
        setJobs(result)
        if (result.some((j) => j.status === 'running')) timer = setTimeout(() => void load(), 2000)
      } catch (e) { if (active) setError(String(e)) }
      finally { if (active) setLoading(false) }
    }
    void load()
    return () => { active = false; clearTimeout(timer) }
  }, [desktop, refresh])

  function changeKind(kind: MediaKind) {
    update({ kind, model: '', referencePaths: [], aspectRatio: kind === 'image' ? '1:1' : '16:9' })
  }
  async function generate() {
    if (sending.current) return
    sending.current = true; setSubmitting(true); setError('')
    try {
      const job = await mediaStationApi.start({ ...form, prompt: form.prompt.trim(), model: form.model.trim() })
      if (!mounted.current) return
      setSelectedId(job.id); setFilter('all'); setQuery(''); setRefresh((n) => n + 1)
    } catch (e) { if (mounted.current) setError(String(e)) }
    finally { sending.current = false; if (mounted.current) setSubmitting(false) }
  }
  async function pickReferences() {
    try {
      const paths = await open({ multiple: form.kind === 'image', filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'webp'] }] })
      if (!paths || !mounted.current) return
      const next = [...new Set([...form.referencePaths, ...(Array.isArray(paths) ? paths : [paths])])]
      if (next.length > (form.kind === 'image' ? 4 : 1)) throw new Error(text('图片最多 4 张参考图，视频最多 1 张首帧。', 'Use up to 4 image references or 1 video first frame.'))
      update({ referencePaths: next })
    } catch (e) { if (mounted.current) setError(String(e)) }
  }
  async function exportOutput(job: MediaJob, index: number) {
    try {
      const destination = await save({ defaultPath: job.outputs[index].name })
      if (destination) await mediaStationApi.export(job.id, index, destination)
    } catch (e) { if (mounted.current) setError(String(e)) }
  }
  async function cancel(job: MediaJob) {
    try { await mediaStationApi.cancel(job.id); if (mounted.current) setRefresh((n) => n + 1) }
    catch (e) { if (mounted.current) setError(String(e)) }
  }
  async function handleFirstFrame(job: MediaJob, index: number) {
    try {
      const path = await mediaStationApi.reference(job.id, index)
      if (mounted.current) update({ kind: 'video', model: '', prompt: '', referencePaths: [path], aspectRatio: job.request.aspectRatio })
    } catch (e) { if (mounted.current) setError(String(e)) }
  }

  return <section className="kv kv-content kv-media">
    <header className="kv-page-header">
      <h1 className="kv-page-title flex items-center gap-2"><Clapperboard size={20} />{text('媒体站', 'Media studio')}</h1>
      <Button size="sm" variant="ghost" onClick={onOpenSettings}><Settings2 size={15} />{text('模型设置', 'Model settings')}</Button>
    </header>
    <div className="kv-scroll custom-scrollbar kv-media-workspace">
      {error && <div className="kv-panel kv-media-notice" role="alert">{error}<IconButton label={text('关闭提示', 'Dismiss')} onClick={() => setError('')}><X size={14} /></IconButton></div>}
      <form className="kv-panel kv-media-composer" onSubmit={(e) => { e.preventDefault(); void generate() }}>
        <div className="kv-media-composer-heading">
          <div className="kv-media-tabs" role="group" aria-label={text('生成类型', 'Media type')}>
            <Button variant={form.kind === 'image' ? 'default' : 'ghost'} aria-pressed={form.kind === 'image'} onClick={() => changeKind('image')}><Images size={16} />{text('生图', 'Image')}</Button>
            <Button variant={form.kind === 'video' ? 'default' : 'ghost'} aria-pressed={form.kind === 'video'} onClick={() => changeKind('video')}><Play size={16} />{text('生视频', 'Video')}</Button>
          </div>
          <span className="kv-field-hint kv-media-format-note">{form.kind === 'image' ? text('文字或图片 → 图片', 'Text or image → Image') : text('文字或首帧 → 视频', 'Text or first frame → Video')}</span>
        </div>
        <label className="kv-media-field kv-media-prompt-field">
          <span className="sr-only">{text('创作描述', 'Prompt')}</span>
          <TextArea value={form.prompt} onChange={(prompt) => update({ prompt })} rows={4}
            placeholder={form.kind === 'image' ? text('你想看见怎样的画面？描述主体、光线、风格，或从一张参考图开始…', 'What would you like to see? Describe the subject, light and style…') : text('让画面如何运动？描述动作、镜头与节奏，或添加一张首帧…', 'Describe the motion, camera and rhythm, or add a first frame…')} />
        </label>
        {form.referencePaths.length > 0 && <div className="kv-media-references">
          {form.referencePaths.map((path) => <div className="kv-chip kv-media-reference-file" key={path}>
            <ImagePlus size={14} /><span title={path}>{path.split(/[\\/]/).pop()}</span>
            <IconButton size="xs" label={text('移除参考图', 'Remove reference')} onClick={() => update({ referencePaths: form.referencePaths.filter((p) => p !== path) })}><X size={13} /></IconButton>
          </div>)}
        </div>}
        <div className={`kv-media-parameters ${form.kind === 'video' ? 'has-duration' : ''}`}>
          <div className="kv-media-field"><Label>{text('供应商', 'Provider')}</Label>
            <Select ariaLabel={text('供应商', 'Provider')} value={form.providerId} options={providers.map((p) => ({ value: p.id, label: p.name }))} onChange={(providerId) => update({ providerId, model: '' })} />
          </div>
          <div className="kv-media-field"><Label>{text('模型', 'Model')}</Label>
            <SuggestInput ariaLabel={text('模型', 'Model')} value={form.model} onChange={(model) => update({ model })}
              options={suggestedModels.map((model) => ({ value: model, label: model }))}
              placeholder={form.kind === 'image' ? text('选择或输入模型', 'Choose or enter a model') : 'grok-imagine-video'} />
          </div>
          <div className="kv-media-field"><Label>{text('画幅', 'Aspect ratio')}</Label>
            <Select ariaLabel={text('画幅', 'Aspect ratio')} value={form.aspectRatio} onChange={(aspectRatio) => update({ aspectRatio })} options={['1:1', '16:9', '9:16', '4:3', '3:4'].map((value) => ({ value, label: value }))} />
          </div>
          {form.kind === 'video' && <div className="kv-media-field"><Label>{text('时长', 'Duration')}</Label>
            <Select ariaLabel={text('时长', 'Duration')} value={String(form.duration)} onChange={(duration) => update({ duration: Number(duration) })} options={[5, 10, 15].map((n) => ({ value: String(n), label: `${n} ${text('秒', 'sec')}` }))} />
          </div>}
        </div>
        <div className="kv-media-composer-footer">
          <div className="kv-media-attach">
            <Button size="sm" variant="ghost" onClick={() => void pickReferences()} disabled={!desktop}><ImagePlus size={16} />{form.kind === 'image' ? text('参考图', 'References') : text('首帧图片', 'First frame')}</Button>
            <span className="kv-field-hint">{form.kind === 'image' ? text('最多 4 张', 'Up to 4 images') : text('1 张图片', 'One image')}</span>
          </div>
          <Button variant="primary" type="submit" disabled={!desktop || submitting || !provider || !form.model.trim() || !form.prompt.trim() || form.prompt.length > 8000 || running >= 3}>
            {submitting ? <LoaderCircle size={16} className="animate-spin" /> : <Sparkles size={16} />}{text('开始生成', 'Generate')}
          </Button>
        </div>
      </form>
      <div className="kv-field-hint kv-media-composer-caption">
        {!desktop && <span>{text('预览模式 · 请在桌面端生成', 'Preview · Generate in the desktop app')}</span>}
        <span>{form.kind === 'video' ? text('xAI 兼容视频 · 720p', 'xAI-compatible video · 720p') : text('每次生成 1 张 · 按供应商计费', 'One image per request · Provider charges apply')}</span>
        <span>{text('结果自动保存在本机', 'Saved to your device')}</span>
      </div>
      <div className="kv-media-library">
        <div className="kv-media-library-heading"><h2 className="kv-row-label">{text('创作记录', 'Creations')} <small className="kv-tag">{jobs.length}</small></h2><IconButton label={text('刷新记录', 'Refresh')} onClick={() => setRefresh((n) => n + 1)} disabled={!desktop}><RefreshCw size={15} /></IconButton></div>
        <div className="kv-media-library-tools"><Select ariaLabel={text('筛选类型', 'Filter type')} value={filter} onChange={setFilter} options={[{ value: 'all', label: text('全部', 'All') }, { value: 'image', label: text('图片', 'Images') }, { value: 'video', label: text('视频', 'Videos') }]} /><Input aria-label={text('搜索描述', 'Search prompts')} value={query} onChange={setQuery} placeholder={text('搜索创作描述…', 'Search prompts…')} /></div>
        {selected && <article className="kv-panel kv-media-detail">
          <div className="kv-row-desc kv-media-detail-header"><span>{statusLabel(selected)} · {selected.request.model} · {selected.request.aspectRatio}</span><IconButton label={text('收起预览', 'Close preview')} onClick={() => setSelectedId('')}><X size={15} /></IconButton></div>
          {selected.status === 'running' && <div className="kv-row-desc kv-media-pending"><LoaderCircle className="animate-spin" size={26} /><p>{text('正在生成，稍后回来也可以。', 'Generating. You can come back later.')}</p><Button size="sm" onClick={() => void cancel(selected)}>{text('停止等待', 'Stop waiting')}</Button><small>{text('供应商可能继续生成并计费。', 'The provider may continue and charge for this task.')}</small></div>}
          {selected.error && <p role="status" className="kv-panel kv-media-job-error">{selected.error}</p>}
          {selected.outputs.map((output, index) => <div key={output.name}><div className="kv-media-preview"><OutputPreview job={selected} index={index} /></div><div className="kv-media-output-actions"><Button size="sm" onClick={() => void exportOutput(selected, index)}><Download size={14} />{text('另存为', 'Save as')}</Button>{selected.request.kind === 'image' && <Button size="sm" onClick={() => void handleFirstFrame(selected, index)}><Play size={14} />{text('用作视频首帧', 'Use as first frame')}</Button>}</div></div>)}
          <p className="kv-media-prompt">{selected.request.prompt}</p>
          <Button size="sm" variant="ghost" onClick={() => update(selected.request)}><RefreshCw size={14} />{text('复用参数', 'Reuse settings')}</Button>
        </article>}
        {loading ? <p role="status">{text('正在读取记录…', 'Loading history…')}</p> : visible.length === 0 ? <div className="kv-row-desc kv-media-empty">
          <Images size={25} strokeWidth={1.25} />
          <h3 className="kv-row-label">{jobs.length ? text('没有匹配的作品', 'No matching creations') : text('作品会出现在这里', 'Your creations will appear here')}</h3>
          <p className="kv-row-desc">{jobs.length ? text('试试其他关键词或类型。', 'Try another keyword or type.') : text('从上方写下第一段描述，或试试这些灵感。', 'Write your first prompt above, or try an idea below.')}</p>
          {!jobs.length && <div className="kv-media-ideas">
            {[text('产品摄影', 'Product photo'), text('电影感场景', 'Cinematic scene'), text('插画海报', 'Illustrated poster')].map((label, i) =>
              <Button key={label} size="sm" onClick={() => update({ prompt: [
                text('晨光中的玻璃花瓶，简洁背景，柔和阴影，产品摄影。', 'A glass vase in morning light, minimal background, soft shadows, product photography.'),
                text('夕阳下的海岸，暖金色光线，宽阔构图，电影质感。', 'A coastline at sunset, warm golden light, wide composition, cinematic atmosphere.'),
                text('以森林与月亮为主题的插画海报，深蓝与银白配色，留出标题空间。', 'An illustrated forest and moon poster in midnight blue and silver, with room for a title.')
              ][i] })}>{label}</Button>)}
          </div>}
        </div> : <div className="kv-media-grid">{visible.map((job) => <button type="button" key={job.id} className="kv-media-card" aria-pressed={selectedId === job.id} onClick={() => setSelectedId(job.id)}><div className="kv-media-cover">{job.outputs[0]?.preview ? <img src={job.outputs[0].preview} alt="" loading="lazy" /> : job.status === 'running' ? <LoaderCircle className="animate-spin" size={25} /> : job.request.kind === 'image' ? <Images size={28} /> : <Play size={28} />}<span>{statusLabel(job)}</span></div><strong>{job.request.prompt}</strong><small>{job.request.aspectRatio} · {new Date(job.createdAt).toLocaleString(zh ? 'zh-CN' : 'en-US', { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })}</small></button>)}</div>}
      </div>
    </div>
  </section>
}
