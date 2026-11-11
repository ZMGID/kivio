// 媒体站的「画布」：一个画框贯穿灵感预览、生成中与结果。画框尺寸随比例缓动，
// 所以悬停灵感、切换比例、开始生成与结果显影读起来是同一张画布在变化。
import { useEffect, useState, type CSSProperties, type ReactNode } from 'react'
import { Film, ImageOff, Images, LoaderCircle } from 'lucide-react'
import { mediaStationApi, type MediaJob, type MediaKind } from '../api/mediaStation'
import { IdeaArt, type IdeaArtName } from './MediaIdeaArt'
import { ratioOf } from './mediaFormat'

export type Text = (cn: string, en: string) => string
export type CanvasState = 'idle' | 'running' | 'done' | 'empty'

/** A tiny outline of the aspect ratio, so ratio labels read as shapes. */
export function RatioGlyph({ ratio, size = 14 }: { ratio: string; size?: number }) {
  const r = ratioOf(ratio)
  const box = size - 2
  const w = r >= 1 ? box : box * r
  const h = r >= 1 ? box / r : box
  return <svg className="kv-media-ratio-glyph" width={size} height={size} viewBox={`0 0 ${size} ${size}`} aria-hidden="true">
    <rect x={(size - w) / 2} y={(size - h) / 2} width={w} height={h} rx={1.6} fill="none" stroke="currentColor" strokeWidth={1.3} />
  </svg>
}

export function MediaCanvas({ ratio, state, children }: { ratio: number; state: CanvasState; children: ReactNode }) {
  return <div className="kv-media-canvas-wrap">
    <div className={`kv-media-canvas is-${state}`} style={{ '--kv-media-ratio': ratio } as CSSProperties}>
      <span className="kv-media-canvas-mark is-tl" aria-hidden="true" />
      <span className="kv-media-canvas-mark is-tr" aria-hidden="true" />
      <span className="kv-media-canvas-mark is-bl" aria-hidden="true" />
      <span className="kv-media-canvas-mark is-br" aria-hidden="true" />
      {children}
    </div>
  </div>
}

/** Empty canvas: rule-of-thirds guides, then the hovered idea drawing itself, the draft prompt, or a hint. */
export function CanvasViewfinder({ kind, ratio, duration, art, prompt, text }: { kind: MediaKind; ratio: string; duration: number; art?: IdeaArtName; prompt: string; text: Text }) {
  const brief = prompt.trim()
  return <div className="kv-media-viewfinder" aria-hidden="true">
    <span className="kv-media-thirds" />
    {art
      ? <div key={`art-${art}`} className="kv-media-canvas-art"><IdeaArt name={art} /></div>
      : brief
        ? <p key="brief" className="kv-media-canvas-brief">{brief}</p>
        : <div key="hint" className="kv-media-canvas-hint">
          <span className="kv-media-crosshair" />
          <p>{kind === 'image' ? text('画面会出现在这里', 'Your image appears here') : text('镜头会在这里播放', 'Your shot plays here')}</p>
        </div>}
    <span className="kv-media-canvas-tag"><RatioGlyph ratio={ratio} size={12} />{ratio}</span>
    {kind === 'video' && <span className="kv-media-timeline">
      <span className="kv-media-timeline-track">{Array.from({ length: duration + 1 }, (_, i) => <i key={i} />)}</span>
      <em>0:{String(duration).padStart(2, '0')}</em>
    </span>}
  </div>
}

function useElapsed(since: number) {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000)
    return () => window.clearInterval(timer)
  }, [])
  const seconds = Math.max(0, Math.floor((now - since) / 1000))
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`
}

/** Running: a slowly drifting light develops behind the frame while the elapsed time ticks. */
function CanvasDeveloping({ job }: { job: MediaJob }) {
  const elapsed = useElapsed(job.createdAt)
  const Icon = job.request.kind === 'video' ? Film : Images
  return <div className="kv-media-develop" aria-hidden="true">
    <span className="kv-media-develop-light" />
    <span className="kv-media-develop-sweep" />
    <span className="kv-media-develop-core">
      <span className="kv-media-develop-ring">
        <svg className="kv-media-develop-arc" viewBox="0 0 48 48"><circle className="track" cx="24" cy="24" r="21" /><circle className="arc" cx="24" cy="24" r="21" /></svg>
        <Icon size={16} strokeWidth={1.6} />
      </span>
      <span className="kv-media-develop-time">{elapsed}</span>
    </span>
  </div>
}

/** Reads one output from the backend; the media fades in from a blur once decoded, like a print developing. */
function OutputPreview({ job, index, onMeasure }: { job: MediaJob; index: number; onMeasure?: (ratio: number) => void }) {
  const [url, setUrl] = useState('')
  const [error, setError] = useState('')
  const [loaded, setLoaded] = useState(false)
  const output = job.outputs[index]
  useEffect(() => {
    let active = true
    let objectUrl = ''
    setUrl(''); setError(''); setLoaded(false)
    mediaStationApi.read(job.id, index).then((bytes) => {
      if (!active) return
      objectUrl = URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type: output.mimeType }))
      setUrl(objectUrl)
    }).catch((e) => { if (active) setError(String(e)) })
    return () => { active = false; if (objectUrl) URL.revokeObjectURL(objectUrl) }
  }, [job.id, index, output.mimeType])
  if (error) return <p role="alert" className="kv-media-output-error">{error}</p>
  if (!url) return <span className="kv-media-output-loading"><LoaderCircle className="animate-spin" size={18} aria-label="Loading" /></span>
  const className = `kv-media-output-media${loaded ? ' is-loaded' : ''}`
  return output.mimeType.startsWith('video/')
    ? <video className={className} src={url} controls preload="auto" onLoadedMetadata={(event) => {
      // WebKit can leave an unplayed video blank until a seek decodes its first frame.
      const video = event.currentTarget
      if (video.currentTime === 0 && video.duration > 0) video.currentTime = Math.min(0.001, video.duration / 2)
      if (video.videoWidth && video.videoHeight) onMeasure?.(video.videoWidth / video.videoHeight)
      setLoaded(true)
    }} />
    : <img className={className} src={url} alt={job.request.prompt} onLoad={(event) => {
      const image = event.currentTarget
      if (image.naturalWidth && image.naturalHeight) onMeasure?.(image.naturalWidth / image.naturalHeight)
      setLoaded(true)
    }} />
}

/** Frame state, ratio and contents for a creation. Returned as data so the caller keeps a single canvas mounted
 *  and only swaps what is inside it. Multiple outputs tile two per row; one output adopts its measured ratio. */
// eslint-disable-next-line react-refresh/only-export-components -- returns canvas JSX so callers keep one frame mounted
export function jobCanvas(job: MediaJob, statusLabel: string, measured: number | undefined, onMeasure: (ratio: number) => void) {
  const count = job.outputs.length
  const requested = ratioOf(job.request.aspectRatio)
  const ratio = count > 1 ? requested * Math.min(count, 2) / Math.ceil(count / 2) : measured ?? requested
  const state: CanvasState = job.status === 'running' ? 'running' : count ? 'done' : 'empty'
  const content = state === 'running'
    ? <CanvasDeveloping key={`run-${job.id}`} job={job} />
    : state === 'done'
      ? <div key={`out-${job.id}`} className={`kv-media-outputs is-${Math.min(count, 4)}`}>
        {job.outputs.map((output, index) => <div key={output.name} className="kv-media-output">
          <OutputPreview job={job} index={index} onMeasure={count === 1 ? onMeasure : undefined} />
        </div>)}
      </div>
      : <div key={`empty-${job.id}`} className="kv-media-canvas-empty" aria-hidden="true">
        {job.request.kind === 'video' ? <Film size={22} strokeWidth={1.4} /> : <ImageOff size={22} strokeWidth={1.4} />}
        <span>{statusLabel}</span>
      </div>
  return { state, ratio, content }
}
