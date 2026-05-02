import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { flushSync } from 'react-dom'
import { Loader2, Copy, Check, Square, Image as ImageIcon, ArrowUp, History as HistoryIcon, ChevronDown, Brain, MousePointer2 } from 'lucide-react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { api, type LensStreamPayload, type LensTranslateStreamPayload, type LensWindowInfo, type ExplainMessage } from './api/tauri'
import ReactMarkdown from 'react-markdown'
import remarkMath from 'remark-math'
import rehypeKatex from 'rehype-katex'
import 'katex/dist/katex.min.css'
import { i18n, type Lang } from './settings/i18n'
import { copyToClipboard } from './utils/clipboard'

type Stage = 'select' | 'ready' | 'answering' | 'translating' | 'translated'
type Mode = 'chat' | 'translate'

/** 解析 webview hash query：'#lens?mode=translate' → 'translate' */
function readModeFromHash(): Mode {
  if (typeof window === 'undefined') return 'chat'
  const hash = window.location.hash || ''
  const q = hash.indexOf('?')
  if (q < 0) return 'chat'
  const params = new URLSearchParams(hash.slice(q + 1))
  return params.get('mode') === 'translate' ? 'translate' : 'chat'
}
type Point = { x: number; y: number }
type BarRect = { x: number; y: number; width: number }
type CapturedFrame = { x: number; y: number; width: number; height: number; label: string }
type Arrow = {
  x1: number
  y1: number
  x2: number
  y2: number
}

const ARROW_COLOR = '#ff3b30'
const ARROW_MIN_DRAG_PX = 8
const ARROW_HEAD_ANGLE_DEG = 30
type HistoryItem = {
  id: string                   // imageId（恢复时复用，重新提问会用同一张图）
  imagePreview: string         // base64 data URL
  appLabel: string
  messages: ExplainMessage[]   // 完整多轮对话
  capturedFrame: CapturedFrame | null
  timestamp: number
}

const HISTORY_MAX = 20
const HISTORY_STORAGE_KEY = 'kivio:lens-history:v1'
const HISTORY_STORAGE_KEY_LEGACY = 'keylingo:lens-history:v1'  // v2.4.5 之前的 key,启动时一次性迁移
const HISTORY_THUMB_SIZE = 96     // 历史记录缩略图边长（px），原始截图压成这个尺寸再持久化

const READY_BAR_H = 56            // 对话栏单行高度（与字号绑定，不随屏幕变）
const ANCHOR_GAP = 12              // 对话栏与选区之间的水平间距
const DRAG_THRESHOLD = 5
/** Approximate CSS cubic-bezier(x1,y1,x2,y2): given linear time t in [0,1] return eased progress.
 *  用 bisection 解 bezier_x(u)=t 求出参数 u，再求 bezier_y(u)。30 次足够单像素精度。 */
function cubicBezier(t: number, x1: number, y1: number, x2: number, y2: number): number {
  if (t <= 0) return 0
  if (t >= 1) return 1
  let lo = 0, hi = 1
  for (let i = 0; i < 30; i++) {
    const u = (lo + hi) / 2
    const x = 3 * (1 - u) * (1 - u) * u * x1 + 3 * (1 - u) * u * u * x2 + u * u * u
    if (x < t) lo = u
    else hi = u
  }
  const u = (lo + hi) / 2
  return 3 * (1 - u) * (1 - u) * u * y1 + 3 * (1 - u) * u * u * y2 + u * u * u
}

const TRANSITION_MS = 380
const FLOATING_PADDING = 0
const FLOATING_GAP = 8

/** Canvas 缩放截图为小缩略图，避免历史记录把整张原图（几 MB）写进 localStorage */
async function makeThumbnail(dataUrl: string, maxSize: number): Promise<string> {
  if (!dataUrl) return ''
  return new Promise((resolve) => {
    const img = new Image()
    img.onload = () => {
      const ratio = Math.min(maxSize / img.width, maxSize / img.height, 1)
      const w = Math.max(1, Math.round(img.width * ratio))
      const h = Math.max(1, Math.round(img.height * ratio))
      const canvas = document.createElement('canvas')
      canvas.width = w
      canvas.height = h
      const ctx = canvas.getContext('2d')
      if (!ctx) { resolve(dataUrl); return }
      ctx.drawImage(img, 0, 0, w, h)
      try { resolve(canvas.toDataURL('image/jpeg', 0.7)) }
      catch { resolve(dataUrl) }
    }
    img.onerror = () => resolve(dataUrl)
    img.src = dataUrl
  })
}

function drawArrow(
  ctx: CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D,
  x1: number,
  y1: number,
  x2: number,
  y2: number,
  lineWidth: number,
) {
  const dx = x2 - x1
  const dy = y2 - y1
  const len = Math.hypot(dx, dy)
  if (len < 1) return

  const headSize = lineWidth * 4
  const angle = Math.atan2(dy, dx)
  const headAngle = (ARROW_HEAD_ANGLE_DEG * Math.PI) / 180

  // 箭杆终点回退一格,避免三角覆盖时尾端有缺口
  const shaftEndX = x2 - Math.cos(angle) * (headSize * 0.6)
  const shaftEndY = y2 - Math.sin(angle) * (headSize * 0.6)

  ctx.save()
  ctx.strokeStyle = ARROW_COLOR
  ctx.fillStyle = ARROW_COLOR
  ctx.lineWidth = lineWidth
  ctx.lineCap = 'round'
  ctx.lineJoin = 'round'

  ctx.beginPath()
  ctx.moveTo(x1, y1)
  ctx.lineTo(shaftEndX, shaftEndY)
  ctx.stroke()

  // 三角箭头
  const wing1X = x2 - Math.cos(angle - headAngle) * headSize
  const wing1Y = y2 - Math.sin(angle - headAngle) * headSize
  const wing2X = x2 - Math.cos(angle + headAngle) * headSize
  const wing2Y = y2 - Math.sin(angle + headAngle) * headSize
  ctx.beginPath()
  ctx.moveTo(x2, y2)
  ctx.lineTo(wing1X, wing1Y)
  ctx.lineTo(wing2X, wing2Y)
  ctx.closePath()
  ctx.fill()

  ctx.restore()
}

async function composeAnnotatedImage(
  imageDataUrl: string,
  arrows: Arrow[],
  frameWidth: number,
  frameHeight: number,
): Promise<string> {
  const img = await new Promise<HTMLImageElement>((resolve, reject) => {
    const el = new Image()
    el.onload = () => resolve(el)
    el.onerror = () => reject(new Error('failed to load image for compose'))
    el.src = imageDataUrl
  })

  const canvas = new OffscreenCanvas(img.naturalWidth, img.naturalHeight)
  const ctx = canvas.getContext('2d')
  if (!ctx) throw new Error('OffscreenCanvas 2d context unavailable')

  ctx.drawImage(img, 0, 0)

  // 逻辑像素 → 物理像素的等比缩放
  // capturedFrame.width 是逻辑像素;PNG 是物理像素 → naturalWidth 大于等于 width
  const scaleX = frameWidth > 0 ? img.naturalWidth / frameWidth : 1
  const scaleY = frameHeight > 0 ? img.naturalHeight / frameHeight : 1
  const lineWidth = Math.max(3, img.naturalWidth / 400)

  for (const a of arrows) {
    drawArrow(
      ctx,
      a.x1 * scaleX,
      a.y1 * scaleY,
      a.x2 * scaleX,
      a.y2 * scaleY,
      lineWidth,
    )
  }

  const blob = await canvas.convertToBlob({ type: 'image/png' })
  const buf = await blob.arrayBuffer()
  let binary = ''
  const bytes = new Uint8Array(buf)
  const chunkSize = 0x8000
  for (let i = 0; i < bytes.length; i += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunkSize))
  }
  return btoa(binary)
}

function ArrowSvg({ arrow }: { arrow: Arrow }) {
  const { x1, y1, x2, y2 } = arrow
  const dx = x2 - x1
  const dy = y2 - y1
  const len = Math.hypot(dx, dy)
  if (len < 1) return null

  // SVG 在逻辑像素坐标系下渲染 → 线宽用屏幕粗细,合成时再按 PNG 物理像素重算
  const lineWidth = 4
  const headSize = lineWidth * 4
  const angle = Math.atan2(dy, dx)
  const headAngle = (ARROW_HEAD_ANGLE_DEG * Math.PI) / 180

  const shaftEndX = x2 - Math.cos(angle) * (headSize * 0.6)
  const shaftEndY = y2 - Math.sin(angle) * (headSize * 0.6)
  const wing1X = x2 - Math.cos(angle - headAngle) * headSize
  const wing1Y = y2 - Math.sin(angle - headAngle) * headSize
  const wing2X = x2 - Math.cos(angle + headAngle) * headSize
  const wing2Y = y2 - Math.sin(angle + headAngle) * headSize

  return (
    <g>
      <line
        x1={x1}
        y1={y1}
        x2={shaftEndX}
        y2={shaftEndY}
        stroke={ARROW_COLOR}
        strokeWidth={lineWidth}
        strokeLinecap="round"
      />
      <polygon
        points={`${x2},${y2} ${wing1X},${wing1Y} ${wing2X},${wing2Y}`}
        fill={ARROW_COLOR}
      />
    </g>
  )
}

/** 从 localStorage 读历史。失败 / 损坏数据 → 空数组。
    一次性迁移：keylingo:lens-history:v1 → kivio:lens-history:v1 */
function loadHistoryFromStorage(): HistoryItem[] {
  try {
    let raw = localStorage.getItem(HISTORY_STORAGE_KEY)
    if (!raw) {
      const legacy = localStorage.getItem(HISTORY_STORAGE_KEY_LEGACY)
      if (legacy) {
        localStorage.setItem(HISTORY_STORAGE_KEY, legacy)
        localStorage.removeItem(HISTORY_STORAGE_KEY_LEGACY)
        raw = legacy
      } else {
        return []
      }
    }
    const parsed = JSON.parse(raw)
    if (!Array.isArray(parsed)) return []
    return parsed.slice(0, HISTORY_MAX)
  } catch {
    return []
  }
}

/** 把历史写回 localStorage。失败时只 console.error 不抛（quota 满 / 隐私模式等） */
function saveHistoryToStorage(history: HistoryItem[]) {
  try {
    localStorage.setItem(HISTORY_STORAGE_KEY, JSON.stringify(history))
  } catch (err) {
    console.error('[lens-history] localStorage save failed:', err)
  }
}

type Metrics = {
  READY_W: number
  SELECT_W: number
  ANSWER_H: number
  SELECT_BOTTOM_OFFSET: number
}

/** 多屏适配：基于当前 viewport 算"比例 + 上下限"，不同分辨率/屏幕大小都能落到舒适区间。 */
const computeMetrics = (vw: number, vh: number): Metrics => ({
  READY_W: Math.round(Math.max(420, Math.min(720, vw * 0.42))),
  SELECT_W: Math.round(Math.max(480, Math.min(820, vw * 0.5))),
  ANSWER_H: Math.round(Math.max(220, Math.min(480, vh * 0.45))),
  SELECT_BOTTOM_OFFSET: Math.round(Math.max(80, Math.min(160, vh * 0.13))),
})

/** 计算 select 态对话栏在 webview 内的位置（webview 全屏，所以用 viewport 大小） */
const computeSelectBar = (vw: number, vh: number, m: Metrics): BarRect => ({
  x: Math.round(vw / 2 - m.SELECT_W / 2),
  y: Math.round(vh - m.SELECT_BOTTOM_OFFSET - READY_BAR_H),
  width: m.SELECT_W,
})

/** 估算 token 数：ASCII 按 ~4 字符/token；非 ASCII（中日韩等）按 1 字符/token */
function estimateTokens(text: string): number {
  let ascii = 0
  for (let i = 0; i < text.length; i++) {
    if (text.charCodeAt(i) < 128) ascii++
  }
  const nonAscii = text.length - ascii
  return Math.ceil(ascii / 4 + nonAscii)
}

function formatTokens(n: number): string {
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`
  return `${n}`
}

/** 思维链区块（Claude Code 风格）：默认折叠，header 显示耗时 + token 估算。点击展开/收起。 */
function ThinkingBlock({
  reasoning,
  active,
  thinkingLabel,
  thoughtLabel,
}: {
  reasoning: string
  active: boolean
  thinkingLabel: string
  thoughtLabel: string
}) {
  const [open, setOpen] = useState(false)
  const [finalDurationMs, setFinalDurationMs] = useState<number | null>(null)
  const [now, setNow] = useState(() => Date.now())
  const startRef = useRef<number | null>(null)
  const bodyRef = useRef<HTMLDivElement>(null)

  // 跟踪 active：开始计时 / 停止计时并锁定最终耗时
  useEffect(() => {
    if (active && startRef.current === null) {
      startRef.current = Date.now()
      setFinalDurationMs(null)
    } else if (!active && startRef.current !== null) {
      setFinalDurationMs(Date.now() - startRef.current)
      startRef.current = null
    }
  }, [active])

  // active 期间每秒刷一次 now，header 显示走秒效果
  useEffect(() => {
    if (!active) return
    const id = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(id)
  }, [active])

  // 展开时自动滚到底，方便流式中跟读
  useEffect(() => {
    if (open && active && bodyRef.current) {
      bodyRef.current.scrollTop = bodyRef.current.scrollHeight
    }
  }, [reasoning, active, open])

  const elapsedMs = active && startRef.current
    ? now - startRef.current
    : finalDurationMs
  const seconds = elapsedMs !== null ? Math.max(1, Math.round(elapsedMs / 1000)) : null
  // O(n) 字符遍历，按 reasoning 长度记忆 — 避免多轮 history 中每次 delta 重渲全部 ThinkingBlock 都重算
  const tokens = useMemo(() => formatTokens(estimateTokens(reasoning)), [reasoning])

  return (
    <div className="not-prose mb-2 rounded-lg border border-black/[0.06] dark:border-white/[0.08] bg-black/[0.025] dark:bg-white/[0.03]">
      <button
        type="button"
        onClick={() => setOpen(o => !o)}
        className="w-full flex items-center gap-1.5 px-2.5 py-1.5 text-[11.5px] text-neutral-500 dark:text-neutral-400 hover:text-neutral-700 dark:hover:text-neutral-200 transition-colors"
      >
        {active
          ? <Loader2 className="animate-spin" size={11} />
          : <Brain size={11} strokeWidth={1.75} />}
        <span className="font-medium">{active ? thinkingLabel : thoughtLabel}</span>
        <span className="text-neutral-400 dark:text-neutral-500">
          {seconds !== null && <> · {seconds}s</>}
          <> · ~{tokens} tokens</>
        </span>
        <ChevronDown size={11} strokeWidth={2} className={`ml-auto transition-transform ${open ? 'rotate-180' : ''}`} />
      </button>
      {open && (
        <div
          ref={bodyRef}
          className="px-2.5 pb-2 max-h-[160px] overflow-y-auto custom-scrollbar text-[11.5px] leading-5 text-neutral-500 dark:text-neutral-400 italic whitespace-pre-wrap break-words"
        >
          {reasoning}
        </div>
      )}
    </div>
  )
}

/**
 * Lens 模式：单 webview 三态机，统一 DOM。
 * - select：webview 全屏 + 灰幕 + hover 应用窗口高亮 + 区域 drag + 底部对话栏（纯文字直发）
 * - ready：截图后对话栏 CSS transition 飞到选区附近，加缩略图，输入聚焦
 * - answering：对话栏下方展开 answer 区（透明背景，对话栏不动）
 *
 * 关键：webview 始终全屏，整个过渡靠 CSS。后端 lens_resolve_anchor 仅算目标坐标，不缩窗口。
 */
export default function Lens() {
  const [stage, setStage] = useState<Stage>('select')
  const [windows, setWindows] = useState<LensWindowInfo[]>([])
  const [hovered, setHovered] = useState<LensWindowInfo | null>(null)
  const [winOrigin, setWinOrigin] = useState<{ x: number; y: number }>({ x: 0, y: 0 })
  const [dragStart, setDragStart] = useState<Point | null>(null)
  const [dragCurrent, setDragCurrent] = useState<Point | null>(null)
  const [dragging, setDragging] = useState(false)
  const [imagePreview, setImagePreview] = useState('')
  const [appLabel, setAppLabel] = useState('')
  const [input, setInput] = useState('')
  // Lens 启动前 Rust 端抓到的选中文本：作为本次会话的上下文前缀
  // 仅在首轮 chat 消息发送时拼接进 prompt；徽章静态显示行数；次轮不再注入。
  const [selectionText, setSelectionText] = useState('')
  const [messages, setMessages] = useState<ExplainMessage[]>([])
  const [streaming, setStreaming] = useState(false)
  const [copied, setCopied] = useState(false)
  const [lang, setLang] = useState<Lang>('zh')
  const [messageOrder, setMessageOrder] = useState<'asc' | 'desc'>('asc')
  const [keepFullscreen, setKeepFullscreen] = useState(true)
  const [floatingRebased, setFloatingRebased] = useState(false)
  const [mode, setMode] = useState<Mode>(() => readModeFromHash())
  // translate 模式专用：OCR 原文 + 翻译结果 + 计时
  const [translateOriginal, setTranslateOriginal] = useState('')
  const [translateText, setTranslateText] = useState('')
  const [translateError, setTranslateError] = useState('')
  const [translateDurationMs, setTranslateDurationMs] = useState<number | null>(null)
  const [translateNow, setTranslateNow] = useState(() => Date.now())
  const translateStartRef = useRef<number | null>(null)
  // viewport 大小：监听 resize（拔显示器/系统缩放变化都会触发），所有相对尺寸由此重算
  const [viewport, setViewport] = useState(() => ({
    w: typeof window !== 'undefined' ? window.innerWidth : 1280,
    h: typeof window !== 'undefined' ? window.innerHeight : 800,
  }))
  const metrics = useMemo(() => computeMetrics(viewport.w, viewport.h), [viewport])
  const [barRect, setBarRect] = useState<BarRect>(() => {
    const w = typeof window !== 'undefined' ? window.innerWidth : 1280
    const h = typeof window !== 'undefined' ? window.innerHeight : 800
    return computeSelectBar(w, h, computeMetrics(w, h))
  })
  // barIntro：select 态首次显示时给对话栏加一次 scale-up 进入动画；之后切换都靠 transition
  const [barIntro, setBarIntro] = useState(true)
  // barNoTransition：reset 时临时禁用 left/top/width transition，避免上次 ready/answering 位置
  // 残留的 380ms 动画在 window hide 时被暂停，下次 show 时从中间帧续播 → 视觉上 bar
  // 从老位置"滑"回 select 默认位置的闪烁。
  const [barNoTransition, setBarNoTransition] = useState(false)
  // capturedFrame：保留最后一次截图选区/窗口的高亮框，作为"已截图"视觉标记，ready/answering 态继续显示
  const [capturedFrame, setCapturedFrame] = useState<CapturedFrame | null>(null)
  // 箭头标注:仅 stage==='ready' 子模式
  // arrows / draftArrow 坐标系 = capturedFrame 逻辑像素 (左上角为原点)
  const [drawMode, setDrawMode] = useState(false)
  const [arrows, setArrows] = useState<Arrow[]>([])
  const [draftArrow, setDraftArrow] = useState<Arrow | null>(null)
  // 任何 stage 切换时强制清掉 draw 子模式 + 已落箭头
  useEffect(() => {
    if (stage !== 'ready') {
      setDrawMode(false)
      setArrows([])
      setDraftArrow(null)
    }
  }, [stage])
  // 内存历史：单次 app 生命周期保留，esc/hide 不清空
  const [history, setHistory] = useState<HistoryItem[]>(loadHistoryFromStorage)
  const [historyOpen, setHistoryOpen] = useState(false)

  const inputRef = useRef<HTMLInputElement>(null)
  const historyPanelRef = useRef<HTMLDivElement>(null)
  const stageRef = useRef<Stage>('select')
  const modeRef = useRef<Mode>(mode)
  const historyOpenRef = useRef(false)
  const imageIdRef = useRef('')
  const copyTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const floatingRebaseTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const focusReqIdRef = useRef(0)
  const prevStreamingRef = useRef(false)
  // Stream 真实结束（成功 / 错误 / 用户主动取消）后才置 true，
  // 让历史持久化 effect 只在这一次 rerun 触发 push；restoreHistory / enterSelect / resetBeforeHide 防御性清零，
  // 避免恢复历史时 setMessages 触发 effect 把恢复的对话又当新条目写一遍历史。
  const justFinishedStreamRef = useRef(false)
  // capture 期间 macOS screencapture 可能短暂让 lens webview 失焦 → 触发 blur 误关闭。
  // 这个 ref 标记"截图进行中"，blur handler 看到就跳过。
  const capturingRef = useRef(false)
  // selectionText 异步 take 的重入 token：每次 enterSelect / resetBeforeHide / restoreHistory 都 +1，
  // 老请求看到 myReq !== current 直接丢弃，避免 take 完成时已经进入新会话被错误注入。
  const selectionReqIdRef = useRef(0)
  // 答案区滚动容器，stream 时自动滚到底部
  const chatScrollRef = useRef<HTMLDivElement>(null)
  // 浮动模式下保存截图时的全屏 metrics，避免窗口缩小后 answerLayout 被压缩得太小
  const fullscreenMetricsRef = useRef<Metrics | null>(null)

  const t = i18n[lang]
  stageRef.current = stage
  modeRef.current = mode
  historyOpenRef.current = historyOpen

  // 选中文本行数：translate 模式不计；空 / 仅空白 → 0（驱动徽章是否显示）
  const selectionLineCount = useMemo(() => {
    if (mode !== 'chat') return 0
    if (!selectionText.trim()) return 0
    return selectionText.split(/\r?\n/).length
  }, [selectionText, mode])

  // 加载设置：语言 + 消息顺序。keepFullscreen 按当前 mode 读对应配置:
  // chat 模式读 settings.lens.keepFullscreenAfterCapture，translate 模式读 settings.screenshotTranslation.keepFullscreenAfterCapture。
  useEffect(() => {
    void (async () => {
      try {
        const settings = await api.getSettings()
        setLang((settings.settingsLanguage as Lang) || 'zh')
        setMessageOrder(settings.lens?.messageOrder === 'desc' ? 'desc' : 'asc')
        const curMode = readModeFromHash()
        const cfg = curMode === 'translate' ? settings.screenshotTranslation : settings.lens
        setKeepFullscreen(cfg?.keepFullscreenAfterCapture !== false)
      } catch (err) { console.error('Failed to load settings', err) }
    })()
  }, [])

  const focusLensInput = useCallback((delays: number[] = [0, 40, 120, 240, 420]) => {
    const requestId = ++focusReqIdRef.current
    const canFocus = () => (
      requestId === focusReqIdRef.current
      && modeRef.current === 'chat'
      && !historyOpenRef.current
      && !capturingRef.current
      && (stageRef.current === 'select' || stageRef.current === 'ready' || stageRef.current === 'answering')
    )

    const run = async () => {
      if (!canFocus()) return
      try {
        await getCurrentWindow().setFocus()
      } catch {
        // Native focus can fail briefly while the window is still becoming visible.
      }
      if (!canFocus()) return
      inputRef.current?.focus({ preventScroll: true })
      requestAnimationFrame(() => {
        if (canFocus()) inputRef.current?.focus({ preventScroll: true })
      })
    }

    delays.forEach(delay => window.setTimeout(() => { void run() }, delay))
  }, [])

  // select 态进入：刷新所有 state、重算对话栏位置、播放 intro 动画
  const enterSelect = useCallback(async () => {
    if (floatingRebaseTimerRef.current) {
      clearTimeout(floatingRebaseTimerRef.current)
      floatingRebaseTimerRef.current = null
    }
    fullscreenMetricsRef.current = null
    // 重新加载设置：用户在设置面板修改后关闭再打开 Lens，需要读到最新值。按当前 mode 选 lens / screenshotTranslation 配置。
    try {
      const settings = await api.getSettings()
      const curMode = readModeFromHash()
      const cfg = curMode === 'translate' ? settings.screenshotTranslation : settings.lens
      setKeepFullscreen(cfg?.keepFullscreenAfterCapture !== false)
    } catch (err) { console.error('Failed to reload settings', err) }
    // 防御：reset 流程会 setMessages([]) + setStreaming(false)，理论上 messages.length===0 effect 不会进
    // 持久化分支，但显式清零更稳
    justFinishedStreamRef.current = false
    // 用 flushSync 同步提交所有 reset 后的状态：webview show 之前 DOM 必须已经反映新位置，
    // 否则 Rust 的 show() 会先把旧 frame 露出来。
    // barNoTransition 同 frame 一起置 true → bar 从老坐标 snap 到 select 坐标，不动画。
    flushSync(() => {
      setBarNoTransition(true)
      setStage('select')
      setMode(readModeFromHash())
      setFloatingRebased(false)
      setHovered(null)
      setDragStart(null)
      setDragCurrent(null)
      setDragging(false)
      setImagePreview('')
      setAppLabel('')
      setInput('')
      setSelectionText('')
      setMessages([])
      setStreaming(false)
      setTranslateOriginal('')
      setTranslateText('')
      setTranslateError('')
      const w = window.innerWidth
      const h = window.innerHeight
      setViewport({ w, h })
      setBarRect(computeSelectBar(w, h, computeMetrics(w, h)))
      setCapturedFrame(null)
      // 重置 intro：先关再开，下一帧让 transition 从 scale-90 到 scale-100
      setBarIntro(false)
    })
    imageIdRef.current = ''
    // 异步 take 走 Rust 端在 lens_request_internal 中暂存的选中文本。
    // token 防御：take 期间用户再开一次 Lens / 关闭，老 promise 落地时 myReq 已过期，丢弃。
    // 仅 chat 模式注入；> 200KB 直接丢弃避免上下文爆炸；trim 后非空才 setSelectionText。
    const myReq = ++selectionReqIdRef.current
    if (readModeFromHash() === 'chat') {
      void (async () => {
        try {
          const text = await api.takeLensSelection()
          if (myReq !== selectionReqIdRef.current) return
          if (text.length > 200_000) return
          if (text.trim()) {
            setSelectionText(text)
            focusLensInput([0, 60, 180])
          }
        } catch (err) {
          console.warn('[lens] take selection failed:', err)
        }
      })()
    }
    requestAnimationFrame(() => {
      // 第二个 raf 同时恢复 transitions 并触发 intro：现在 bar 已经在 select 位置，
      // 只对 transform/opacity 做缩放进入动画，不会回放历史 left/top 过渡。
      requestAnimationFrame(() => {
        setBarIntro(true)
        setBarNoTransition(false)
      })
    })
    try {
      const win = getCurrentWindow()
      const [pos, scale] = await Promise.all([win.outerPosition(), win.scaleFactor()])
      const sf = scale || 1
      setWinOrigin({ x: pos.x / sf, y: pos.y / sf })
    } catch (err) { console.error('Failed to read window origin', err) }
    try {
      const list = await api.lensListWindows()
      setWindows(list)
    } catch (err) {
      console.error('Failed to list windows', err)
      setWindows([])
    }
    await api.showWindow()
    focusLensInput()
  }, [focusLensInput])

  useEffect(() => {
    void enterSelect()
    const handleReset = () => { void enterSelect() }
    window.addEventListener('lens:reset', handleReset)
    return () => window.removeEventListener('lens:reset', handleReset)
  }, [enterSelect])

  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined
    getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (focused) focusLensInput([0, 40, 120])
    }).then((dispose) => {
      if (cancelled) dispose()
      else unlisten = dispose
    }).catch(err => console.error('[lens-focus] listen failed:', err))
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [focusLensInput])

  // viewport resize（拔显示器 / 切分辨率 / DPI 变更，以及浮动模式下 raf 同步动画的逐帧缩放）
  // 都触发 'resize' 事件 → 更新 viewport state，让相对尺寸 metrics 重算。
  // 注意：浮动模式 rebase 已经在 flyBarToAnchor 里通过同步动画完成，不再在 resize handler 里抢占 barRect。
  useEffect(() => {
    const onResize = () => {
      setViewport({ w: window.innerWidth, h: window.innerHeight })
    }
    window.addEventListener('resize', onResize)
    return () => window.removeEventListener('resize', onResize)
  }, [])

  // viewport 或 metrics 变化时，select 态重算底部 bar 位置（ready/answering 态保持当前飞入位置不动，避免对话中闪跳）
  useEffect(() => {
    if (stageRef.current === 'select') {
      setBarRect(computeSelectBar(viewport.w, viewport.h, metrics))
    }
  }, [viewport, metrics])

  // 流式结束（streaming → false 且有任意 assistant 回答）时把当前会话推入历史。
  // 按 imageId 去重：同一张截图多轮对话作为单条历史持续更新到最前。
  // translate 模式不入对话历史（OCR+翻译是一次性任务，无对话语义）。
  // 缩略图压缩到 96x96 jpeg 再写历史，避免 localStorage 被几 MB 的 base64 撑爆。
  useEffect(() => {
    // 只在真实"流刚结束"路径触发：handleSend / handleStop 的 finally 会先置 ref 再 setStreaming(false)。
    // restoreHistory / enterSelect / resetBeforeHide 调用前会显式清零 ref，避免恢复历史时 effect 误触发。
    if (!justFinishedStreamRef.current) return
    if (mode !== 'chat') return
    if (streaming) return
    if (!imageIdRef.current || messages.length === 0) return
    const hasAssistant = messages.some(m => m.role === 'assistant' && m.content)
    if (!hasAssistant) return
    justFinishedStreamRef.current = false

    const id = imageIdRef.current
    let cancelled = false
    void (async () => {
      const thumb = await makeThumbnail(imagePreview, HISTORY_THUMB_SIZE)
      if (cancelled) return
      // 把活跃 image 拷贝到 lens-history 持久目录（lens_close 不会再删它，下次打开历史还能继续聊）
      api.lensCommitImageToHistory(id).catch(err => console.error('[lens-history] commit failed:', err))
      setHistory(prev => {
        const filtered = prev.filter(h => h.id !== id)
        const next: HistoryItem = {
          id,
          imagePreview: thumb,
          appLabel,
          messages,
          capturedFrame,
          timestamp: Date.now(),
        }
        return [next, ...filtered].slice(0, HISTORY_MAX)
      })
    })()
    return () => { cancelled = true }
  }, [mode, streaming, messages, imagePreview, appLabel, capturedFrame])

  // history 任意变化：1) 同步 localStorage  2) 检测淘汰并删除磁盘上对应的 PNG
  const prevHistoryIdsRef = useRef<Set<string>>(new Set(history.map(h => h.id)))
  useEffect(() => {
    saveHistoryToStorage(history)
    const curIds = new Set(history.map(h => h.id))
    prevHistoryIdsRef.current.forEach(id => {
      if (!curIds.has(id)) {
        api.lensDeleteHistoryImage(id).catch(err => console.error('[lens-history] delete failed:', err))
      }
    })
    prevHistoryIdsRef.current = curIds
  }, [history])

  // 监听 lens-stream 事件：把 reasoning_delta / delta 累积到最后一条 assistant 消息
  // StrictMode 双挂载下 listen 是 async：cleanup 时 unlisten 可能还没赋值，需要 cancelled 旗标
  // 让 promise resolve 时立即 dispose，否则会留下"幽灵 listener"导致每个事件触发 N 次（字符重复）
  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined
    api.onLensStream((payload: LensStreamPayload) => {
      if (payload.imageId !== imageIdRef.current) return
      if (payload.done) {
        setStreaming(false)
        return
      }
      if (payload.reasoningDelta) {
        setMessages(prev => {
          const last = prev[prev.length - 1]
          if (!last || last.role !== 'assistant') return prev
          return [...prev.slice(0, -1), { ...last, reasoning: (last.reasoning ?? '') + payload.reasoningDelta }]
        })
      }
      if (payload.delta) {
        setMessages(prev => {
          const last = prev[prev.length - 1]
          if (!last || last.role !== 'assistant') return prev
          return [...prev.slice(0, -1), { ...last, content: last.content + payload.delta }]
        })
      }
    }).then((dispose) => {
      if (cancelled) dispose()
      else unlisten = dispose
    }).catch(err => console.error(err))
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [])

  // messages 变化时自动滚动：正序滚到底（看新内容），倒序滚到顶（最新在顶）
  useEffect(() => {
    const el = chatScrollRef.current
    if (!el) return
    if (messageOrder === 'desc') el.scrollTop = 0
    else el.scrollTop = el.scrollHeight
  }, [messages, messageOrder])

  // Windows WebView2 在 input disabled/read-write 切换后容易丢 caret；回答结束后显式还焦点。
  useEffect(() => {
    const wasStreaming = prevStreamingRef.current
    prevStreamingRef.current = streaming
    if (!wasStreaming || streaming) return
    if (mode !== 'chat') return
    if (historyOpen) return
    if (stageRef.current !== 'answering' && stageRef.current !== 'ready') return

    const id = setTimeout(() => {
      focusLensInput([0, 60, 160])
    }, 30)
    return () => clearTimeout(id)
  }, [streaming, mode, historyOpen, focusLensInput])

  // 关闭前同步重置 state，让 webview surface 在 hide 之前已经是空 select 态。
  // 否则下次 show 时 macOS 会先显示上次的 ready 态 surface 一帧，再被 lens:reset 覆盖 → 闪一下上次内容。
  // barNoTransition：禁用 left/top/width transition，避免 380ms 动画被 hide 暂停后下次 show 续播。
  const resetBeforeHide = useCallback(() => {
    if (floatingRebaseTimerRef.current) {
      clearTimeout(floatingRebaseTimerRef.current)
      floatingRebaseTimerRef.current = null
    }
    fullscreenMetricsRef.current = null
    // 防御：和 enterSelect 同理 —— reset 路径不该走持久化
    justFinishedStreamRef.current = false
    flushSync(() => {
      setBarNoTransition(true)
      setStage('select')
      setFloatingRebased(false)
      setHovered(null)
      setDragStart(null)
      setDragCurrent(null)
      setDragging(false)
      setImagePreview('')
      setAppLabel('')
      setInput('')
      setSelectionText('')
      setMessages([])
      setStreaming(false)
      setBarRect(computeSelectBar(viewport.w, viewport.h, metrics))
      setCapturedFrame(null)
      setBarIntro(false)
    })
    imageIdRef.current = ''
    // 让任何还没落地的 takeLensSelection 老 promise 作废，避免关闭后 setSelectionText 拖回来
    selectionReqIdRef.current++
    focusReqIdRef.current++
  }, [viewport, metrics])

  // 全局 Esc：流式时取消流 / 否则关闭
  useEffect(() => {
    const handler = async (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      if (stageRef.current === 'answering' && streaming) {
        try { await api.lensCancelStream() } catch (err) { console.error(err) }
        setStreaming(false)
        return
      }
      resetBeforeHide()
      try { await api.lensClose() } catch (err) { console.error(err) }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [streaming, resetBeforeHide])

  // drawMode 键盘:Cmd+Z 撤销最后一支箭头,Esc 退出 drawMode(arrows 保留)
  useEffect(() => {
    if (!drawMode) return
    const onKey = (e: KeyboardEvent) => {
      // 输入框聚焦时不拦截,让用户继续打字
      const target = e.target as HTMLElement | null
      const isInput = target?.tagName === 'INPUT' || target?.tagName === 'TEXTAREA'

      if (e.key === 'Escape' && !isInput) {
        e.preventDefault()
        e.stopPropagation()
        setDrawMode(false)
        setDraftArrow(null)
        return
      }
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'z' && !e.shiftKey && !isInput) {
        e.preventDefault()
        e.stopPropagation()
        setArrows(prev => prev.slice(0, -1))
      }
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [drawMode])

  // select 态切到其他应用 → 自动收起灰幕。
  // 注意：截图过程中 screencapture 可能让 lens 短暂失焦，capturingRef 防止误关。
  useEffect(() => {
    const handleBlur = () => {
      if (capturingRef.current) return
      if (stageRef.current === 'select') {
        resetBeforeHide()
        void api.lensClose()
      }
    }
    window.addEventListener('blur', handleBlur)
    return () => window.removeEventListener('blur', handleBlur)
  }, [resetBeforeHide])

  /** webview client 坐标 → 全局逻辑坐标（与 CGWindow bounds 同坐标系） */
  const clientToGlobal = (p: Point): Point => ({
    x: winOrigin.x + p.x,
    y: winOrigin.y + p.y,
  })

  /** 命中检测：找第一个包含该全局坐标的应用窗口 */
  const hitTest = (gp: Point): LensWindowInfo | null => {
    for (const w of windows) {
      if (gp.x >= w.x && gp.x < w.x + w.width && gp.y >= w.y && gp.y < w.y + w.height) {
        return w
      }
    }
    return null
  }

  // 拖动选区矩形（webview 内坐标）
  const dragRect = useMemo(() => {
    if (!dragStart || !dragCurrent) return null
    const x = Math.min(dragStart.x, dragCurrent.x)
    const y = Math.min(dragStart.y, dragCurrent.y)
    const w = Math.abs(dragCurrent.x - dragStart.x)
    const h = Math.abs(dragCurrent.y - dragStart.y)
    return { x, y, width: w, height: h }
  }, [dragStart, dragCurrent])

  // hover 高亮区（webview 内坐标）
  const hoverRect = useMemo(() => {
    if (!hovered || dragging) return null
    return {
      x: hovered.x - winOrigin.x,
      y: hovered.y - winOrigin.y,
      width: hovered.width,
      height: hovered.height,
    }
  }, [hovered, dragging, winOrigin])

  const handleMouseDown = (e: React.MouseEvent) => {
    if (stage !== 'select') return
    const p: Point = { x: e.clientX, y: e.clientY }
    setDragStart(p)
    setDragCurrent(p)
    setDragging(false)
  }

  const handleMouseMove = (e: React.MouseEvent) => {
    if (stage !== 'select') return
    const p: Point = { x: e.clientX, y: e.clientY }
    if (dragStart) {
      setDragCurrent(p)
      const dx = Math.abs(p.x - dragStart.x)
      const dy = Math.abs(p.y - dragStart.y)
      if (!dragging && (dx > DRAG_THRESHOLD || dy > DRAG_THRESHOLD)) {
        setDragging(true)
        setHovered(null)
      }
      return
    }
    const gp = clientToGlobal(p)
    setHovered(hitTest(gp))
  }

  /** 截图后 lens 默认模式：在前端直接算 bar 位置，让对话栏飞到选区左/右侧（不再上下出现）。
   *  优先右侧，右侧空间不够再放左侧；都不够时贴大空间一侧。垂直与选区中心对齐并 clamp 在 viewport 内。 */
  const flyBarToAnchor = async (
    anchorAbsX: number,
    anchorAbsY: number,
    anchorW: number,
    anchorH: number,
    label: string,
  ) => {
    const ax = anchorAbsX - winOrigin.x
    const ay = anchorAbsY - winOrigin.y
    const vw = window.innerWidth
    const vh = window.innerHeight
    const READY_W = metrics.READY_W
    const ANSWER_H = metrics.ANSWER_H

    const rightStart = ax + anchorW + ANCHOR_GAP
    const spaceRight = vw - rightStart - 16
    const spaceLeft = ax - ANCHOR_GAP - 16

    let targetX: number
    if (spaceRight >= READY_W) {
      targetX = rightStart
    } else if (spaceLeft >= READY_W) {
      targetX = ax - READY_W - ANCHOR_GAP
    } else {
      // 左右都放不下完整 bar：贴空间更大的一侧屏幕边
      targetX = spaceRight >= spaceLeft ? vw - READY_W - 16 : 16
    }

    // 垂直：与选区中心对齐；总高度需容纳 bar + 8 + answer 区
    const totalH = READY_BAR_H + 8 + ANSWER_H
    let targetY = ay + anchorH / 2 - READY_BAR_H / 2
    if (targetY + totalH > vh - 16) targetY = vh - totalH - 16
    if (targetY < 16) targetY = 16

    if (targetX < 16) targetX = 16
    if (targetX + READY_W > vw - 16) targetX = vw - READY_W - 16

    // translate 模式截完直接进 translating；chat 模式进 ready 等用户提问
    const targetStage: Stage = mode === 'translate' ? 'translating' : 'ready'

    if (!keepFullscreen) {
      // 浮动模式：从 capture 瞬间起，"缩窗"和 chat 模式的"bar CSS 飞到 (0,0)"同步进行。
      // - chat 模式：bar 从 select 位置 cubic-bezier(0.22,1,0.36,1) lerp 到 (FLOATING_PADDING, FLOATING_PADDING)
      // - translate 模式：卡片本来就没有"起始位置"(选区阶段不渲染)，不要 fly，否则会随 window 漂移成"伪 fly"。
      //   所以 mount 时 barIntro=false（透明 + scale-0.92 隐身），window 动画结束后 setBarIntro(true) → CSS 缓动 fade-in。
      // 关键：raf 驱动 lensSetFloating 与 CSS transition 同 vsync，cubic-bezier 与全屏模式同曲线。
      fullscreenMetricsRef.current = metrics
      const floatX = winOrigin.x + targetX - FLOATING_PADDING
      const floatY = winOrigin.y + targetY - FLOATING_PADDING
      const floatW = READY_W + FLOATING_PADDING * 2
      const floatH = targetStage === 'ready'
        ? READY_BAR_H + FLOATING_PADDING * 2
        : READY_BAR_H + FLOATING_GAP + metrics.ANSWER_H + FLOATING_PADDING * 2
      const startX = winOrigin.x
      const startY = winOrigin.y
      const startW = window.innerWidth
      const startH = window.innerHeight
      const isTranslateMode = mode === 'translate'
      flushSync(() => {
        setAppLabel(label)
        setFloatingRebased(false)
        setBarRect({ x: FLOATING_PADDING, y: FLOATING_PADDING, width: READY_W })
        setStage(targetStage)
        if (isTranslateMode) {
          // 卡片 mount 在隐身状态 + 禁用 transition（避免 (selectX,selectY) → (0,0) 的瞬时 left/top 跳动触发动画）
          setBarIntro(false)
          setBarNoTransition(true)
        }
      })
      if (isTranslateMode) {
        // 隔两帧再恢复 transition，让 window 动画结束时 setBarIntro(true) 能走 CSS 缓动 fade-in
        requestAnimationFrame(() => {
          requestAnimationFrame(() => setBarNoTransition(false))
        })
      }
      if (floatingRebaseTimerRef.current) clearTimeout(floatingRebaseTimerRef.current)
      const animStart = performance.now()
      const animateFrame = () => {
        if (stageRef.current === 'select') return
        const elapsed = performance.now() - animStart
        const tLinear = Math.min(elapsed / TRANSITION_MS, 1)
        // 与全屏模式同款 cubic-bezier(0.22, 1, 0.36, 1) ease-out 缓动，
        // bar CSS transitionTimingFunction 也用同一条曲线，两者完全同步。
        const t = cubicBezier(tLinear, 0.22, 1, 0.36, 1)
        const curX = startX + (floatX - startX) * t
        const curY = startY + (floatY - startY) * t
        const curW = startW + (floatW - startW) * t
        const curH = startH + (floatH - startH) * t
        void api.lensSetFloating({ x: curX, y: curY, width: curW, height: curH })
          .catch((err: unknown) => console.error('[lens] lensSetFloating frame failed:', err))
        if (tLinear < 1) {
          requestAnimationFrame(animateFrame)
        } else {
          setFloatingRebased(true)
          if (isTranslateMode) setBarIntro(true)
        }
      }
      requestAnimationFrame(animateFrame)
    } else {
      // 全屏模式：保持现有逻辑
      flushSync(() => {
        setAppLabel(label)
        setBarRect({ x: Math.round(targetX), y: Math.round(targetY), width: READY_W })
        setStage(targetStage)
      })
    }
    if (mode === 'chat') {
      focusLensInput([TRANSITION_MS + 20, TRANSITION_MS + 120, TRANSITION_MS + 260])
    }
  }

  /** translate 模式：截完立即调 OCR + 翻译。
   *  流式：lens-translate-stream 事件累积 original/translated；done 事件结束并锁定耗时
   *  非流式：API 返回完整结果一次性灌入（也通过事件，后端在两步完成后 emit 一次完整 delta） */
  const runTranslate = useCallback(async (id: string) => {
    setTranslateOriginal('')
    setTranslateText('')
    setTranslateError('')
    setTranslateDurationMs(null)
    translateStartRef.current = Date.now()
    setTranslateNow(Date.now())
    try {
      const r = await api.lensTranslate(id)
      if (!r.success) {
        // 失败兜底：done 事件应该已经带 error 了，但补一刀防止前端漏 done
        setTranslateError(r.error || 'Failed')
        if (translateStartRef.current !== null) {
          setTranslateDurationMs(Date.now() - translateStartRef.current)
          translateStartRef.current = null
        }
        setStage('translated')
      }
      // 成功路径：等 lens-translate-stream 的 done 事件触发 stage / 计时（避免事件还没到 stage 就跳，或反之文字还没到完成态）
    } catch (err) {
      setTranslateError(err instanceof Error ? err.message : String(err))
      if (translateStartRef.current !== null) {
        setTranslateDurationMs(Date.now() - translateStartRef.current)
        translateStartRef.current = null
      }
      setStage('translated')
    }
  }, [])

  // lens-translate-stream 事件监听（与 lens-stream 同款 cancelled 旗标处理 StrictMode 双挂）
  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined
    api.onLensTranslateStream((payload: LensTranslateStreamPayload) => {
      if (payload.imageId !== imageIdRef.current) return
      if (payload.done) {
        if (payload.error) setTranslateError(payload.error)
        if (translateStartRef.current !== null) {
          setTranslateDurationMs(Date.now() - translateStartRef.current)
          translateStartRef.current = null
        }
        setStage('translated')
        return
      }
      if (!payload.delta) return
      if (payload.kind === 'original') {
        setTranslateOriginal(prev => prev + payload.delta)
      } else if (payload.kind === 'translated') {
        setTranslateText(prev => prev + payload.delta)
      }
    }).then((dispose) => {
      if (cancelled) dispose()
      else unlisten = dispose
    }).catch(err => console.error(err))
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [])

  // translating 期间每秒刷一次，header 走秒
  useEffect(() => {
    if (stage !== 'translating') return
    const id = setInterval(() => setTranslateNow(Date.now()), 1000)
    return () => clearInterval(id)
  }, [stage])

  const handleCaptureWindow = async (info: LensWindowInfo) => {
    // capturingRef 全程 true，避免 macOS screencapture 短暂让 lens webview 失焦时触发 blur handler 误关
    capturingRef.current = true
    try {
      const result = await api.lensCaptureWindow(info.id)
      if (!result.success || !result.imageId) {
        console.error('lensCaptureWindow failed:', result.error)
        void enterSelect()
        return
      }
      const newId = result.imageId
      imageIdRef.current = newId

      // 记录截图框（webview 内坐标）作为已截视觉标记，截完保留显示
      setCapturedFrame({
        x: info.x - winOrigin.x,
        y: info.y - winOrigin.y,
        width: info.width,
        height: info.height,
        label: info.owner,
      })
      void (async () => {
        try {
          const img = await api.explainReadImage(newId)
          if (img.success) setImagePreview(img.data ?? '')
        } catch (err) { console.error(err) }
      })()
      await flyBarToAnchor(
        Math.round(info.x), Math.round(info.y), Math.round(info.width), Math.round(info.height),
        info.owner,
      )
      if (mode === 'translate') void runTranslate(newId)
    } finally {
      capturingRef.current = false
    }
  }

  const handleCaptureRegion = async (rect: { x: number; y: number; width: number; height: number }) => {
    const gp = clientToGlobal({ x: rect.x, y: rect.y })
    const params = {
      absoluteX: Math.round(gp.x),
      absoluteY: Math.round(gp.y),
      x: Math.round(rect.x),
      y: Math.round(rect.y),
      width: Math.round(rect.width),
      height: Math.round(rect.height),
      scaleFactor: window.devicePixelRatio || 1,
    }
    // capturingRef 全程 true 直到 flyBarToAnchor 完成（同 handleCaptureWindow 注释）
    capturingRef.current = true
    try {
      const result = await api.lensCaptureRegion(params)
      if (!result.success || !result.imageId) {
        console.error('lensCaptureRegion failed:', result.error)
        void enterSelect()
        return
      }
      const newId = result.imageId
      imageIdRef.current = newId

      setCapturedFrame({
        x: params.x,
        y: params.y,
        width: params.width,
        height: params.height,
        label: '',
      })
      void (async () => {
        try {
          const img = await api.explainReadImage(newId)
          if (img.success) setImagePreview(img.data ?? '')
        } catch (err) { console.error(err) }
      })()
      await flyBarToAnchor(params.absoluteX, params.absoluteY, params.width, params.height, '')
      if (mode === 'translate') void runTranslate(newId)
    } finally {
      capturingRef.current = false
    }
  }

  const handleMouseUp = async (e: React.MouseEvent) => {
    if (stage !== 'select') return
    const releasedAt: Point = { x: e.clientX, y: e.clientY }

    if (dragging && dragStart) {
      const x = Math.min(dragStart.x, releasedAt.x)
      const y = Math.min(dragStart.y, releasedAt.y)
      const w = Math.abs(releasedAt.x - dragStart.x)
      const h = Math.abs(releasedAt.y - dragStart.y)
      setDragStart(null)
      setDragCurrent(null)
      setDragging(false)
      if (w < 10 || h < 10) return
      await handleCaptureRegion({ x, y, width: w, height: h })
      return
    }

    setDragStart(null)
    setDragCurrent(null)
    setDragging(false)
    if (hovered) {
      await handleCaptureWindow(hovered)
    }
  }

  const handleSend = async () => {
    if (!input.trim() || streaming) return
    const question = input.trim()
    setHistoryOpen(false)
    setInput('')

    // 默认沿用当前 image_id;若有箭头则先合成 + 注册新图,把后续 ask 切到合成版
    let effectiveImageId = imageIdRef.current
    if (arrows.length > 0 && imagePreview && capturedFrame) {
      try {
        const base64 = await composeAnnotatedImage(
          imagePreview,
          arrows,
          capturedFrame.width,
          capturedFrame.height,
        )
        const result = await api.lensRegisterAnnotatedImage(base64)
        if (result.success && result.imageId) {
          effectiveImageId = result.imageId
          imageIdRef.current = result.imageId
          setImagePreview(`data:image/png;base64,${base64}`)
          setArrows([])
          setDraftArrow(null)
          setDrawMode(false)
        } else {
          console.warn('[lens-arrow] register annotated image failed:', result.error)
        }
      } catch (err) {
        console.warn('[lens-arrow] compose failed, fallback to original:', err)
      }
    }

    // 首轮 chat 注入:把启动时抓到的选中文本作为 [已选文本] 段前置到 user prompt;
    // 后续轮次(messages.length>0)严格不重复注入.translate 模式不到这里.
    const isFirstTurn = messages.length === 0
    const ctx = (isFirstTurn && mode === 'chat') ? selectionText.trim() : ''
    const userContent = ctx
      ? (lang === 'zh'
          ? `[已选文本]\n${ctx}\n\n[用户问题]\n${question}`
          : `[Selected Text]\n${ctx}\n\n[Question]\n${question}`)
      : question
    const userMsg: ExplainMessage = { role: 'user', content: userContent }
    const placeholder: ExplainMessage = { role: 'assistant', content: '' }
    const sendMessages: ExplainMessage[] = [...messages, userMsg]
    flushSync(() => {
      setMessages([...sendMessages, placeholder])
      setStage('answering')
      setStreaming(true)
    })
    try {
      const result = await api.lensAsk(effectiveImageId || '', sendMessages)
      if (!result.success) {
        const errText = `${t.lensError}: ${result.error}`
        setMessages(prev => {
          const last = prev[prev.length - 1]
          if (!last || last.role !== 'assistant') return prev
          return [...prev.slice(0, -1), { role: 'assistant', content: errText }]
        })
      } else if (result.response) {
        // 非流式:把完整答案塞进占位 assistant;流式情况已在 onLensStream 累积,避免覆盖
        setMessages(prev => {
          const last = prev[prev.length - 1]
          if (!last || last.role !== 'assistant') return prev
          if (last.content.length > 0) return prev
          return [...prev.slice(0, -1), { role: 'assistant', content: result.response! }]
        })
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err)
      setMessages(prev => {
        const last = prev[prev.length - 1]
        if (!last || last.role !== 'assistant') return prev
        return [...prev.slice(0, -1), { role: 'assistant', content: `${t.lensError}: ${msg}` }]
      })
    } finally {
      // ref 在 setStreaming(false) 之前置 true,让持久化 effect 在本次 rerun 中识别这是"流刚结束"路径
      justFinishedStreamRef.current = true
      setStreaming(false)
    }
  }

  const handleStop = async () => {
    try { await api.lensCancelStream() } catch (err) { console.error(err) }
    // 用户主动取消但已经流出部分内容，也持久化 —— 关掉再开历史能接着问
    justFinishedStreamRef.current = true
    setStreaming(false)
  }

  const handleCopy = async () => {
    // 复制最后一条 assistant 消息
    const lastAssistant = [...messages].reverse().find(m => m.role === 'assistant' && m.content)
    if (!lastAssistant) return
    const ok = await copyToClipboard(lastAssistant.content)
    if (!ok) return
    setCopied(true)
    if (copyTimeoutRef.current) clearTimeout(copyTimeoutRef.current)
    copyTimeoutRef.current = setTimeout(() => setCopied(false), 2000)
  }

  // 点击历史项：把当前会话恢复到该 item（image / appLabel / messages / capturedFrame）
  // 取消任何正在跑的流，避免后端继续 emit delta 灌入新恢复的 messages（如果新旧 imageId 巧合相同会污染）
  const restoreHistory = (item: HistoryItem) => {
    setHistoryOpen(false)
    if (streaming) {
      void api.lensCancelStream().catch(err => console.error(err))
    }
    imageIdRef.current = item.id
    // 防御：恢复历史 setMessages 会触发持久化 effect，但本路径不是"流刚结束"，不该 push 重复条目
    justFinishedStreamRef.current = false
    flushSync(() => {
      setImagePreview(item.imagePreview)
      setAppLabel(item.appLabel)
      setInput('')
      setSelectionText('')
      setMessages(item.messages)
      setCapturedFrame(item.capturedFrame)
      setStreaming(false)
      setStage('answering')
    })
    // 老 takeLensSelection promise 失效，避免恢复历史后被新 take 文本污染
    selectionReqIdRef.current++
    focusLensInput([50, 140, 260])
  }

  // 相对时间字符串（"刚刚" / "3 分钟前"）
  const relTime = (ts: number): string => {
    const diff = Date.now() - ts
    const m = Math.floor(diff / 60000)
    if (m < 1) return lang === 'zh' ? '刚刚' : 'just now'
    if (m < 60) return lang === 'zh' ? `${m} 分钟前` : `${m}m ago`
    const h = Math.floor(m / 60)
    if (h < 24) return lang === 'zh' ? `${h} 小时前` : `${h}h ago`
    return lang === 'zh' ? `${Math.floor(h / 24)} 天前` : `${Math.floor(h / 24)}d ago`
  }

  useEffect(() => () => {
    if (copyTimeoutRef.current) clearTimeout(copyTimeoutRef.current)
    if (floatingRebaseTimerRef.current) clearTimeout(floatingRebaseTimerRef.current)
    focusReqIdRef.current++
  }, [])

  // 点击 history 面板外部 → 关闭
  useEffect(() => {
    if (!historyOpen) return
    const onDown = (e: MouseEvent) => {
      if (!historyPanelRef.current?.contains(e.target as Node)) {
        setHistoryOpen(false)
      }
    }
    document.addEventListener('mousedown', onDown, true)
    return () => document.removeEventListener('mousedown', onDown, true)
  }, [historyOpen])

  // ====== 单一渲染 ======
  const showThumb = stage !== 'select' && (imagePreview || appLabel)
  // 流式期间禁止发送/输入，答完之后可对同一张截图继续问新问题（每次仍为独立 Q&A，自动入历史）
  const sendDisabled = !input.trim() || streaming
  // 对话栏（输入框）只在 chat 模式显示；translate 模式只渲染浮动结果卡片
  const showBar = mode === 'chat'
  // translate 浮动卡片：截图后在选区旁出现，加载/完成两态
  const showTranslateCard = mode === 'translate' && (stage === 'translating' || stage === 'translated')
  // 浮动布局生效条件：用户关了"保持全屏覆盖" 且 已经真的截过图（窗口被缩成浮动模式）。
  // 没截图就直接提问的场景下，window 还是全屏 overlay、bar 还在底部居中，此时仍按全屏布局走。
  const isFloatingLayout = !keepFullscreen && capturedFrame !== null && stage !== 'select'
  const stableAnswerHeight = isFloatingLayout
    ? fullscreenMetricsRef.current?.ANSWER_H || metrics.ANSWER_H
    : metrics.ANSWER_H

  // 答案区展开方向 + 高度自适应：
  // 1) 下方空间够 ANSWER_H → 向下，目标高
  // 2) 上方空间够 → 向上，目标高
  // 3) 都不够 → 选大的那侧，高度收缩为该侧可用空间（最少 180，避免太矮）
  const answerLayout = useMemo(() => {
    if (isFloatingLayout) {
      return { placeAbove: false, height: stableAnswerHeight }
    }
    const target = stableAnswerHeight
    const spaceBelow = viewport.h - (barRect.y + READY_BAR_H + 8) - 16
    const spaceAbove = barRect.y - 8 - 16
    if (spaceBelow >= target) return { placeAbove: false, height: target }
    if (spaceAbove >= target) return { placeAbove: true, height: target }
    if (spaceAbove > spaceBelow) {
      return { placeAbove: true, height: Math.max(180, spaceAbove) }
    }
    return { placeAbove: false, height: Math.max(180, spaceBelow) }
  }, [barRect, isFloatingLayout, stableAnswerHeight, viewport.h])

  // 浮动模式下：stage / 布局变化时动态调整窗口尺寸
  useEffect(() => {
    if (keepFullscreen) return
    if (stage === 'select') return
    if (!floatingRebased) return

    const w = barRect.width + FLOATING_PADDING * 2
    let h = READY_BAR_H + FLOATING_PADDING * 2

    if (stage === 'answering') {
      h += FLOATING_GAP + answerLayout.height
    }

    // translate 卡片预留空间
    if ((stage === 'translating' || stage === 'translated') && mode === 'translate') {
      h = Math.max(h, READY_BAR_H + FLOATING_GAP + stableAnswerHeight + FLOATING_PADDING * 2)
    }

    // 只更新尺寸，位置不变（不传 x/y）
    api.lensSetFloating({ width: w, height: h }).catch(err => console.error('[lens-floating] resize failed:', err))
  }, [stage, answerLayout, barRect, floatingRebased, keepFullscreen, mode, stableAnswerHeight])

  return (
    <div
      className="fixed inset-0 select-none"
      onMouseDown={handleMouseDown}
      onMouseMove={handleMouseMove}
      onMouseUp={handleMouseUp}
      data-tauri-drag-region="false"
    >
      {/* select 态全屏覆盖层：完全透明，仅用于捕获鼠标事件，不再加黑色蒙层 */}
      <div
        className="absolute inset-0 transition-opacity ease-out pointer-events-none"
        style={{
          backgroundColor: 'transparent',
          transitionDuration: `${TRANSITION_MS}ms`,
          opacity: stage === 'select' && !hoverRect && !dragRect ? 1 : 0,
        }}
      />

      {/* 已截图框：截完保留显示作为视觉标记（橙色边框 + 浅外发光，无挖洞遮罩） */}
      {/* 浮动模式下不显示高亮框 */}
      {capturedFrame && stage !== 'select' && keepFullscreen && (
        <>
          <div
            className="absolute border-[2px] border-[#D97757] rounded-md pointer-events-none"
            style={{
              left: capturedFrame.x,
              top: capturedFrame.y,
              width: capturedFrame.width,
              height: capturedFrame.height,
              boxShadow: '0 0 16px 2px rgba(217,119,87,0.45)',
            }}
          />
          {capturedFrame.label && (
            <div
              className="absolute -translate-y-full pl-2 pr-2.5 py-1 rounded-md bg-neutral-900/95 text-white text-[12px] font-medium whitespace-nowrap shadow-lg pointer-events-none border-l-2 border-[#D97757]"
              style={{
                left: Math.max(8, capturedFrame.x),
                top: Math.max(28, capturedFrame.y - 8),
              }}
            >
              {t.lensScreenshotOf} {capturedFrame.label}
            </div>
          )}
        </>
      )}

      {/* drawMode:在 capturedFrame 矩形内画箭头.frozen background = imagePreview;
          用 SVG 叠加 + 自带 mousedown/move/up,pointer-events 仅在 drawMode 启用 */}
      {capturedFrame && stage === 'ready' && keepFullscreen && drawMode && (
        <>
          {/* 截图框外加深(dim),让用户聚焦在截图区) */}
          <div
            className="absolute inset-0 pointer-events-none bg-black/40"
            style={{ zIndex: 10 }}
          />
          {/* capturedFrame 内:背景填充冻结 PNG + SVG 收事件 */}
          <div
            className="absolute"
            style={{
              left: capturedFrame.x,
              top: capturedFrame.y,
              width: capturedFrame.width,
              height: capturedFrame.height,
              backgroundImage: imagePreview ? `url("${imagePreview}")` : undefined,
              backgroundSize: '100% 100%',
              backgroundRepeat: 'no-repeat',
              cursor: 'crosshair',
              zIndex: 11,
              touchAction: 'none',
            }}
            onPointerDown={(e) => {
              e.stopPropagation()
              ;(e.currentTarget as HTMLDivElement).setPointerCapture(e.pointerId)
              const rect = e.currentTarget.getBoundingClientRect()
              const x = e.clientX - rect.left
              const y = e.clientY - rect.top
              setDraftArrow({ x1: x, y1: y, x2: x, y2: y })
            }}
            onPointerMove={(e) => {
              if (!draftArrow) return
              e.stopPropagation()
              const rect = e.currentTarget.getBoundingClientRect()
              const x = Math.max(0, Math.min(rect.width, e.clientX - rect.left))
              const y = Math.max(0, Math.min(rect.height, e.clientY - rect.top))
              setDraftArrow(d => (d ? { ...d, x2: x, y2: y } : d))
            }}
            onPointerUp={(e) => {
              e.stopPropagation()
              if (!draftArrow) return
              const dx = draftArrow.x2 - draftArrow.x1
              const dy = draftArrow.y2 - draftArrow.y1
              if (Math.hypot(dx, dy) >= ARROW_MIN_DRAG_PX) {
                setArrows(prev => [...prev, draftArrow])
              }
              setDraftArrow(null)
              ;(e.currentTarget as HTMLDivElement).releasePointerCapture(e.pointerId)
            }}
            onPointerCancel={(e) => {
              // 浏览器主动释放捕获(例如系统对话框打断),清掉 draft
              e.stopPropagation()
              setDraftArrow(null)
              try { (e.currentTarget as HTMLDivElement).releasePointerCapture(e.pointerId) } catch { /* 已被释放,忽略 */ }
            }}
          >
            <svg
              width={capturedFrame.width}
              height={capturedFrame.height}
              className="absolute inset-0 pointer-events-none"
              style={{ overflow: 'visible' }}
            >
              {arrows.map((a, i) => (
                <ArrowSvg key={i} arrow={a} />
              ))}
              {draftArrow && <ArrowSvg arrow={draftArrow} />}
            </svg>
          </div>
        </>
      )}

      {/* select-only：hover 高亮 / drag 选区 / 顶部 hint */}
      {stage === 'select' && (
        <>
          {hoverRect && (
            <>
              <div
                className="absolute border-[2px] border-[#D97757] rounded-md pointer-events-none"
                style={{
                  left: hoverRect.x,
                  top: hoverRect.y,
                  width: hoverRect.width,
                  height: hoverRect.height,
                  boxShadow: '0 0 16px 2px rgba(217,119,87,0.45)',
                }}
              />
              {hovered && (
                <div
                  className="absolute -translate-y-full pl-2 pr-2.5 py-1 rounded-md bg-neutral-900/95 text-white text-[12px] font-medium whitespace-nowrap shadow-lg pointer-events-none border-l-2 border-[#D97757]"
                  style={{
                    left: Math.max(8, hoverRect.x),
                    top: Math.max(28, hoverRect.y - 8),
                  }}
                >
                  {t.lensScreenshotOf} {hovered.owner}
                </div>
              )}
            </>
          )}
          {dragRect && dragging && (
            <div
              className="absolute border-[2px] border-[#D97757] rounded-sm pointer-events-none"
              style={{
                left: dragRect.x,
                top: dragRect.y,
                width: dragRect.width,
                height: dragRect.height,
                boxShadow: '0 0 16px 2px rgba(217,119,87,0.45)',
              }}
            />
          )}
        </>
      )}

      {/* 对话栏 + 答案区：始终渲染，CSS transition 处理位置 / 大小变化。
          - select：底部居中 680，缩略图槽位用 sparkle 占位
          - ready：飞到选区附近 600，左侧切换为缩略图 + 应用名
          - answering：在对话栏下方 absolute 展开 answer 区（固定 360 高） */}
      {showBar && (
        <div
          className="absolute ease-out"
          onMouseDown={(e) => e.stopPropagation()}
          onMouseMove={(e) => e.stopPropagation()}
          onMouseUp={(e) => e.stopPropagation()}
          onClick={(e) => e.stopPropagation()}
          style={{
            left: barRect.x,
            top: barRect.y,
            width: barRect.width,
            transitionProperty: barNoTransition ? 'none' : 'left, top, width, transform, opacity',
            transitionDuration: barNoTransition ? '0ms' : `${TRANSITION_MS}ms`,
            transitionTimingFunction: 'cubic-bezier(0.22, 1, 0.36, 1)',
            transform: barIntro ? 'scale(1)' : 'scale(0.92)',
            opacity: barIntro ? 1 : 0,
          }}
        >
          {/* 输入栏卡片 */}
          <div
            className="flex items-center gap-3 pl-4 pr-2 py-2 rounded-[18px] bg-white dark:bg-neutral-900 shadow-[0_10px_28px_-20px_rgba(0,0,0,0.28)] ring-1 ring-black/[0.04] dark:ring-white/[0.06] cursor-default"
            data-tauri-drag-region="false"
          >
            <div className="shrink-0 flex items-center gap-2">
              {showThumb ? (
                <div className="flex items-center gap-2.5">
                  <div className="w-10 h-10 rounded-xl overflow-hidden ring-1 ring-black/[0.06] dark:ring-white/[0.06] bg-neutral-100 dark:bg-neutral-800 flex items-center justify-center shadow-sm">
                    {imagePreview ? (
                      <img src={imagePreview} alt="snap" className="w-full h-full object-cover" />
                    ) : (
                      <ImageIcon size={14} className="text-neutral-400" />
                    )}
                  </div>
                  {appLabel && (
                    <span className="text-[13px] font-medium text-neutral-800 dark:text-neutral-200 max-w-[100px] truncate">{appLabel}</span>
                  )}
                </div>
              ) : (
                <img
                  src="/logo-mark.png"
                  alt=""
                  className="w-7 h-7 object-contain dark:invert"
                  draggable={false}
                />
              )}
              {selectionLineCount > 0 && (
                <span
                  title={lang === 'zh' ? `已选中 ${selectionLineCount} 行` : `${selectionLineCount} lines selected`}
                  className="select-none px-1.5 py-0.5 rounded-md bg-neutral-100 dark:bg-neutral-800 text-[11px] font-medium tabular-nums text-neutral-600 dark:text-neutral-400 ring-1 ring-black/[0.04] dark:ring-white/[0.06]"
                >
                  {selectionLineCount}
                </span>
              )}
              {stage === 'ready' && (
                <button
                  type="button"
                  onClick={() => setDrawMode(m => !m)}
                  disabled={!imagePreview}
                  title={imagePreview
                    ? (drawMode ? t.lensArrowToggleOff : t.lensArrowToggle)
                    : t.lensArrowDisabledHint}
                  className={`shrink-0 w-8 h-8 rounded-lg flex items-center justify-center transition-colors ${
                    drawMode
                      ? 'bg-blue-500 text-white hover:bg-blue-600'
                      : 'text-neutral-600 dark:text-neutral-300 hover:bg-black/[0.05] dark:hover:bg-white/[0.06]'
                  } ${!imagePreview ? 'opacity-40 cursor-not-allowed' : ''}`}
                >
                  <MousePointer2 size={15} strokeWidth={1.75} />
                </button>
              )}
            </div>
            <input
              ref={inputRef}
              autoFocus
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key !== 'Enter' || e.shiftKey) return
                // IME 合成中（中/日/韩选词按回车）跳过 — isComposing 官方信号 + keyCode 229 兜底
                if (e.nativeEvent.isComposing || e.keyCode === 229) return
                e.preventDefault()
                void handleSend()
              }}
              readOnly={streaming}
              aria-disabled={streaming}
              placeholder={t.lensAskPlaceholder}
              className={`flex-1 bg-transparent text-[16px] text-neutral-900 dark:text-white placeholder-neutral-500 dark:placeholder-neutral-400 focus:outline-none ${streaming ? 'opacity-60' : ''}`}
            />
            {/* History dropdown：按钮 + 弹出面板（容器作为 ref，点击外部关闭） */}
            <div ref={historyPanelRef} className="relative shrink-0">
              <button
                type="button"
                onClick={() => setHistoryOpen(o => !o)}
                className="flex items-center gap-1 h-9 px-2.5 rounded-lg text-neutral-600 dark:text-neutral-300 hover:bg-black/[0.05] dark:hover:bg-white/[0.06] transition-colors"
                title={t.lensHistory}
              >
                <HistoryIcon size={15} strokeWidth={1.75} />
                {history.length > 0 && (
                  <span className="text-[11px] font-medium tabular-nums text-neutral-500 dark:text-neutral-400">{history.length}</span>
                )}
                <ChevronDown size={13} strokeWidth={2} className={`transition-transform ${historyOpen ? 'rotate-180' : ''}`} />
              </button>
              {historyOpen && (
                <div
                  className="absolute right-0 bottom-full mb-2 w-[240px] rounded-xl bg-white dark:bg-neutral-900 shadow-[0_18px_44px_-12px_rgba(0,0,0,0.4)] ring-1 ring-black/[0.06] dark:ring-white/[0.08] overflow-hidden z-50"
                >
                  <div className="max-h-[200px] overflow-y-auto custom-scrollbar py-1">
                    {history.length === 0 ? (
                      <div className="px-2.5 py-1.5 text-[11px] text-neutral-400 dark:text-neutral-500">
                        {t.lensNoHistory}
                      </div>
                    ) : (
                      history.map(item => {
                        // 首条 user 消息可能含 [已选文本]\n...\n\n[用户问题]\n... 的拼接形式（chat 启动注入），
                        // 历史预览只显示问题原文，剥掉 marker 段
                        const firstUserRaw = item.messages.find(m => m.role === 'user')?.content ?? ''
                        const zhMarker = '[用户问题]\n'
                        const enMarker = '[Question]\n'
                        const zhIdx = firstUserRaw.indexOf(zhMarker)
                        const enIdx = firstUserRaw.indexOf(enMarker)
                        const firstUserQ = zhIdx >= 0
                          ? firstUserRaw.slice(zhIdx + zhMarker.length)
                          : enIdx >= 0
                            ? firstUserRaw.slice(enIdx + enMarker.length)
                            : firstUserRaw
                        const turns = item.messages.filter(m => m.role === 'user').length
                        return (
                          <button
                            key={`${item.id}-${item.timestamp}`}
                            type="button"
                            onClick={() => restoreHistory(item)}
                            className="w-full flex items-center gap-2 px-2.5 py-1.5 text-left hover:bg-black/[0.04] dark:hover:bg-white/[0.06] transition-colors"
                          >
                            <div className="shrink-0 w-6 h-6 rounded overflow-hidden bg-neutral-100 dark:bg-neutral-800 ring-1 ring-black/[0.05] dark:ring-white/[0.06] flex items-center justify-center">
                              {item.imagePreview ? (
                                <img src={item.imagePreview} alt="" className="w-full h-full object-cover" />
                              ) : (
                                <ImageIcon size={10} className="text-neutral-400" />
                              )}
                            </div>
                            <div className="min-w-0 flex-1">
                              <div className="text-[11.5px] text-neutral-800 dark:text-neutral-200 truncate leading-tight">
                                {firstUserQ}
                              </div>
                              <div className="text-[9.5px] text-neutral-400 dark:text-neutral-500 mt-0.5 truncate leading-tight">
                                {item.appLabel ? `${item.appLabel} · ` : ''}{turns > 1 ? `${turns} 轮 · ` : ''}{relTime(item.timestamp)}
                              </div>
                            </div>
                          </button>
                        )
                      })
                    )}
                  </div>
                </div>
              )}
            </div>
            <button
              type="button"
              onClick={() => void handleSend()}
              disabled={sendDisabled}
              className={`shrink-0 w-10 h-10 rounded-xl flex items-center justify-center transition-all duration-150 active:scale-95 ${
                !sendDisabled
                  ? 'bg-[#D97757] hover:bg-[#C56646] hover:scale-105'
                  : 'bg-neutral-200 dark:bg-neutral-700 cursor-not-allowed'
              }`}
            >
              <ArrowUp
                size={18}
                strokeWidth={2.25}
                className={!sendDisabled ? 'text-white' : 'text-neutral-400 dark:text-neutral-500'}
              />
            </button>
          </div>

          {/* select 态键盘提示（在对话栏卡片下方） */}
          {stage === 'select' && (
            <div className="mt-2 flex justify-center gap-3 text-[11px] text-white/70 pointer-events-none">
              <span>↵ {t.lensHintSend}</span>
              <span>·</span>
              <span>esc {t.lensHintEsc}</span>
            </div>
          )}

          {/* answer 区：absolute 展开在对话栏上方或下方（自适应空间），渲染整个 chat list（多轮对话） */}
          <div
            className="absolute left-0 right-0 rounded-2xl overflow-hidden window-frosted transition-all ease-out select-text"
            style={{
              top: answerLayout.placeAbove ? undefined : 'calc(100% + 8px)',
              bottom: answerLayout.placeAbove ? 'calc(100% + 8px)' : undefined,
              height: stage === 'answering' ? answerLayout.height : 0,
              opacity: stage === 'answering' ? 1 : 0,
              transitionDuration: `${TRANSITION_MS}ms`,
              pointerEvents: stage === 'answering' ? 'auto' : 'none',
            }}
          >
            {stage === 'answering' && (() => {
              // 显示顺序：desc 反转数组（新在顶）；isLast 始终基于原数组末尾索引（最新的）
              const ordered = messageOrder === 'desc' ? messages.slice().reverse() : messages
              const lastChronoIdx = messages.length - 1
              const lastMsg = messages[lastChronoIdx]
              const showActions = lastMsg && lastMsg.role === 'assistant' && !!lastMsg.content
              const Actions = (
                <div className="flex items-center gap-1">
                  <button
                    onClick={() => void handleCopy()}
                    className="flex items-center gap-1 px-2 py-0.5 text-[10px] text-neutral-500 hover:text-neutral-800 dark:text-neutral-400 dark:hover:text-neutral-100 rounded hover:bg-black/5 dark:hover:bg-white/10 transition-colors"
                  >
                    {copied ? <Check size={11} /> : <Copy size={11} />}
                    <span>{copied ? t.lensCopied : t.lensCopy}</span>
                  </button>
                  {streaming && (
                    <button
                      onClick={() => void handleStop()}
                      className="flex items-center gap-1 px-2 py-0.5 text-[10px] text-neutral-500 hover:text-red-500 dark:text-neutral-400 rounded hover:bg-black/5 dark:hover:bg-white/10 transition-colors"
                    >
                      <Square size={10} strokeWidth={2.5} fill="currentColor" />
                      <span>{t.lensStop}</span>
                    </button>
                  )}
                </div>
              )
              return (
              <div ref={chatScrollRef} className="h-full overflow-y-auto custom-scrollbar px-3.5 py-3">
                {/* desc 模式下操作按钮放最前（贴最新答案） */}
                {messageOrder === 'desc' && showActions && Actions}
                {ordered.map((m, displayIdx) => {
                  const origIdx = messageOrder === 'desc' ? messages.length - 1 - displayIdx : displayIdx
                  const isUser = m.role === 'user'
                  const isLast = origIdx === lastChronoIdx
                  return (
                    <div key={origIdx} className={`mb-3 ${isUser ? 'flex justify-end' : ''}`}>
                      {isUser ? (
                        <div className="px-3 py-2 rounded-2xl bg-[#D97757]/15 dark:bg-[#D97757]/20 text-[13.5px] text-neutral-800 dark:text-neutral-100 max-w-[88%] whitespace-pre-wrap break-words">
                          {m.content}
                        </div>
                      ) : (
                        <div className="prose prose-sm dark:prose-invert max-w-none text-[13.5px] leading-7 text-neutral-800 dark:text-neutral-200">
                          {m.reasoning && (
                            <ThinkingBlock
                              reasoning={m.reasoning}
                              active={isLast && streaming && !m.content}
                              thinkingLabel={t.lensThinking}
                              thoughtLabel={t.lensThought}
                            />
                          )}
                          {m.content ? (
                            <ReactMarkdown remarkPlugins={[remarkMath]} rehypePlugins={[rehypeKatex]}>
                              {m.content}
                            </ReactMarkdown>
                          ) : isLast && streaming && !m.reasoning ? (
                            <div className="not-prose flex items-center gap-2 text-neutral-500 dark:text-neutral-400">
                              <Loader2 className="animate-spin" size={14} />
                              <span className="text-[12px]">{t.lensAsking}</span>
                            </div>
                          ) : null}
                        </div>
                      )}
                    </div>
                  )
                })}
                {/* asc 模式下操作按钮在末尾 */}
                {messageOrder === 'asc' && showActions && Actions}
              </div>
              )
            })()}
          </div>
        </div>
      )}

      {/* translate 模式浮动结果卡：原文 + 译文，复用 barRect 锚点。
          外层 select-none 用 select-text 覆盖，让用户可选中复制部分文本。 */}
      {showTranslateCard && (
        <div
          className="absolute ease-out rounded-2xl bg-white dark:bg-neutral-900 shadow-[0_10px_28px_-20px_rgba(0,0,0,0.28)] ring-1 ring-black/[0.04] dark:ring-white/[0.06] overflow-hidden select-text"
          onMouseDown={(e) => e.stopPropagation()}
          onMouseMove={(e) => e.stopPropagation()}
          onMouseUp={(e) => e.stopPropagation()}
          onClick={(e) => e.stopPropagation()}
          style={{
            left: barRect.x,
            top: barRect.y,
            width: barRect.width,
            maxHeight: !keepFullscreen
              ? READY_BAR_H + 8 + stableAnswerHeight
              : Math.min(viewport.h - 32, READY_BAR_H + 8 + stableAnswerHeight),
            transitionProperty: barNoTransition ? 'none' : 'left, top, width, transform, opacity',
            transitionDuration: barNoTransition ? '0ms' : `${TRANSITION_MS}ms`,
            transitionTimingFunction: 'cubic-bezier(0.22, 1, 0.36, 1)',
            transform: barIntro ? 'scale(1)' : 'scale(0.92)',
            opacity: barIntro ? 1 : 0,
          }}
          data-tauri-drag-region="false"
        >
          {/* 顶部缩略图 + 应用名 + 状态徽章（耗时 / token 估算） */}
          <div className="flex items-center gap-2.5 px-3.5 py-2.5 border-b border-black/[0.05] dark:border-white/[0.06]">
            <div className="shrink-0 w-8 h-8 rounded-lg overflow-hidden ring-1 ring-black/[0.06] dark:ring-white/[0.06] bg-neutral-100 dark:bg-neutral-800 flex items-center justify-center">
              {imagePreview ? (
                <img src={imagePreview} alt="snap" className="w-full h-full object-cover" />
              ) : (
                <ImageIcon size={12} className="text-neutral-400" />
              )}
            </div>
            <span className="text-[12.5px] font-medium text-neutral-700 dark:text-neutral-300 truncate flex-1">
              {appLabel || t.lensScreenshotOf.replace('：', '').replace(':', '')}
            </span>
            {(() => {
              const elapsedMs = stage === 'translating' && translateStartRef.current
                ? translateNow - translateStartRef.current
                : translateDurationMs
              const seconds = elapsedMs !== null ? Math.max(1, Math.round(elapsedMs / 1000)) : null
              const tokens = formatTokens(estimateTokens(translateOriginal + translateText))
              return (
                <span className="shrink-0 flex items-center gap-1 text-[10.5px] text-neutral-400 dark:text-neutral-500 tabular-nums">
                  {seconds !== null && <span>{seconds}s</span>}
                  {translateText && <span>· ~{tokens} tokens</span>}
                </span>
              )
            })()}
          </div>

          {/* 内容区 */}
          <div className="px-3.5 py-3 overflow-y-auto custom-scrollbar"
            style={{
              maxHeight: !keepFullscreen
                ? stableAnswerHeight
                : Math.min(viewport.h - 110, stableAnswerHeight)
            }}>
            {translateError ? (
              <div className="text-[12.5px] text-red-500 leading-6 whitespace-pre-wrap break-words">
                {t.lensError}: {translateError}
              </div>
            ) : (
              <>
                {/* 译文区（主体）：合并模式下分隔符前的所有 delta 都属于这块，先于原文出现 */}
                {translateText ? (
                  <div className="prose prose-sm dark:prose-invert max-w-none text-[13.5px] leading-7 text-neutral-800 dark:text-neutral-200">
                    <ReactMarkdown remarkPlugins={[remarkMath]} rehypePlugins={[rehypeKatex]}>
                      {translateText}
                    </ReactMarkdown>
                  </div>
                ) : (
                  <div className="space-y-2">
                    <div className="h-3.5 rounded bg-gradient-to-r from-neutral-200 via-neutral-100 to-neutral-200 dark:from-neutral-800 dark:via-neutral-700 dark:to-neutral-800 bg-[length:200%_100%] animate-[shimmer_1.4s_linear_infinite]" />
                    <div className="h-3.5 rounded bg-gradient-to-r from-neutral-200 via-neutral-100 to-neutral-200 dark:from-neutral-800 dark:via-neutral-700 dark:to-neutral-800 bg-[length:200%_100%] animate-[shimmer_1.4s_linear_infinite] w-[88%]" />
                    <div className="h-3.5 rounded bg-gradient-to-r from-neutral-200 via-neutral-100 to-neutral-200 dark:from-neutral-800 dark:via-neutral-700 dark:to-neutral-800 bg-[length:200%_100%] animate-[shimmer_1.4s_linear_infinite] w-[72%]" />
                  </div>
                )}
                {/* 原文区（参考）：分隔符之后的 delta 才到这里，置于译文下方小字灰色 */}
                {translateOriginal && (
                  <>
                    <div className="border-t border-black/[0.05] dark:border-white/[0.06] -mx-3.5 my-3" />
                    <div className="prose prose-sm dark:prose-invert max-w-none text-[12.5px] leading-6 text-neutral-500 dark:text-neutral-400">
                      <ReactMarkdown remarkPlugins={[remarkMath]} rehypePlugins={[rehypeKatex]}>
                        {translateOriginal}
                      </ReactMarkdown>
                    </div>
                  </>
                )}
              </>
            )}
          </div>

          {/* 底部操作栏：复制译文 */}
          {stage === 'translated' && translateText && !translateError && (
            <div className="flex items-center gap-1 px-3 py-1.5 border-t border-black/[0.05] dark:border-white/[0.06]">
              <button
                onClick={async () => {
                  if (await copyToClipboard(translateText)) {
                    setCopied(true)
                    if (copyTimeoutRef.current) clearTimeout(copyTimeoutRef.current)
                    copyTimeoutRef.current = setTimeout(() => setCopied(false), 2000)
                  }
                }}
                className="flex items-center gap-1 px-2 py-0.5 text-[11px] text-neutral-500 hover:text-neutral-800 dark:text-neutral-400 dark:hover:text-neutral-100 rounded hover:bg-black/5 dark:hover:bg-white/10 transition-colors"
              >
                {copied ? <Check size={12} /> : <Copy size={12} />}
                <span>{copied ? t.lensCopied : t.lensCopy}</span>
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  )
}
