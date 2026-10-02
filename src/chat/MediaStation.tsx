import { useEffect, useRef, useState } from 'react'
import { open, save } from '@tauri-apps/plugin-dialog'
import { Camera, Clapperboard, Download, ImagePlus, Images, LoaderCircle, Palette, Play, RefreshCw, Search, Settings2, Sparkles, X } from 'lucide-react'
import { mediaStationApi, type MediaJob, type MediaKind, type MediaRequest } from '../api/mediaStation'
import { getSettingsCached } from '../api/settingsCache'
import { isTauriRuntime, type ModelProvider } from '../api/tauri'
import { Button, IconButton } from '../components/Button'
import { Input, Select, TextArea } from '../settings/public/controls'
import { useLang } from '../components/i18n'
import { resolveModelInfo } from '../data/modelMatching'
import './market/market.css'
import './MediaStation.css'

/** Enabled models of a provider that can generate the given media kind, by the model library capability
 *  (users can toggle it per model). Mirrors the backend gate: media requests need an API key (no account
 *  OAuth) and an OpenAI-compatible or Gemini format; video additionally needs a vendor protocol it knows. */
function mediaModels(provider: ModelProvider, kind: MediaKind): string[] {
  if (provider.request?.oauth || provider.apiFormat === 'anthropic_messages') return []
  if (kind === 'video' && provider.apiFormat === 'gemini') return []
  return provider.enabledModels.filter((model) => {
    const capabilities = resolveModelInfo(model, provider.modelOverrides, provider).capabilities
    return kind === 'image' ? capabilities?.imageGeneration : capabilities?.videoGeneration
  })
}

// Unsubmitted form state survives navigation in this window; credentials remain in settings.
let draft: MediaRequest = { kind: 'image', providerId: '', model: '', prompt: '', aspectRatio: '1:1', duration: 5, referencePaths: [] }

const IDEAS = [
  { icon: Camera, title: ['产品摄影', 'Product photo'], prompt: ['晨光中的玻璃花瓶，简洁背景，柔和阴影，产品摄影。', 'A glass vase in morning light, minimal background, soft shadows, product photography.'] },
  { icon: Clapperboard, title: ['电影感场景', 'Cinematic scene'], prompt: ['夕阳下的海岸，暖金色光线，宽阔构图，电影质感。', 'A coastline at sunset, warm golden light, wide composition, cinematic atmosphere.'] },
  { icon: Palette, title: ['插画海报', 'Illustrated poster'], prompt: ['以森林与月亮为主题的插画海报，深蓝与银白配色，留出标题空间。', 'An illustrated forest and moon poster in midnight blue and silver, with room for a title.'] },
] as const

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
  // 只列出能生成当前类型的供应商和模型。
  const capableProviders = providers.filter((p) => mediaModels(p, form.kind).length > 0)
  const provider = capableProviders.find((p) => p.id === form.providerId)
  const modelChoices = provider ? mediaModels(provider, form.kind) : []
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

  // 当前供应商 / 模型不支持所选类型（切换类型、复用旧任务、设置变更）时，落到第一个可用项。
  const fallbackProvider = provider ?? capableProviders[0]
  const fallbackModel = fallbackProvider && (mediaModels(fallbackProvider, form.kind).includes(form.model) ? form.model : mediaModels(fallbackProvider, form.kind)[0])
  useEffect(() => {
    if (!fallbackProvider) return
    if (fallbackProvider.id !== form.providerId || fallbackModel !== form.model) update({ providerId: fallbackProvider.id, model: fallbackModel ?? '' })
  }, [fallbackProvider, fallbackModel, form.providerId, form.model])

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
  async function resume(job: MediaJob) {
    try { await mediaStationApi.resume(job.id); if (mounted.current) setRefresh((n) => n + 1) }
    catch (e) { if (mounted.current) setError(String(e)) }
  }
  async function handleFirstFrame(job: MediaJob, index: number) {
    try {
      const path = await mediaStationApi.reference(job.id, index)
      if (mounted.current) update({ kind: 'video', model: '', prompt: '', referencePaths: [path], aspectRatio: job.request.aspectRatio })
    } catch (e) { if (mounted.current) setError(String(e)) }
  }

  const canGenerate = desktop && !submitting && Boolean(provider) && Boolean(form.model.trim()) && Boolean(form.prompt.trim()) && form.prompt.length <= 8000 && running < 3

  return <section className="kv kv-content kv-media">
    <header className="kv-page-header kv-media-header">
      <div className="kv-media-title">
        <h1 className="kv-page-title">{text('媒体站', 'Media studio')}</h1>
        <div className="kv-plugin-segments" role="group" aria-label={text('生成类型', 'Media type')}>
          {([['image', text('生图', 'Image')], ['video', text('生视频', 'Video')]] as const).map(([kind, label]) =>
            <button key={kind} type="button" className="kv-plugin-segment" aria-pressed={form.kind === kind} aria-current={form.kind === kind ? 'page' : undefined} onClick={() => changeKind(kind)}>{label}</button>)}
        </div>
      </div>
      <Button size="sm" variant="ghost" onClick={onOpenSettings}><Settings2 size={15} />{text('模型设置', 'Model settings')}</Button>
    </header>
    <div className="kv-scroll custom-scrollbar kv-media-workspace">
      <div className="kv-media-library">
        <div className="kv-media-library-heading">
          <h2>{text('创作记录', 'Creations')}{jobs.length > 0 && <span className="kv-media-count">{jobs.length}</span>}</h2>
          <div className="kv-media-library-tools">
            <div className="kv-plugin-segments" role="group" aria-label={text('筛选类型', 'Filter type')}>
              {([['all', text('全部', 'All')], ['image', text('图片', 'Images')], ['video', text('视频', 'Videos')]] as const).map(([value, label]) =>
                <button key={value} type="button" className="kv-plugin-segment" aria-pressed={filter === value} aria-current={filter === value ? 'page' : undefined} onClick={() => setFilter(value)}>{label}</button>)}
            </div>
            <label className="kv-media-search">
              <Search size={14} aria-hidden="true" />
              <Input aria-label={text('搜索描述', 'Search prompts')} value={query} onChange={setQuery} placeholder={text('搜索创作描述…', 'Search prompts…')} />
            </label>
            <IconButton size="sm" variant="ghost" label={text('刷新记录', 'Refresh')} onClick={() => setRefresh((n) => n + 1)} disabled={!desktop}><RefreshCw size={15} /></IconButton>
          </div>
        </div>
        {selected && <article className="kv-panel kv-media-detail">
          <div className="kv-row-desc kv-media-detail-header"><span>{statusLabel(selected)} · {selected.request.model} · {selected.request.aspectRatio}</span><IconButton label={text('收起预览', 'Close preview')} onClick={() => setSelectedId('')}><X size={15} /></IconButton></div>
          {selected.status === 'running' && <div className="kv-row-desc kv-media-pending"><LoaderCircle className="animate-spin" size={26} /><p>{text('正在生成，稍后回来也可以。', 'Generating. You can come back later.')}</p><Button size="sm" onClick={() => void cancel(selected)}>{text('停止等待', 'Stop waiting')}</Button><small>{text('供应商可能继续生成并计费。', 'The provider may continue and charge for this task.')}</small></div>}
          {selected.error && <p role="status" className="kv-panel kv-media-job-error">{selected.error}</p>}
          {selected.providerTaskId && selected.status !== 'running' && selected.status !== 'completed' && <Button size="sm" onClick={() => void resume(selected)}><RefreshCw size={14} />{text('继续获取结果', 'Fetch result again')}</Button>}
          {selected.outputs.map((output, index) => <div key={output.name}><div className="kv-media-preview"><OutputPreview job={selected} index={index} /></div><div className="kv-media-output-actions"><Button size="sm" onClick={() => void exportOutput(selected, index)}><Download size={14} />{text('另存为', 'Save as')}</Button>{selected.request.kind === 'image' && <Button size="sm" onClick={() => void handleFirstFrame(selected, index)}><Play size={14} />{text('用作视频首帧', 'Use as first frame')}</Button>}</div></div>)}
          <p className="kv-media-prompt">{selected.request.prompt}</p>
          <Button size="sm" variant="ghost" onClick={() => update(selected.request)}><RefreshCw size={14} />{text('复用参数', 'Reuse settings')}</Button>
        </article>}
        {loading ? <p role="status" className="kv-field-hint">{text('正在读取记录…', 'Loading history…')}</p> : visible.length === 0 ? <div className="kv-media-empty">
          <span className="kv-media-empty-icon"><Images size={22} strokeWidth={1.5} /></span>
          <h3>{jobs.length ? text('没有匹配的作品', 'No matching creations') : text('作品会出现在这里', 'Your creations will appear here')}</h3>
          <p>{jobs.length ? text('试试其他关键词或类型。', 'Try another keyword or type.') : text('在下方写下第一段描述，或试试这些灵感。', 'Write your first prompt below, or try one of these ideas.')}</p>
          {!jobs.length && <div className="kv-media-ideas">
            {IDEAS.map(({ icon: Icon, title, prompt }) =>
              <button key={title[0]} type="button" className="kv-media-idea" onClick={() => update({ prompt: text(prompt[0], prompt[1]) })}>
                <Icon size={16} strokeWidth={1.75} />
                <strong>{text(title[0], title[1])}</strong>
                <span>{text(prompt[0], prompt[1])}</span>
              </button>)}
          </div>}
        </div> : <div className="kv-media-grid">{visible.map((job) => <button type="button" key={job.id} className="kv-media-card" aria-pressed={selectedId === job.id} onClick={() => setSelectedId(job.id)}><div className="kv-media-cover">{job.outputs[0]?.preview ? <img src={job.outputs[0].preview} alt="" loading="lazy" /> : job.status === 'running' ? <LoaderCircle className="animate-spin" size={25} /> : job.request.kind === 'image' ? <Images size={28} /> : <Play size={28} />}<span>{statusLabel(job)}</span></div><strong>{job.request.prompt}</strong><small>{job.request.aspectRatio} · {new Date(job.createdAt).toLocaleString(zh ? 'zh-CN' : 'en-US', { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })}</small></button>)}</div>}
      </div>
    </div>
    <div className="kv-media-dock">
      {error && <div className="kv-panel kv-media-notice" role="alert">{error}<IconButton label={text('关闭提示', 'Dismiss')} onClick={() => setError('')}><X size={14} /></IconButton></div>}
      <form className="kv-media-composer" onSubmit={(e) => { e.preventDefault(); if (canGenerate) void generate() }}>
        <label className="kv-media-prompt-field">
          <span className="sr-only">{text('创作描述', 'Prompt')}</span>
          <TextArea value={form.prompt} onChange={(prompt) => update({ prompt })} rows={3} className="kv-media-prompt-input"
            onKeyDown={(e) => { if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); e.currentTarget.form?.requestSubmit() } }}
            placeholder={form.kind === 'image' ? text('你想看见怎样的画面？描述主体、光线、风格，或从一张参考图开始…', 'What would you like to see? Describe the subject, light and style…') : text('让画面如何运动？描述动作、镜头与节奏，或添加一张首帧…', 'Describe the motion, camera and rhythm, or add a first frame…')} />
        </label>
        {form.referencePaths.length > 0 && <div className="kv-media-references">
          {form.referencePaths.map((path) => <div className="kv-chip kv-media-reference-file" key={path}>
            <ImagePlus size={13} /><span title={path}>{path.split(/[\\/]/).pop()}</span>
            <IconButton size="xs" label={text('移除参考图', 'Remove reference')} onClick={() => update({ referencePaths: form.referencePaths.filter((p) => p !== path) })}><X size={12} /></IconButton>
          </div>)}
        </div>}
        <div className="kv-media-toolbar">
          <IconButton size="sm" variant="ghost" label={form.kind === 'image' ? text('添加参考图（最多 4 张）', 'Add references (up to 4)') : text('添加首帧图片', 'Add first frame')} onClick={() => void pickReferences()} disabled={!desktop}><ImagePlus size={16} /></IconButton>
          <Select className="kv-media-provider" ariaLabel={text('供应商', 'Provider')} value={form.providerId} options={capableProviders.map((p) => ({ value: p.id, label: p.name }))} onChange={(providerId) => update({ providerId, model: '' })} />
          <Select className="kv-media-model" ariaLabel={text('模型', 'Model')} value={form.model} onChange={(model) => update({ model })}
            options={modelChoices.map((model) => ({ value: model, label: model }))} />
          <Select className="kv-media-ratio" ariaLabel={text('画幅', 'Aspect ratio')} value={form.aspectRatio} onChange={(aspectRatio) => update({ aspectRatio })} options={['1:1', '16:9', '9:16', '4:3', '3:4'].map((value) => ({ value, label: value }))} />
          {form.kind === 'video' && <Select className="kv-media-duration" ariaLabel={text('时长', 'Duration')} value={String(form.duration)} onChange={(duration) => update({ duration: Number(duration) })} options={[5, 10, 15].map((n) => ({ value: String(n), label: `${n} ${text('秒', 'sec')}` }))} />}
          <Button className="kv-media-generate" size="sm" variant="primary" type="submit" disabled={!canGenerate}>
            {submitting ? <LoaderCircle size={15} className="animate-spin" /> : <Sparkles size={15} />}{text('开始生成', 'Generate')}
          </Button>
        </div>
      </form>
      <p className="kv-field-hint kv-media-composer-caption">
        {[
          !desktop && text('预览模式 · 请在桌面端生成', 'Preview · Generate in the desktop app'),
          form.kind === 'video' ? text('xAI 兼容视频 · 720p', 'xAI-compatible video · 720p') : text('每次生成 1 张 · 按供应商计费', 'One image per request · Provider charges apply'),
          text('结果自动保存在本机', 'Saved to your device'),
        ].filter(Boolean).join(' · ')}
      </p>
    </div>
  </section>
}
