import { ErrorNotice } from '../components/AppNotice'
import { useModelDatabase } from '../data/useModelDatabase'
import { useCallback, useEffect, useRef, useState, type CSSProperties } from 'react'
import { open, save } from '@tauri-apps/plugin-dialog'
import { createWindowStore, useWindowStore } from '../utils/windowStore'
import { Check, Circle, Film, ImagePlus, Images, Info, LoaderCircle, Play, RefreshCw, Search, Sparkles, Timer, Trash2, X } from 'lucide-react'
import { mediaStationApi, type MediaJob, type MediaKind, type MediaRequest } from '../api/mediaStation'
import { getSettingsCached } from '../api/settingsCache'
import { isTauriRuntime, type ModelProvider } from '../api/tauri'
import { Button, IconButton } from '../components/Button'
import { confirmDialog } from '../components/dialogQueue'
import { Input, Select, TextArea } from '../settings/public/controls'
import { useLang } from '../components/i18n'
import { resolveModelInfo } from '../data/modelMatching'
import { IdeaArt } from './MediaIdeaArt'
import { CanvasViewfinder, MediaCanvas, RatioGlyph, jobCanvas, type Text } from './MediaCanvas'
import { JobDetails, MediaDetail, MediaResultActions, type ResultActions } from './MediaJobViews'
import { dayLabel, promptHasTerm, ratioOf, timeLabel, togglePromptTerm } from './mediaFormat'
import { FACETS, IDEAS, IDEAS_PER_PAGE, type Idea } from './mediaIdeas'
import { CenterHeader, CenterTabs } from './market/PluginCenterParts'
import { MediaIcon } from '../settings/public/icons'
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

/** Rotates across visits in this window so the page doesn't open on the same four every time. */
let ideaOffset = Math.floor(Math.random() * 3) * IDEAS_PER_PAGE
const RATIOS = ['1:1', '16:9', '9:16', '4:3', '3:4']
const PROMPT_LIMIT = 8000

export function MediaStation({ onOpenSettings }: { onOpenSettings: () => void }) {
  useModelDatabase()
  const zh = useLang() === 'zh'
  const text: Text = (cn, en) => zh ? cn : en
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
  const [providersLoaded, setProvidersLoaded] = useState(false)
  const [listedJobs, setListedJobs] = useState<MediaJob[]>([])
  // 「创作」是默认视图，只展示本次生成；完整历史在单独的「创作记录」视图里。
  const view = media.view
  const [ideaStart, setIdeaStart] = useState(() => ideaOffset)
  const [shuffles, setShuffles] = useState(0)
  const [previewIdea, setPreviewIdea] = useState<Idea | null>(null)
  const [fillPulse, setFillPulse] = useState(0)
  const [measured, setMeasured] = useState<{ id: string; ratio: number } | null>(null)
  const [filter, setFilter] = useState<'all' | MediaKind>('all')
  const [query, setQuery] = useState('')
  const [loading, setLoading] = useState(desktop)
  const composerRef = useRef<HTMLFormElement>(null)
  const jobs = listedJobs.filter((job) => !media.deletedIds.includes(job.id))
  const selected = jobs.find((j) => j.id === selectedId)
  // 只列出能生成当前类型的供应商和模型。
  const capableProviders = providers.filter((p) => mediaModels(p, form.kind).length > 0)
  const provider = capableProviders.find((p) => p.id === form.providerId)
  const modelChoices = provider ? mediaModels(provider, form.kind) : []
  const running = jobs.filter((j) => j.status === 'running').length
  const matching = jobs.filter((j) => j.request.prompt.toLowerCase().includes(query.toLowerCase()))
  const visible = matching.filter((j) => filter === 'all' || j.request.kind === filter).sort((a, b) => b.createdAt - a.createdAt)
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
      setProvidersLoaded(true)
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
    setPreviewIdea(null)
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
        message: text(`永久删除 ${unique.length} 条创作记录及其本机源文件？此操作无法撤销，已导出的副本不受影响。其他位置使用这些源文件的参考图可能失效。`, `Permanently delete ${unique.length} creation record(s) and their local source files? This cannot be undone. Exported copies are safe. References to these source files elsewhere may become invalid.`),
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
  const nextIdeas = () => { setShuffles((n) => n + 1); setIdeaStart((start) => (ideaOffset = (start + IDEAS_PER_PAGE) % ideaPool.length)) }
  useEffect(() => () => { ideaOffset = (ideaOffset + IDEAS_PER_PAGE) % IDEAS.image.length }, [])
  const ideaText = (idea: Idea) => text(idea.prompt[0], idea.prompt[1])
  const hovered = previewIdea && shownIdeas.includes(previewIdea) ? previewIdea : undefined
  // A scene stays marked while modifiers are layered onto it.
  const picked = ideaPool.find((idea) => form.prompt.startsWith(ideaText(idea).replace(/[。.]$/, '')))
  function applyIdea(idea: Idea) {
    update({ prompt: ideaText(idea), aspectRatio: idea.ratio })
    setFillPulse((n) => n + 1)
    composerRef.current?.querySelector('textarea')?.focus()
  }
  function toggleTerm(term: string) {
    update({ prompt: togglePromptTerm(mediaWindow.getSnapshot().draft.prompt, term, zh) })
    setFillPulse((n) => n + 1)
  }
  /** Actions on a result; reusing it brings the parameters back to the create view. */
  const resultActions = (job: MediaJob): ResultActions => ({
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

  const blocker = !desktop ? text('请在桌面端生成', 'Generate in the desktop app')
    : !provider || !form.model.trim() ? text('先选择支持该类型的供应商和模型', 'Choose a capable provider and model first')
    : !form.prompt.trim() ? text('先描述想要的画面', 'Describe what you want first')
    : form.prompt.length > PROMPT_LIMIT ? text(`描述不能超过 ${PROMPT_LIMIT} 字`, `Keep the prompt within ${PROMPT_LIMIT} characters`)
    : running >= 3 ? text('最多同时生成 3 个，请等待其中一个完成', 'Up to 3 generations run at once')
    : ''
  const canGenerate = !submitting && !blocker
  const modKey = typeof navigator !== 'undefined' && navigator.platform.startsWith('Mac') ? '⌘' : 'Ctrl'

  // The create stage keeps one canvas mounted: idle viewfinder, or the latest creation's state.
  const stageCanvas = latest
    ? jobCanvas(latest, statusLabel(latest), measured?.id === latest.id ? measured.ratio : undefined, (ratio) => setMeasured({ id: latest.id, ratio }))
    : {
      state: 'idle' as const,
      ratio: ratioOf(hovered?.ratio ?? form.aspectRatio),
      content: <CanvasViewfinder kind={form.kind} ratio={hovered?.ratio ?? form.aspectRatio} duration={form.duration} art={(hovered ?? picked)?.art} prompt={form.prompt} text={text} />,
    }

  const ordered = visible
  const selectedIndex = selected ? ordered.findIndex((job) => job.id === selected.id) : -1
  const openJob = (id: string) => setMedia((state) => ({ ...state, selectedId: id }))
  const groups: { label: string; jobs: MediaJob[] }[] = []
  for (const job of visible) {
    const label = dayLabel(job.createdAt, zh)
    if (groups.at(-1)?.label === label) groups.at(-1)!.jobs.push(job)
    else groups.push({ label, jobs: [job] })
  }
  const kindCount = (kind: 'all' | MediaKind) => kind === 'all' ? matching.length : matching.filter((job) => job.request.kind === kind).length
  const filters = [['all', text('全部', 'All')], ['image', text('图片', 'Images')], ['video', text('视频', 'Videos')]] as const
  let cardIndex = 0

  return <section className="kv kv-content kv-media">
    <div className="kv-media-top">
      <CenterHeader
        heading={<h1 className="flex items-center gap-2.5"><MediaIcon size={24} className="text-[var(--color-muted-foreground)]" />{text('媒体站', 'Media studio')}</h1>}
        subtitle={text('用你配置的供应商生成图片与视频，作品只保存在本机。', 'Generate images and video with your own providers. Creations stay on this device.')}
        actions={running > 0 ? <button type="button" className="kv-media-running" onClick={() => setMediaView('history')} data-tauri-drag-region="false">
          <span className="kv-media-live" aria-hidden="true" />{text(`${running} 个生成中`, `${running} generating`)}
        </button> : null}
      />
      <CenterTabs<'image' | 'video' | 'history'>
        value={view === 'history' ? 'history' : form.kind}
        onChange={(tab) => {
          if (tab === 'history') { setMediaView('history'); return }
          setMediaView('create')
          if (tab !== form.kind) changeKind(tab)
        }}
        tabs={[
          { id: 'image', label: text('生图', 'Image') },
          { id: 'video', label: text('生视频', 'Video') },
          { id: 'history', label: text('创作记录', 'Creations'), count: jobs.length },
        ]}
      />
    </div>
    {error && !(view === 'history' && selected) && <ErrorNotice message={error} />}
    {view === 'create' ? <div key="create" className="kv-scroll custom-scrollbar kv-media-workspace">
      <div className={`kv-media-stage is-${latest ? 'job' : 'idle'}`} role={latest ? 'article' : undefined} aria-label={latest ? text('本次生成', 'Latest result') : undefined}>
        <div className="kv-media-brief">
          {latest ? <div key={`job-${latest.id}`} className="kv-media-brief-inner">
            <JobDetails job={latest} text={text} zh={zh} statusLabel={statusLabel(latest)} onCancel={() => cancel(latest.id)} cancelling={media.pending.includes(`cancel:${latest.id}`)}
              headerEnd={<IconButton size="sm" variant="ghost" label={text('收起', 'Dismiss')} onClick={() => updateLatestResult(form.kind, '')}><X size={15} /></IconButton>} />
            <MediaResultActions job={latest} {...resultActions(latest)} />
          </div> : <div key={`idle-${form.kind}`} className="kv-media-brief-inner kv-media-recipe">
            <div className="kv-media-recipe-head">
              <h2>{form.kind === 'image' ? text('想画点什么', 'What would you like to make') : text('想让什么动起来', 'What should move')}</h2>
              <Button size="sm" variant="ghost" onClick={nextIdeas}><RefreshCw key={shuffles} size={13} className={shuffles ? 'kv-media-spin-once' : undefined} />{text('换一批', 'Shuffle')}</Button>
            </div>
            <p className="kv-media-recipe-lede">{form.kind === 'image' ? text('选一个场景起笔，再叠加风格、光线和构图。', 'Start from a scene, then layer style, light and framing.') : text('选一个镜头起笔，再叠加运镜、节奏和风格。', 'Start from a shot, then layer camera, pace and style.')}</p>
            <div className="kv-media-scenes" key={`${form.kind}-${ideaStart}`} onMouseLeave={() => setPreviewIdea(null)}>
              {shownIdeas.map((idea, i) => <button key={idea.art} type="button" className="kv-media-idea" aria-pressed={picked === idea}
                style={{ '--kv-idea-delay': `${i * 35}ms` } as CSSProperties} title={ideaText(idea)}
                onMouseEnter={() => setPreviewIdea(idea)} onFocus={() => setPreviewIdea(idea)} onBlur={() => setPreviewIdea(null)} onClick={() => applyIdea(idea)}>
                <span className="kv-media-idea-art"><IdeaArt name={idea.art} /><span className="kv-media-idea-ratio"><RatioGlyph ratio={idea.ratio} size={10} />{idea.ratio}</span></span>
                <strong>{text(idea.title[0], idea.title[1])}</strong>
              </button>)}
            </div>
            {FACETS[form.kind].map((facet) => <div key={facet.label[1]} className="kv-media-facet" role="group" aria-label={text(facet.label[0], facet.label[1])}>
              <span className="kv-media-facet-label">{text(facet.label[0], facet.label[1])}</span>
              <div className="kv-media-chips">{facet.terms.map((pair) => {
                const term = text(pair[0], pair[1])
                const on = promptHasTerm(form.prompt, term)
                return <button key={pair[1]} type="button" className="kv-media-chip" aria-pressed={on} onClick={() => toggleTerm(term)}>
                  {on && <Check size={11} strokeWidth={2.5} aria-hidden="true" />}{term}
                </button>
              })}</div>
            </div>)}
          </div>}
        </div>
        <MediaCanvas ratio={stageCanvas.ratio} state={stageCanvas.state}>{stageCanvas.content}</MediaCanvas>
      </div>
    </div> : <div key="history" className={`kv-scroll custom-scrollbar kv-media-workspace kv-media-library${selection.size ? ' is-selecting' : ''}`}>
      <div className="kv-media-library-bar">
        <div className="kv-media-chips" role="group" aria-label={text('筛选类型', 'Filter type')}>
          {filters.map(([value, label]) =>
            <button key={value} type="button" className="kv-media-chip is-filter" aria-pressed={filter === value} onClick={() => setFilter(value)}>
              {label}<span className="kv-media-chip-count" aria-hidden="true">{kindCount(value)}</span>
            </button>)}
        </div>
        <span className="kv-media-bar-fill" />
        <label className="kv-media-search">
          <Search size={14} aria-hidden="true" />
          <Input aria-label={text('搜索描述', 'Search prompts')} value={query} onChange={setQuery} placeholder={text('搜索创作描述…', 'Search prompts…')} />
        </label>
        <IconButton size="sm" variant="ghost" label={text('刷新记录', 'Refresh')} onClick={refreshHistory} disabled={!desktop}><RefreshCw size={15} /></IconButton>
        <Button size="sm" variant="ghost" disabled={deleting || !deletableVisible.length} onClick={() => setMedia((state) => ({ ...state, selection: [...new Set([...state.selection, ...deletableVisible.map((job) => job.id)])] }))}>{text('全选当前结果', 'Select all filtered results')}</Button>
      </div>
      {loading ? <div className="kv-media-wall is-loading" aria-hidden="true">
        <p role="status" className="sr-only">{text('正在读取记录…', 'Loading history…')}</p>
        {[1, 1.78, 0.75, 1, 1.33, 0.56, 1.78, 1].map((ratio, i) => <span key={i} className="kv-media-skeleton kv-skeleton" style={{ '--kv-card-ratio': ratio } as CSSProperties} />)}
      </div> : visible.length === 0 ? <div className="kv-media-empty">
        <span className="kv-media-empty-art" aria-hidden="true"><i /><i /><i /></span>
        <h3>{jobs.length ? text('没有匹配的作品', 'No matching creations') : text('还没有作品', 'No creations yet')}</h3>
        <p>{jobs.length ? text('试试其他关键词或类型。', 'Try another keyword or type.') : text('生成的图片和视频都会收在这里。', 'Every image and video you generate lands here.')}</p>
        {!jobs.length && <Button size="sm" onClick={() => setMediaView('create')}><Sparkles size={14} />{text('去创作', 'Start creating')}</Button>}
      </div> : groups.map((group) => <section key={group.label} className="kv-media-day" aria-label={group.label}>
        <h3 className="kv-media-day-title">{group.label}<span>{group.jobs.length}</span></h3>
        <div className="kv-media-wall">{group.jobs.map((job) => {
          const preview = job.outputs[0]?.preview
          const checked = selection.has(job.id)
          return <div key={job.id} className={`kv-media-card-wrap${checked ? ' is-selected' : ''}${preview ? '' : ' is-textual'}`}
            style={{ '--kv-card-ratio': ratioOf(job.request.aspectRatio), '--kv-card-delay': `${Math.min(cardIndex++, 14) * 28}ms` } as CSSProperties}>
            <button type="button" className="kv-media-card" aria-haspopup="dialog" onClick={() => openJob(job.id)}>
              <div className="kv-media-cover">
                {preview ? <img src={preview} alt="" loading="lazy" />
                  : job.status === 'running' ? <span className="kv-media-cover-develop" aria-hidden="true" />
                    : job.request.kind === 'image' ? <Images size={22} strokeWidth={1.4} aria-hidden="true" /> : <Film size={22} strokeWidth={1.4} aria-hidden="true" />}
                {job.request.kind === 'video' && preview && <span className="kv-media-cover-kind" aria-hidden="true"><Play size={11} /></span>}
                {job.status !== 'completed' && <span className={`kv-media-badge is-${job.status}`}>{statusLabel(job)}</span>}
              </div>
              <div className="kv-media-caption"><strong>{job.request.prompt}</strong><small>{timeLabel(job.createdAt, zh)} · {job.request.aspectRatio}</small></div>
            </button>
            <span className="kv-media-selection-mark">
              <IconButton size="sm" shape="circle" role="checkbox" aria-checked={checked}
                label={text(`选择记录：${job.request.prompt}`, `Select creation: ${job.request.prompt}`)}
                disabled={deleting || job.status === 'running'}
                title={job.status === 'running' ? text('打开详情并停止等待后再删除', 'Open details and stop waiting before deleting') : undefined}
                onClick={() => toggleSelection(job.id)}>{checked ? <Check size={14} /> : <Circle size={14} />}</IconButton>
            </span>
          </div>
        })}</div>
      </section>)}
      {selection.size > 0 && <div className="kv-media-selection-bar" aria-label={text('记录选择', 'Record selection')}>
        <span role="status">{text(`已选择 ${selection.size} 条`, `${selection.size} selected`)}</span>
        <Button size="sm" variant="ghost" onClick={() => setMedia((state) => ({ ...state, selection: [] }))}>{text('取消选择', 'Clear')}</Button>
        <Button size="sm" variant="danger" disabled={deleting || !selection.size} onClick={() => void deleteJobs([...selection])}><Trash2 size={14} />{text('删除所选', 'Delete selected')}</Button>
      </div>}
    </div>}
    {view === 'history' && selected && <MediaDetail job={selected} zh={zh} errorMessage={error} cancelling={media.pending.includes(`cancel:${selected.id}`)} statusLabel={statusLabel(selected)}
      position={selectedIndex >= 0 ? `${selectedIndex + 1} / ${ordered.length}` : ''}
      onPrev={selectedIndex > 0 ? () => openJob(ordered[selectedIndex - 1].id) : undefined}
      onNext={selectedIndex >= 0 && selectedIndex < ordered.length - 1 ? () => openJob(ordered[selectedIndex + 1].id) : undefined}
      onClose={() => setMedia((state) => ({ ...state, selectedId: '' }))} onCancel={() => cancel(selected.id)} actions={resultActions(selected)} />}
    {view === 'create' && <div className="kv-media-dock">
      {media.editHint && <div className="kv-media-edit-hint">
        <Info size={14} aria-hidden="true" /><p role="status">{media.editHint}</p>
        <IconButton size="xs" variant="ghost" label={text('关闭提示', 'Dismiss hint')} onClick={() => setMedia((state) => ({ ...state, editHint: '' }))}><X size={12} /></IconButton>
      </div>}
      {desktop && providersLoaded && capableProviders.length === 0 && <div className="kv-media-setup-hint">
        <Info size={14} aria-hidden="true" />
        <p>{form.kind === 'image' ? text('还没有启用支持生图的模型。', 'No enabled model can generate images yet.') : text('还没有启用支持生视频的模型。', 'No enabled model can generate video yet.')}</p>
        <Button size="sm" variant="ghost" onClick={onOpenSettings}>{text('去设置', 'Open settings')}</Button>
      </div>}
      <form ref={composerRef} className="kv-media-composer" onSubmit={(e) => { e.preventDefault(); if (canGenerate) void generate() }}>
        {fillPulse > 0 && <span key={fillPulse} className="kv-media-composer-flash" aria-hidden="true" />}
        {form.referencePaths.length > 0 && <div className="kv-media-references">
          {form.referencePaths.map((path) => <div className="kv-chip kv-media-reference-file" key={path}>
            <ImagePlus size={13} aria-hidden="true" /><span title={path}>{path.split(/[\\/]/).pop()}</span>
            <IconButton size="xs" label={text('移除参考图', 'Remove reference')} onClick={() => update({ referencePaths: form.referencePaths.filter((p) => p !== path) })}><X size={12} /></IconButton>
          </div>)}
        </div>}
        <label className="kv-media-prompt-field">
          <span className="sr-only">{text('创作描述', 'Prompt')}</span>
          <TextArea variant="plain" value={form.prompt} onChange={(prompt) => update({ prompt })} rows={2} maxRows={10} className="kv-media-prompt-input"
            onKeyDown={(e) => { if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); e.currentTarget.form?.requestSubmit() } }}
            placeholder={form.kind === 'image' ? text('你想看见怎样的画面？描述主体、光线、风格，或从一张参考图开始…', 'What would you like to see? Describe the subject, light and style…') : text('让画面如何运动？描述动作、镜头与节奏，或添加一张首帧…', 'Describe the motion, camera and rhythm, or add a first frame…')} />
        </label>
        <div className="kv-media-toolbar">
          <IconButton size="sm" variant="ghost" label={form.kind === 'image' ? text('添加参考图（最多 4 张）', 'Add references (up to 4)') : text('添加首帧图片', 'Add first frame')} onClick={() => void pickReferences()} disabled={!desktop}><ImagePlus size={16} /></IconButton>
          <span className="kv-media-toolbar-sep" aria-hidden="true" />
          <Select className="kv-media-provider" size="sm" triggerLabel={capableProviders.find((p) => p.id === form.providerId)?.name ?? text('供应商', 'Provider')} ariaLabel={text('供应商', 'Provider')} value={form.providerId} options={capableProviders.map((p) => ({ value: p.id, label: p.name }))} onChange={(providerId) => update({ providerId, model: '' })} />
          <Select className="kv-media-model" size="sm" triggerLabel={form.model || text('模型', 'Model')} ariaLabel={text('模型', 'Model')} value={form.model} onChange={(model) => update({ model })}
            options={modelChoices.map((model) => ({ value: model, label: model }))} />
          <span className="kv-media-toolbar-sep" aria-hidden="true" />
          <Select className="kv-media-ratio" size="sm" triggerIcon={<RatioGlyph ratio={form.aspectRatio} />} triggerLabel={form.aspectRatio} ariaLabel={text('画幅', 'Aspect ratio')} value={form.aspectRatio} onChange={(aspectRatio) => update({ aspectRatio })} options={RATIOS.map((value) => ({ value, label: value }))} />
          {form.kind === 'video' && <Select className="kv-media-duration" size="sm" triggerIcon={<Timer size={14} />} triggerLabel={`${form.duration} ${text('秒', 'sec')}`} ariaLabel={text('时长', 'Duration')} value={String(form.duration)} onChange={(duration) => update({ duration: Number(duration) })} options={[5, 10, 15].map((n) => ({ value: String(n), label: `${n} ${text('秒', 'sec')}` }))} />}
          <span className="kv-media-toolbar-fill" />
          {form.prompt.length > PROMPT_LIMIT - 1000 && <span className={`kv-media-counter${form.prompt.length > PROMPT_LIMIT ? ' is-over' : ''}`}>{form.prompt.length} / {PROMPT_LIMIT}</span>}
          {/* Like the chat composer's send key, generating is the composer's own control rather than a shared action button. */}
          <span className="kv-media-generate-wrap" title={submitting ? undefined : blocker || undefined}>
            <button type="submit" className={`kv-media-generate${canGenerate ? ' is-ready' : ''}${submitting ? ' is-busy' : ''}`} disabled={!canGenerate} data-tauri-drag-region="false">
              {submitting ? <LoaderCircle size={14} className="animate-spin" aria-hidden="true" /> : <Sparkles size={14} aria-hidden="true" />}
              <span>{text('开始生成', 'Generate')}</span>
            </button>
          </span>
        </div>
      </form>
      <p className="kv-field-hint kv-media-composer-caption">
        {[
          !desktop && text('预览模式 · 请在桌面端生成', 'Preview · Generate in the desktop app'),
          form.kind === 'video' ? text('每次生成 1 段 · 按供应商计费', 'One video per request · Provider charges apply') : text('每次生成 1 张 · 按供应商计费', 'One image per request · Provider charges apply'),
          text('结果自动保存在本机', 'Saved to your device'),
        ].filter(Boolean).join(' · ')}
        <kbd>{modKey} ↵</kbd>{text('生成', 'to generate')}
      </p>
    </div>}
  </section>
}
