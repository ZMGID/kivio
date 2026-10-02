import { useEffect, useRef, useState, type CSSProperties } from 'react'
import { open, save } from '@tauri-apps/plugin-dialog'
import { ArrowLeft, Download, History, ImagePlus, Images, LoaderCircle, Play, RefreshCw, Search, Settings2, Sparkles, X } from 'lucide-react'
import { mediaStationApi, type MediaJob, type MediaKind, type MediaRequest } from '../api/mediaStation'
import { getSettingsCached } from '../api/settingsCache'
import { isTauriRuntime, type ModelProvider } from '../api/tauri'
import { Button, IconButton } from '../components/Button'
import { Input, Select, TextArea } from '../settings/public/controls'
import { useLang } from '../components/i18n'
import { resolveModelInfo } from '../data/modelMatching'
import { IdeaArt, type IdeaArtName } from './MediaIdeaArt'
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

// Unsubmitted form state and the latest result survive navigation in this window; credentials remain in settings.
let lastLatestId = ''
let draft: MediaRequest = { kind: 'image', providerId: '', model: '', prompt: '', aspectRatio: '1:1', duration: 5, referencePaths: [] }

type Idea = { art: IdeaArtName; ratio: string; title: readonly [string, string]; prompt: readonly [string, string] }

/** Starter prompts per media kind, each with its own line drawing and a fitting aspect ratio. */
const IDEAS: Record<MediaKind, readonly Idea[]> = {
  image: [
    { art: 'vase', ratio: '1:1', title: ['产品摄影', 'Product photo'], prompt: ['晨光中的玻璃花瓶，简洁背景，柔和阴影，产品摄影。', 'A glass vase in morning light, minimal background, soft shadows, product photography.'] },
    { art: 'coast', ratio: '16:9', title: ['电影感场景', 'Cinematic scene'], prompt: ['夕阳下的海岸，暖金色光线，宽阔构图，电影质感。', 'A coastline at sunset, warm golden light, wide composition, cinematic atmosphere.'] },
    { art: 'poster', ratio: '3:4', title: ['插画海报', 'Illustrated poster'], prompt: ['以森林与月亮为主题的插画海报，深蓝与银白配色，留出标题空间。', 'An illustrated forest and moon poster in midnight blue and silver, with room for a title.'] },
    { art: 'portrait', ratio: '3:4', title: ['人像写真', 'Portrait'], prompt: ['窗边自然光下的人像特写，浅景深，胶片质感，温柔色调。', 'A close-up portrait by a window in natural light, shallow depth of field, film look, soft tones.'] },
    { art: 'coffee', ratio: '1:1', title: ['美食摄影', 'Food photo'], prompt: ['俯拍木桌上的拿铁，心形拉花，晨光斜照，温暖色调。', 'A top-down latte with heart latte art on a wooden table, slanted morning light, warm tones.'] },
    { art: 'arches', ratio: '4:3', title: ['建筑光影', 'Architecture'], prompt: ['极简混凝土拱廊，强烈的光影，一个人走过，建筑摄影。', 'A minimal concrete arcade with strong light and shadow, a lone figure walking through, architectural photography.'] },
    { art: 'room', ratio: '1:1', title: ['等距小屋', 'Isometric room'], prompt: ['等距视角的温馨小书房，柔和光照，3D 渲染。', 'A cozy isometric study room, soft lighting, 3D render.'] },
    { art: 'shanshui', ratio: '16:9', title: ['水墨山水', 'Ink landscape'], prompt: ['水墨山水，远山与云雾，一叶扁舟，大面积留白。', 'An ink-wash landscape with distant mountains, mist and a small boat, generous negative space.'] },
    { art: 'interior', ratio: '4:3', title: ['室内设计', 'Interior'], prompt: ['北欧风客厅，浅色沙发，落地灯，绿植，自然光，室内效果图。', 'A Scandinavian living room with a light sofa, floor lamp and plants in natural light, interior render.'] },
    { art: 'cat', ratio: '1:1', title: ['宠物写真', 'Pet portrait'], prompt: ['一只蜷在毛毯上睡觉的橘猫，午后阳光，柔焦。', 'An orange cat curled up asleep on a blanket in afternoon sun, soft focus.'] },
    { art: 'leaf', ratio: '3:4', title: ['植物图鉴', 'Botanical plate'], prompt: ['龟背竹叶片的植物图鉴，复古科学插画风格，米色纸张。', 'A botanical plate of a monstera leaf, vintage scientific illustration on cream paper.'] },
    { art: 'street', ratio: '9:16', title: ['雨夜街头', 'Rainy street'], prompt: ['雨夜的街头，霓虹倒映在积水里，撑伞的行人，赛博朋克。', 'A rainy street at night, neon reflected in puddles, a pedestrian with an umbrella, cyberpunk.'] },
  ],
  video: [
    { art: 'skyline', ratio: '16:9', title: ['航拍推进', 'Aerial push-in'], prompt: ['黄昏的城市天际线，无人机缓慢向前推进，灯光逐渐亮起。', 'A city skyline at dusk, a drone slowly pushes forward as the lights come on.'] },
    { art: 'turntable', ratio: '1:1', title: ['产品旋转', 'Product turntable'], prompt: ['白色背景上的运动鞋缓慢 360 度旋转，柔和棚拍光。', 'A sneaker slowly rotates 360 degrees on a white background in soft studio light.'] },
    { art: 'peaks', ratio: '16:9', title: ['自然延时', 'Nature time-lapse'], prompt: ['云海在雪山间流动的延时摄影，日出时分，金色光线。', 'A time-lapse of clouds flowing between snowy peaks at sunrise, golden light.'] },
    { art: 'fox', ratio: '16:9', title: ['动画角色', 'Animated character'], prompt: ['一只小狐狸在森林边蹦跳，手绘动画风格，镜头跟随。', 'A small fox hops at the edge of a forest in a hand-drawn animation style, the camera follows.'] },
    { art: 'waves', ratio: '16:9', title: ['海浪慢镜', 'Slow-mo wave'], prompt: ['巨浪卷起的慢动作，阳光穿透浪尖，水花飞溅。', 'A huge wave curling in slow motion, sunlight through the crest, spray flying.'] },
    { art: 'road', ratio: '16:9', title: ['公路追车', 'Road chase'], prompt: ['跑车在沙漠公路上疾驰，低机位跟拍，黄昏逆光。', 'A sports car speeds down a desert highway, low tracking shot, backlit at dusk.'] },
    { art: 'pour', ratio: '9:16', title: ['液体慢镜', 'Pour shot'], prompt: ['冰饮倒入玻璃杯的慢镜头，冰块翻滚，气泡上升。', 'Slow motion of a cold drink poured into a glass, ice tumbling, bubbles rising.'] },
    { art: 'blossom', ratio: '9:16', title: ['花开延时', 'Blooming'], prompt: ['一朵花从花苞到盛开的延时摄影，黑色背景，微距。', 'A time-lapse of a flower opening from bud to full bloom, black background, macro.'] },
    { art: 'walk', ratio: '9:16', title: ['人物跟拍', 'Tracking shot'], prompt: ['女孩走在秋天的林荫道上，侧面跟拍，落叶飘下。', 'A girl walks along an autumn avenue, side tracking shot, leaves falling.'] },
    { art: 'rainwin', ratio: '16:9', title: ['雨窗氛围', 'Rainy window'], prompt: ['雨滴顺着窗玻璃滑落，窗外城市灯光虚化，浅景深。', 'Raindrops run down a window, city lights blurred outside, shallow depth of field.'] },
    { art: 'fireworks', ratio: '16:9', title: ['烟花夜景', 'Fireworks'], prompt: ['夜空中绽放的烟花，城市剪影，镜头缓慢上摇。', 'Fireworks bursting over a city skyline at night, the camera slowly tilts up.'] },
    { art: 'plane', ratio: '16:9', title: ['纸飞机', 'Paper plane'], prompt: ['一架纸飞机穿过云层飞行，镜头跟随，绘本风格。', 'A paper plane glides through clouds, the camera follows, storybook style.'] },
  ],
}
const IDEAS_PER_PAGE = 4
/** Rotates across visits in this window so the page doesn't open on the same four every time. */
let ideaOffset = Math.floor(Math.random() * 3) * IDEAS_PER_PAGE

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

type DetailActions = {
  text: (cn: string, en: string) => string
  statusLabel: string
  onClose: () => void
  onCancel: () => void
  onResume: () => void
  onExport: (index: number) => void
  onFirstFrame: (index: number) => void
  onReuse: () => void
}

function jobMeta(job: MediaJob, statusLabel: string) {
  return [statusLabel, job.request.model, job.request.aspectRatio, job.request.kind === 'video' ? `${job.request.duration}s` : ''].filter(Boolean).join(' · ')
}

/** Running state: a canvas in the requested aspect ratio with a soft sheen, plus elapsed time and the stop action. */
function MediaPending({ job, text, onCancel }: { job: MediaJob; text: DetailActions['text']; onCancel: () => void }) {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000)
    return () => window.clearInterval(timer)
  }, [])
  const seconds = Math.max(0, Math.floor((now - job.createdAt) / 1000))
  const elapsed = `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`
  const [w, h] = job.request.aspectRatio.split(':').map(Number)
  const ratio = w > 0 && h > 0 ? w / h : 1
  return <div className="kv-media-pending" style={{ '--kv-media-ratio': ratio } as CSSProperties}>
    <div className="kv-media-pending-canvas" aria-hidden="true">
      {job.request.kind === 'video' ? <Play size={22} strokeWidth={1.5} /> : <Images size={22} strokeWidth={1.5} />}
      <span className="kv-media-pending-ratio">{job.request.aspectRatio}</span>
    </div>
    <div className="kv-media-pending-status" aria-live="polite">
      <div>
        <p>{text('正在生成，稍后回来也可以。', 'Generating. You can come back later.')}</p>
        <small>{text(`已等待 ${elapsed} · 供应商可能继续生成并计费`, `${elapsed} elapsed · The provider may continue and charge`)}</small>
      </div>
      <Button size="sm" onClick={onCancel}>{text('停止等待', 'Stop waiting')}</Button>
    </div>
  </div>
}

/** Preview, status and prompt of one creation; shared by the latest result and the history dialog. */
function MediaResultBody({ job, text, onCancel }: { job: MediaJob; text: DetailActions['text']; onCancel: () => void }) {
  return <>
    {job.status === 'running' && <MediaPending job={job} text={text} onCancel={onCancel} />}
    {job.error && <p role="status" className="kv-panel warn kv-media-job-error">{job.error}</p>}
    {job.outputs.map((output, index) => <div key={output.name} className="kv-media-preview"><OutputPreview job={job} index={index} /></div>)}
    <p className="kv-media-prompt">{job.request.prompt}</p>
  </>
}

function MediaResultActions({ job, text, onResume, onExport, onFirstFrame, onReuse }: { job: MediaJob } & Omit<DetailActions, 'statusLabel' | 'onClose' | 'onCancel'>) {
  return <>
    <Button size="sm" variant="ghost" onClick={onReuse}><RefreshCw size={14} />{text('复用参数', 'Reuse settings')}</Button>
    <span className="kv-media-detail-spacer" />
    {job.providerTaskId && job.status !== 'running' && job.status !== 'completed' && <Button size="sm" onClick={onResume}><RefreshCw size={14} />{text('继续获取结果', 'Fetch result again')}</Button>}
    {job.request.kind === 'image' && job.outputs.map((output, index) => <Button key={`frame-${output.name}`} size="sm" onClick={() => onFirstFrame(index)}><Play size={14} />{text('用作视频首帧', 'Use as first frame')}</Button>)}
    {job.outputs.map((output, index) => <Button key={`save-${output.name}`} size="sm" variant="primary" onClick={() => onExport(index)}><Download size={14} />{text('另存为', 'Save as')}</Button>)}
  </>
}

/** A creation opened from the history grid, shown as a modal over the history view. */
function MediaDetail({ job, ...actions }: { job: MediaJob } & DetailActions) {
  const dialog = useRef<HTMLDialogElement>(null)
  useEffect(() => {
    const node = dialog.current
    if (node && !node.open) node.showModal()
    return () => node?.close()
  }, [])
  const { text, statusLabel, onClose, onCancel } = actions
  return <dialog ref={dialog} className="kv-modal kv-media-detail" aria-label={text('作品详情', 'Creation details')}
    onCancel={(e) => { e.preventDefault(); onClose() }} onClick={(e) => { if (e.target === e.currentTarget) onClose() }}>
    <header className="kv-media-detail-header">
      <span>{jobMeta(job, statusLabel)}</span>
      <IconButton size="sm" variant="ghost" label={text('关闭', 'Close')} onClick={onClose}><X size={15} /></IconButton>
    </header>
    <div className="kv-media-detail-body custom-scrollbar"><MediaResultBody job={job} text={text} onCancel={onCancel} /></div>
    <footer className="kv-media-detail-actions"><MediaResultActions job={job} {...actions} /></footer>
  </dialog>
}

export function MediaStation({ onOpenSettings }: { onOpenSettings: () => void }) {
  const zh = useLang() === 'zh'
  const text = (cn: string, en: string) => zh ? cn : en
  const desktop = isTauriRuntime()
  const [form, setForm] = useState<MediaRequest>(() => draft)
  const [providers, setProviders] = useState<ModelProvider[]>([])
  const [jobs, setJobs] = useState<MediaJob[]>([])
  const [selectedId, setSelectedId] = useState('')
  // 「创作」是默认视图，只展示本次生成；完整历史在单独的「创作记录」视图里。
  const [view, setView] = useState<'create' | 'history'>('create')
  const [latestId, setLatestId] = useState(lastLatestId)
  const [ideaStart, setIdeaStart] = useState(() => ideaOffset)
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
      lastLatestId = job.id; setLatestId(job.id); setRefresh((n) => n + 1)
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

  const latest = jobs.find((j) => j.id === latestId)
  const ideaPool = IDEAS[form.kind]
  const shownIdeas = Array.from({ length: IDEAS_PER_PAGE }, (_, i) => ideaPool[(ideaStart + i) % ideaPool.length])
  const nextIdeas = () => setIdeaStart((start) => (ideaOffset = (start + IDEAS_PER_PAGE) % ideaPool.length))
  useEffect(() => () => { ideaOffset = (ideaOffset + IDEAS_PER_PAGE) % IDEAS.image.length }, [])
  /** Actions on a result; reusing it brings the parameters back to the create view. */
  const resultActions = (job: MediaJob) => ({
    text,
    onResume: () => void resume(job),
    onExport: (index: number) => void exportOutput(job, index),
    onFirstFrame: (index: number) => { setSelectedId(''); setView('create'); void handleFirstFrame(job, index) },
    onReuse: () => { setSelectedId(''); setView('create'); update(job.request) },
  })

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
      <div className="kv-media-header-actions">
        <Button size="sm" variant="ghost" aria-pressed={view === 'history'} onClick={() => setView(view === 'history' ? 'create' : 'history')}>
          {view === 'history' ? <><ArrowLeft size={15} />{text('返回创作', 'Back to create')}</> : <><History size={15} />{text('创作记录', 'Creations')}{jobs.length > 0 && <span className="kv-media-count">{jobs.length}</span>}</>}
        </Button>
        <Button size="sm" variant="ghost" onClick={onOpenSettings}><Settings2 size={15} />{text('模型设置', 'Model settings')}</Button>
      </div>
    </header>
    {error && <div className="kv-panel kv-media-notice" role="alert">{error}<IconButton label={text('关闭提示', 'Dismiss')} onClick={() => setError('')}><X size={14} /></IconButton></div>}
    {view === 'create' ? <div className="kv-scroll custom-scrollbar kv-media-workspace">
      {latest ? <article className="kv-media-latest" aria-label={text('本次生成', 'Latest result')}>
        <header className="kv-media-detail-header">
          <span>{jobMeta(latest, statusLabel(latest))}</span>
          <IconButton size="sm" variant="ghost" label={text('收起', 'Dismiss')} onClick={() => { lastLatestId = ''; setLatestId('') }}><X size={15} /></IconButton>
        </header>
        <div className="kv-media-detail-body"><MediaResultBody job={latest} text={text} onCancel={() => void cancel(latest)} /></div>
        <footer className="kv-media-detail-actions"><MediaResultActions job={latest} {...resultActions(latest)} /></footer>
      </article> : <div className="kv-media-hero">
        <div className="kv-media-hero-head">
          <div>
            <h3>{form.kind === 'image' ? text('想画点什么', 'What would you like to make') : text('想让什么动起来', 'What should move')}</h3>
            <p>{form.kind === 'image' ? text('选一个方向，会填好描述和画幅；也可以直接在下面写', 'Pick a direction to fill in the prompt and ratio, or just write below') : text('选一个镜头，会填好描述和画幅；也可以直接在下面写', 'Pick a shot to fill in the prompt and ratio, or just write below')}</p>
          </div>
          <Button size="sm" variant="ghost" onClick={nextIdeas}><RefreshCw size={14} />{text('换一批', 'Shuffle')}</Button>
        </div>
        <div className="kv-media-ideas" key={`${form.kind}-${ideaStart}`}>
          {shownIdeas.map(({ art, ratio, title, prompt }, i) =>
            <button key={art} type="button" className="kv-media-idea" style={{ '--kv-idea-delay': `${i * 40}ms` } as CSSProperties}
              title={text(prompt[0], prompt[1])} onClick={() => update({ prompt: text(prompt[0], prompt[1]), aspectRatio: ratio })}>
              <span className="kv-media-idea-art"><IdeaArt name={art} /><span className="kv-media-idea-ratio">{ratio}</span></span>
              <strong>{text(title[0], title[1])}</strong>
              <span className="kv-media-idea-prompt">{text(prompt[0], prompt[1])}</span>
            </button>)}
        </div>
      </div>}
    </div> : <div className="kv-scroll custom-scrollbar kv-media-workspace">
      <div className="kv-media-library">
        <div className="kv-media-library-heading">
          <div className="kv-plugin-segments" role="group" aria-label={text('筛选类型', 'Filter type')}>
            {([['all', text('全部', 'All')], ['image', text('图片', 'Images')], ['video', text('视频', 'Videos')]] as const).map(([value, label]) =>
              <button key={value} type="button" className="kv-plugin-segment" aria-pressed={filter === value} aria-current={filter === value ? 'page' : undefined} onClick={() => setFilter(value)}>{label}</button>)}
          </div>
          <div className="kv-media-library-tools">
            <label className="kv-media-search">
              <Search size={14} aria-hidden="true" />
              <Input aria-label={text('搜索描述', 'Search prompts')} value={query} onChange={setQuery} placeholder={text('搜索创作描述…', 'Search prompts…')} />
            </label>
            <IconButton size="sm" variant="ghost" label={text('刷新记录', 'Refresh')} onClick={() => setRefresh((n) => n + 1)} disabled={!desktop}><RefreshCw size={15} /></IconButton>
          </div>
        </div>
        {loading ? <p role="status" className="kv-field-hint">{text('正在读取记录…', 'Loading history…')}</p> : visible.length === 0 ? <div className="kv-media-empty">
          <span className="kv-media-empty-icon"><Images size={22} strokeWidth={1.5} /></span>
          <h3>{jobs.length ? text('没有匹配的作品', 'No matching creations') : text('还没有作品', 'No creations yet')}</h3>
          <p>{jobs.length ? text('试试其他关键词或类型。', 'Try another keyword or type.') : text('回到创作页生成第一张吧。', 'Go back and create your first one.')}</p>
        </div> : <div className="kv-media-grid">{visible.map((job) => <button type="button" key={job.id} className="kv-media-card" aria-haspopup="dialog" onClick={() => setSelectedId(job.id)}><div className="kv-media-cover">{job.outputs[0]?.preview ? <img src={job.outputs[0].preview} alt="" loading="lazy" /> : job.status === 'running' ? <LoaderCircle className="animate-spin" size={25} /> : job.request.kind === 'image' ? <Images size={28} /> : <Play size={28} />}{job.status !== 'completed' && <span className={`kv-media-badge is-${job.status}`}>{statusLabel(job)}</span>}</div><strong>{job.request.prompt}</strong><small>{job.request.aspectRatio} · {new Date(job.createdAt).toLocaleString(zh ? 'zh-CN' : 'en-US', { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })}</small></button>)}</div>}
      </div>
    </div>}
    {view === 'history' && selected && <MediaDetail key={selected.id} job={selected} statusLabel={statusLabel(selected)}
      onClose={() => setSelectedId('')} onCancel={() => void cancel(selected)} {...resultActions(selected)} />}
    {view === 'create' && <div className="kv-media-dock">
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
          form.kind === 'video' ? text('每次生成 1 段 · 按供应商计费', 'One video per request · Provider charges apply') : text('每次生成 1 张 · 按供应商计费', 'One image per request · Provider charges apply'),
          text('结果自动保存在本机', 'Saved to your device'),
        ].filter(Boolean).join(' · ')}
      </p>
    </div>}
  </section>
}
