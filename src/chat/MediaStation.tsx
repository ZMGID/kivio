import { useCallback, useEffect, useRef, useState, type CSSProperties } from 'react'
import { open, save } from '@tauri-apps/plugin-dialog'
import { createWindowStore, useWindowStore } from '../utils/windowStore'
import { ArrowLeft, Check, Circle, Download, History, ImagePlus, Images, LoaderCircle, Play, RefreshCw, Search, Settings2, Sparkles, Trash2, X } from 'lucide-react'
import { mediaStationApi, type MediaJob, type MediaKind, type MediaRequest } from '../api/mediaStation'
import { getSettingsCached } from '../api/settingsCache'
import { isTauriRuntime, type ModelProvider } from '../api/tauri'
import { Button, IconButton } from '../components/Button'
import { confirmDialog } from '../components/dialogQueue'
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

type MediaWindow = {
  draft: MediaRequest
  latestIds: Record<MediaKind, string>
  error: string
  /** Confirmed deletions. A later history response must not paint these jobs again. */
  deletedIds: readonly string[]
  /** In-flight keys: `start`, `resume:<id>`, `cancel:<id>`, `delete:<id>`, `export:<id>:<index>`, `reference:<id>:<index>:<mode>`. */
  pending: readonly string[]
  historyEpoch: number
  selectedId: string
  selection: readonly string[]
  editHint: string
  view: 'create' | 'history'
}

function initialMediaWindow(): MediaWindow {
  return {
    draft: { kind: 'image', providerId: '', model: '', prompt: '', aspectRatio: '1:1', duration: 5, referencePaths: [] },
    latestIds: { image: '', video: '' },
    error: '',
    deletedIds: [],
    pending: [],
    historyEpoch: 0,
    selectedId: '',
    selection: [],
    editHint: '',
    view: 'create',
  }
}

/** Submission, draft, and current-result binding survive navigation in this window. Job status stays on the backend. */
const mediaWindow = createWindowStore(initialMediaWindow())
let referenceSerial = 0
// User edits invalidate pending references; automatic model fallback does not.
let draftRevision = 0

function setMediaView(view: MediaWindow['view']) {
  mediaWindow.setState((state) => state.view === view ? state : { ...state, view })
}

function togglePending<State extends { pending: readonly string[] }>(state: State, keys: readonly string[], active: boolean): State {
  const drop = new Set(keys)
  const pending = active
    ? [...state.pending, ...keys.filter((key) => !state.pending.includes(key))]
    : state.pending.filter((key) => !drop.has(key))
  if (pending.length === state.pending.length && pending.every((key, index) => key === state.pending[index])) return state
  return { ...state, pending }
}

function runMedia(key: string, pendingKeys: readonly string[], command: () => Promise<void>) {
  return mediaWindow.run(key, async () => {
    mediaWindow.setState((state) => togglePending(state, pendingKeys, true))
    try { await command() }
    catch (e) { mediaWindow.setState((state) => ({ ...state, error: String(e) })) }
    finally { mediaWindow.setState((state) => togglePending(state, pendingKeys, false)) }
  })
}

// eslint-disable-next-line react-refresh/only-export-components
export function resetMediaWindowForTests() {
  referenceSerial += 1
  draftRevision += 1
  mediaWindow.setState(initialMediaWindow())
}

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
    ? <video src={url} controls preload="auto" onLoadedMetadata={(event) => {
      // WebKit can leave an unplayed video blank until a seek decodes its first frame.
      const video = event.currentTarget
      if (video.currentTime === 0 && video.duration > 0) video.currentTime = Math.min(0.001, video.duration / 2)
    }} />
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
  onEdit: (index: number) => void
  onDelete: () => void
  deleting: boolean
  resuming: boolean
  exporting: (index: number) => boolean
}

function jobMeta(job: MediaJob, statusLabel: string) {
  return [statusLabel, job.request.model, job.request.aspectRatio, job.request.kind === 'video' ? `${job.request.duration}s` : ''].filter(Boolean).join(' · ')
}

/** Running state: a canvas in the requested aspect ratio with a soft sheen, plus elapsed time and the stop action. */
function MediaPending({ job, text, onCancel, busy }: { job: MediaJob; text: DetailActions['text']; onCancel: () => void; busy: boolean }) {
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
      <Button size="sm" onClick={onCancel} disabled={busy}>{text('停止等待', 'Stop waiting')}</Button>
    </div>
  </div>
}

/** Preview, status and prompt of one creation; shared by the latest result and the history dialog. */
function MediaResultBody({ job, text, onCancel, cancelling }: { job: MediaJob; text: DetailActions['text']; onCancel: () => void; cancelling: boolean }) {
  return <>
    {job.status === 'running' && <MediaPending job={job} text={text} onCancel={onCancel} busy={cancelling} />}
    {job.error && <p role="status" className="kv-panel warn kv-media-job-error">{job.error}</p>}
    {job.outputs.map((output, index) => <div key={output.name} className="kv-media-preview"><OutputPreview job={job} index={index} /></div>)}
    <p className="kv-media-prompt">{job.request.prompt}</p>
  </>
}

function MediaResultActions({ job, text, onResume, onExport, onFirstFrame, onReuse, onEdit, onDelete, deleting, resuming, exporting }: { job: MediaJob } & Omit<DetailActions, 'statusLabel' | 'onClose' | 'onCancel'>) {
  return <>
    <Button size="sm" variant="ghost" onClick={onReuse}><RefreshCw size={14} />{job.request.kind === 'video' ? text('修改参数 / 重新生成', 'Change settings / regenerate') : text('复用参数', 'Reuse settings')}</Button>
    <Button size="sm" variant="danger" onClick={onDelete} disabled={deleting || job.status === 'running'} title={job.status === 'running' ? text('请先停止等待，再删除记录和源文件。', 'Stop waiting before deleting the record and source files.') : undefined}><Trash2 size={14} />{text('删除', 'Delete')}</Button>
    {job.status === 'running' && <small className="kv-field-hint">{text('请先停止等待再删除', 'Stop waiting before deleting')}</small>}
    <span className="kv-media-detail-spacer" />
    {job.providerTaskId && job.status !== 'running' && job.status !== 'completed' && <Button size="sm" onClick={onResume} disabled={resuming}><RefreshCw size={14} />{text('继续获取结果', 'Fetch result again')}</Button>}
    {job.request.kind === 'image' && job.outputs.map((output, index) => <Button key={`edit-${output.name}`} size="sm" onClick={() => onEdit(index)}><ImagePlus size={14} />{text('继续编辑', 'Continue editing')}{job.outputs.length > 1 && ` ${index + 1}`}</Button>)}
    {job.request.kind === 'image' && job.outputs.map((output, index) => <Button key={`frame-${output.name}`} size="sm" onClick={() => onFirstFrame(index)}><Play size={14} />{text('用作视频首帧', 'Use as first frame')}</Button>)}
    {job.outputs.map((output, index) => <Button key={`save-${output.name}`} size="sm" variant="primary" disabled={exporting(index)} onClick={() => onExport(index)}><Download size={14} />{text('另存为', 'Save as')}</Button>)}
  </>
}

/** A creation opened from the history grid, shown as a modal over the history view. */
function MediaDetail({ job, errorMessage, cancelling, ...actions }: { job: MediaJob; errorMessage: string; cancelling: boolean } & DetailActions) {
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
    <div className="kv-media-detail-body custom-scrollbar">{errorMessage && <p role="alert" className="kv-panel warn">{errorMessage}</p>}<MediaResultBody job={job} text={text} onCancel={onCancel} cancelling={cancelling} /></div>
    <footer className="kv-media-detail-actions"><MediaResultActions job={job} {...actions} /></footer>
  </dialog>
}

export function MediaStation({ onOpenSettings }: { onOpenSettings: () => void }) {
  const zh = useLang() === 'zh'
  const text = (cn: string, en: string) => zh ? cn : en
  const desktop = isTauriRuntime()
  const [media, setMedia] = useWindowStore(mediaWindow)
  const form = media.draft
  const latestIds = media.latestIds
  const error = media.error
  const selectedId = media.selectedId
  const selection = new Set(media.selection)
  const submitting = media.pending.includes('start')
  const deleting = media.pending.some((key) => key.startsWith('delete:'))
  const [providers, setProviders] = useState<ModelProvider[]>([])
  const [listedJobs, setListedJobs] = useState<MediaJob[]>([])
  // 「创作」是默认视图，只展示本次生成；完整历史在单独的「创作记录」视图里。
  const view = media.view
  const [ideaStart, setIdeaStart] = useState(() => ideaOffset)
  const [filter, setFilter] = useState('all')
  const [query, setQuery] = useState('')
  const [loading, setLoading] = useState(desktop)
  const jobs = listedJobs.filter((job) => !media.deletedIds.includes(job.id))
  const selected = jobs.find((j) => j.id === selectedId)
  // 只列出能生成当前类型的供应商和模型。
  const capableProviders = providers.filter((p) => mediaModels(p, form.kind).length > 0)
  const provider = capableProviders.find((p) => p.id === form.providerId)
  const modelChoices = provider ? mediaModels(provider, form.kind) : []
  const running = jobs.filter((j) => j.status === 'running').length
  const visible = jobs.filter((j) => (filter === 'all' || j.request.kind === filter) && j.request.prompt.toLowerCase().includes(query.toLowerCase()))
  const deletableVisible = visible.filter((job) => job.status !== 'running')
  const statusLabel = (job: MediaJob) => ({ running: text('生成中', 'Generating'), completed: text('已完成', 'Completed'), failed: text('失败', 'Failed'), cancelled: text('已停止等待', 'Stopped waiting'), interrupted: text('已中断', 'Interrupted') })[job.status]

  const update = useCallback((patch: Partial<MediaRequest>) => {
    draftRevision += 1
    setMedia((state) => ({ ...state, draft: { ...state.draft, ...patch } }))
  }, [setMedia])
  useEffect(() => {
    if (!desktop) return
    let active = true
    getSettingsCached().then((settings) => {
      if (!active) return
      setProviders(settings.providers.filter((p) => p.enabled))
      const current = mediaWindow.getSnapshot().draft
      if (!current.providerId) {
        const initial = settings.defaultModels.imageGeneration
        if (initial.providerId) setMedia((state) => ({ ...state, draft: { ...state.draft, providerId: initial.providerId, model: initial.model } }))
      }
    }).catch((e) => { if (active) setMedia((state) => ({ ...state, error: String(e) })) })
    return () => { active = false }
  }, [desktop, setMedia])
  useEffect(() => {
    if (!desktop) return
    let active = true
    let timer: ReturnType<typeof setTimeout>
    const load = async () => {
      try {
        const result = await mediaStationApi.list()
        if (!active) return
        const retained = result.filter((job) => !mediaWindow.getSnapshot().deletedIds.includes(job.id))
        setListedJobs(retained)
        if (retained.some((j) => j.status === 'running')) timer = setTimeout(() => void load(), 2000)
      } catch (e) { if (active) setMedia((state) => ({ ...state, error: String(e) })) }
      finally { if (active) setLoading(false) }
    }
    void load()
    return () => { active = false; clearTimeout(timer) }
  }, [desktop, media.historyEpoch, setMedia])

  // 当前供应商 / 模型不支持所选类型（切换类型、复用旧任务、设置变更）时，落到第一个可用项。
  const fallbackProvider = provider ?? capableProviders[0]
  const fallbackModel = fallbackProvider && (mediaModels(fallbackProvider, form.kind).includes(form.model) ? form.model : mediaModels(fallbackProvider, form.kind)[0])
  useEffect(() => {
    if (!fallbackProvider) return
    if (fallbackProvider.id !== form.providerId || fallbackModel !== form.model) {
      setMedia((state) => ({ ...state, draft: { ...state.draft, providerId: fallbackProvider.id, model: fallbackModel ?? '' } }))
    }
  }, [fallbackProvider, fallbackModel, form.providerId, form.model, setMedia])

  function changeKind(kind: MediaKind) {
    update({ kind, model: '', referencePaths: [], aspectRatio: kind === 'image' ? '1:1' : '16:9' })
  }
  function updateLatestResult(kind: MediaKind, id: string) {
    setMedia((state) => ({ ...state, latestIds: { ...state.latestIds, [kind]: id } }))
  }
  function refreshHistory() {
    setMedia((state) => ({ ...state, historyEpoch: state.historyEpoch + 1 }))
  }
  function generate() {
    const current = mediaWindow.getSnapshot().draft
    const request: MediaRequest = { ...current, prompt: current.prompt.trim(), model: current.model.trim() }
    void runMedia('start', ['start'], async () => {
      setMedia((state) => ({ ...state, error: '' }))
      const job = await mediaStationApi.start(request)
      if (mediaWindow.getSnapshot().deletedIds.includes(job.id)) return
      setMedia((state) => ({
        ...state,
        latestIds: { ...state.latestIds, [job.request.kind]: job.id },
        historyEpoch: state.historyEpoch + 1,
      }))
    })
  }
  function pickReferences() {
    const multiple = mediaWindow.getSnapshot().draft.kind === 'image'
    const revision = draftRevision
    void runMedia('references', ['references'], async () => {
      const serial = ++referenceSerial
      const paths = await open({ multiple, filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'webp'] }] })
      if (!paths || serial !== referenceSerial || revision !== draftRevision) return
      const picked = Array.isArray(paths) ? paths : [paths]
      const limitMessage = text('图片最多 4 张参考图，视频最多 1 张首帧。', 'Use up to 4 image references or 1 video first frame.')
      setMedia((state) => {
        const next = [...new Set([...state.draft.referencePaths, ...picked])]
        if (next.length > (state.draft.kind === 'image' ? 4 : 1)) return { ...state, error: limitMessage }
        return { ...state, draft: { ...state.draft, referencePaths: next }, error: '' }
      })
    })
  }
  function exportOutput(job: MediaJob, index: number) {
    const output = job.outputs[index]
    if (!output) return
    const key = `export:${job.id}:${index}`
    void runMedia(key, [key], async () => {
      const destination = await save({ defaultPath: output.name })
      if (!destination || mediaWindow.getSnapshot().deletedIds.includes(job.id)) return
      await mediaStationApi.export(job.id, index, destination)
    })
  }
  function cancel(jobId: string) {
    const key = `cancel:${jobId}`
    void runMedia(key, [key], async () => {
      setMedia((state) => ({ ...state, error: '' }))
      await mediaStationApi.cancel(jobId)
      if (mediaWindow.getSnapshot().deletedIds.includes(jobId)) return
      refreshHistory()
    })
  }
  function resume(jobId: string) {
    const key = `resume:${jobId}`
    void runMedia(key, [key], async () => {
      setMedia((state) => ({ ...state, error: '' }))
      await mediaStationApi.resume(jobId)
      if (mediaWindow.getSnapshot().deletedIds.includes(jobId)) return
      refreshHistory()
    })
  }
  function toggleSelection(id: string) {
    setMedia((state) => ({
      ...state,
      selection: state.selection.includes(id) ? state.selection.filter((item) => item !== id) : [...state.selection, id],
    }))
  }
  function deleteJobs(ids: string[]) {
    const unique = [...new Set(ids)]
    if (!unique.length) return
    if (unique.some((id) => jobs.find((job) => job.id === id)?.status === 'running')) {
      setMedia((state) => ({ ...state, error: text('请先停止等待，再删除记录和源文件。', 'Stop waiting before deleting the record and source files.') }))
      return
    }
    const flags = unique.map((id) => `delete:${id}`)
    void runMedia(`delete:${[...unique].sort().join(',')}`, flags, async () => {
      const confirmed = await confirmDialog({
        message: text(`永久删除 ${unique.length} 条创作记录及其本机源文件？作品库中的关联作品也会移除。此操作无法撤销，已导出的副本不受影响。其他位置使用这些源文件的参考图可能失效。`, `Permanently delete ${unique.length} creation record(s) and their local source files? Linked Works entries will also be removed. This cannot be undone. Exported copies are safe. References to these source files elsewhere may become invalid.`),
        confirmLabel: text('永久删除', 'Delete permanently'), danger: true,
      })
      if (!confirmed) return
      try {
        setMedia((state) => ({ ...state, error: '' }))
        const result = await mediaStationApi.delete(unique)
        const removed = new Set(result.deletedIds)
        const failure = result.failures.length
          ? text('部分记录未删除，请处理错误后重试：', 'Some records were not deleted. Resolve the errors and retry: ') + result.failures.map(({ id, error: reason }) => `${id}: ${reason}`).join('；')
          : ''
        setMedia((state) => ({
          ...state,
          deletedIds: [...new Set([...state.deletedIds, ...result.deletedIds])],
          latestIds: {
            image: removed.has(state.latestIds.image) ? '' : state.latestIds.image,
            video: removed.has(state.latestIds.video) ? '' : state.latestIds.video,
          },
          selectedId: removed.has(state.selectedId) ? '' : state.selectedId,
          selection: state.selection.filter((id) => !removed.has(id)),
          historyEpoch: state.historyEpoch + 1,
          error: failure || state.error,
        }))
      } catch (e) {
        setMedia((state) => ({ ...state, error: text('删除失败，可重试：', 'Deletion failed. Retry: ') + String(e) }))
      }
    })
  }
  function editImage(job: MediaJob, index: number) {
    const key = `reference:${job.id}:${index}:edit`
    const revision = draftRevision
    const selectedAtStart = mediaWindow.getSnapshot().selectedId
    void runMedia(key, [key], async () => {
      const serial = ++referenceSerial
      try {
        const path = await mediaStationApi.reference(job.id, index)
        if (serial !== referenceSerial || revision !== draftRevision || mediaWindow.getSnapshot().selectedId !== selectedAtStart || mediaWindow.getSnapshot().deletedIds.includes(job.id)) return
        setMedia((state) => ({
          ...state,
          draft: { ...job.request, referencePaths: [path] },
          selectedId: '',
          editHint: text('已用这张作品作为参考图。修改描述后手动生成新作品，原作品不会被覆盖。', 'This output is now the reference image. Change the prompt and generate a new creation manually; the original stays unchanged.'),
        }))
        setMediaView('create')
      } catch (e) {
        if (serial !== referenceSerial || revision !== draftRevision || mediaWindow.getSnapshot().selectedId !== selectedAtStart) return
        throw e
      }
    })
  }
  function handleFirstFrame(job: MediaJob, index: number) {
    const aspectRatio = job.request.aspectRatio
    const key = `reference:${job.id}:${index}:frame`
    const revision = draftRevision
    const selectedAtStart = mediaWindow.getSnapshot().selectedId
    void runMedia(key, [key], async () => {
      const serial = ++referenceSerial
      try {
        const path = await mediaStationApi.reference(job.id, index)
        if (serial !== referenceSerial || revision !== draftRevision || mediaWindow.getSnapshot().selectedId !== selectedAtStart || mediaWindow.getSnapshot().deletedIds.includes(job.id)) return
        setMedia((state) => ({ ...state, draft: { ...state.draft, kind: 'video', model: '', prompt: '', referencePaths: [path], aspectRatio } }))
      } catch (e) {
        if (serial !== referenceSerial || revision !== draftRevision || mediaWindow.getSnapshot().selectedId !== selectedAtStart) return
        throw e
      }
    })
  }

  const latest = jobs.find((j) => j.id === latestIds[form.kind] && j.request.kind === form.kind)
  const ideaPool = IDEAS[form.kind]
  const shownIdeas = Array.from({ length: IDEAS_PER_PAGE }, (_, i) => ideaPool[(ideaStart + i) % ideaPool.length])
  const nextIdeas = () => setIdeaStart((start) => (ideaOffset = (start + IDEAS_PER_PAGE) % ideaPool.length))
  useEffect(() => () => { ideaOffset = (ideaOffset + IDEAS_PER_PAGE) % IDEAS.image.length }, [])
  /** Actions on a result; reusing it brings the parameters back to the create view. */
  const resultActions = (job: MediaJob) => ({
    text,
    onResume: () => resume(job.id),
    onExport: (index: number) => exportOutput(job, index),
    onEdit: (index: number) => editImage(job, index),
    onDelete: () => deleteJobs([job.id]),
    deleting: media.pending.includes(`delete:${job.id}`),
    resuming: media.pending.includes(`resume:${job.id}`),
    exporting: (index: number) => media.pending.includes(`export:${job.id}:${index}`),
    onFirstFrame: (index: number) => { setMedia((state) => ({ ...state, selectedId: '' })); setMediaView('create'); handleFirstFrame(job, index) },
    onReuse: () => {
      draftRevision += 1
      setMediaView('create')
      setMedia((state) => ({
        ...state,
        selectedId: '',
        draft: { ...state.draft, ...job.request },
        editHint: job.request.kind === 'video' ? text('修改参数后手动生成新视频；不会编辑或覆盖原视频。', 'Change settings, then generate a new video manually. This does not edit or overwrite the original video.') : '',
      }))
    },
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
        <Button size="sm" variant="ghost" aria-pressed={view === 'history'} onClick={() => setMediaView(view === 'history' ? 'create' : 'history')}>
          {view === 'history' ? <><ArrowLeft size={15} />{text('返回创作', 'Back to create')}</> : <><History size={15} />{text('创作记录', 'Creations')}{jobs.length > 0 && <span className="kv-media-count">{jobs.length}</span>}</>}
        </Button>
        <Button size="sm" variant="ghost" onClick={onOpenSettings}><Settings2 size={15} />{text('模型设置', 'Model settings')}</Button>
      </div>
    </header>
    {error && !(view === 'history' && selected) && <div className="kv-panel kv-media-notice" role="alert">{error}<IconButton label={text('关闭提示', 'Dismiss')} onClick={() => setMedia((state) => ({ ...state, error: '' }))}><X size={14} /></IconButton></div>}
    {view === 'create' ? <div className="kv-scroll custom-scrollbar kv-media-workspace">
      {latest ? <article className="kv-media-latest" aria-label={text('本次生成', 'Latest result')}>
        <header className="kv-media-result-header">
          <span>{jobMeta(latest, statusLabel(latest))}</span>
          <IconButton size="sm" variant="ghost" label={text('收起', 'Dismiss')} onClick={() => updateLatestResult(form.kind, '')}><X size={15} /></IconButton>
        </header>
        <div className="kv-media-result-body"><MediaResultBody job={latest} text={text} onCancel={() => cancel(latest.id)} cancelling={media.pending.includes(`cancel:${latest.id}`)} /></div>
        <footer className="kv-media-result-actions"><MediaResultActions job={latest} {...resultActions(latest)} /></footer>
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
            <IconButton size="sm" variant="ghost" label={text('刷新记录', 'Refresh')} onClick={refreshHistory} disabled={!desktop}><RefreshCw size={15} /></IconButton>
          </div>
          <Button size="sm" disabled={deleting || !deletableVisible.length} onClick={() => setMedia((state) => ({ ...state, selection: [...new Set([...state.selection, ...deletableVisible.map((job) => job.id)])] }))}>{text('全选当前结果', 'Select all filtered results')}</Button>
        </div>
        {selection.size > 0 && <div className="kv-media-selection-toolbar" aria-label={text('记录选择', 'Record selection')}>
          <span role="status">{text(`已选择 ${selection.size} 条`, `${selection.size} selected`)}</span>
          <Button size="sm" variant="danger" disabled={deleting || !selection.size} onClick={() => void deleteJobs([...selection])}><Trash2 size={14} />{text('删除所选', 'Delete selected')}</Button>
          <small className="kv-field-hint">{text('生成中的记录请先打开详情停止等待，再删除。', 'Open running records and stop waiting before deleting.')}</small>
        </div>}
        {loading ? <p role="status" className="kv-field-hint">{text('正在读取记录…', 'Loading history…')}</p> : visible.length === 0 ? <div className="kv-media-empty">
          <span className="kv-media-empty-icon"><Images size={22} strokeWidth={1.5} /></span>
          <h3>{jobs.length ? text('没有匹配的作品', 'No matching creations') : text('还没有作品', 'No creations yet')}</h3>
          <p>{jobs.length ? text('试试其他关键词或类型。', 'Try another keyword or type.') : text('回到创作页生成第一张吧。', 'Go back and create your first one.')}</p>
        </div> : <div className="kv-media-grid">{visible.map((job) => <div key={job.id} className={`kv-media-card-wrap${selection.has(job.id) ? ' is-selected' : ''}`}>
          <button type="button" className="kv-media-card" aria-haspopup="dialog" onClick={() => setMedia((state) => ({ ...state, selectedId: job.id }))}>
            <div className="kv-media-cover">{job.outputs[0]?.preview ? <img src={job.outputs[0].preview} alt="" loading="lazy" /> : job.status === 'running' ? <LoaderCircle className="animate-spin" size={25} /> : job.request.kind === 'image' ? <Images size={28} /> : <Play size={28} />}
              {job.status !== 'completed' && <span className={`kv-media-badge is-${job.status}`}>{statusLabel(job)}</span>}
            </div><strong>{job.request.prompt}</strong><small>{job.request.aspectRatio} · {new Date(job.createdAt).toLocaleString(zh ? 'zh-CN' : 'en-US', { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })}</small>
          </button>
          <span className="kv-media-selection-mark">
            <IconButton size="sm" shape="circle" role="checkbox" aria-checked={selection.has(job.id)}
              label={text(`选择记录：${job.request.prompt}`, `Select creation: ${job.request.prompt}`)}
              disabled={deleting || job.status === 'running'}
              title={job.status === 'running' ? text('打开详情并停止等待后再删除', 'Open details and stop waiting before deleting') : undefined}
              onClick={() => toggleSelection(job.id)}>{selection.has(job.id) ? <Check size={16} /> : <Circle size={16} />}</IconButton>
          </span>
        </div>)}</div>}
      </div>
    </div>}
    {view === 'history' && selected && <MediaDetail key={selected.id} job={selected} errorMessage={error} cancelling={media.pending.includes(`cancel:${selected.id}`)} statusLabel={statusLabel(selected)}
      onClose={() => setMedia((state) => ({ ...state, selectedId: '' }))} onCancel={() => cancel(selected.id)} {...resultActions(selected)} />}
    {view === 'create' && <div className="kv-media-dock">
      {media.editHint && <p role="status" className="kv-field-hint kv-media-edit-hint">{media.editHint}</p>}
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
