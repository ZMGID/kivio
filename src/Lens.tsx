import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'
import { flushSync } from 'react-dom'
import { Loader2, Copy, Check, Square, Image as ImageIcon, ArrowUp, History as HistoryIcon, ChevronDown, MousePointer2, Code, Eye, Globe, X } from 'lucide-react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { api, type LensStreamPayload, type LensTranslateStreamPayload, type LensWindowInfo, type ExplainMessage, type LensWebSearchPayload, type LensQuickAction } from './api/tauri'
import ReactMarkdown from 'react-markdown'
import remarkMath from 'remark-math'
import rehypeKatex from 'rehype-katex'
import 'katex/dist/katex.min.css'
import { i18n, type Lang } from './settings/i18n'
import { copyToClipboard } from './utils/clipboard'

import type { Arrow, BarRect, CapturedFrame, HistoryItem, Metrics, Mode, Point, Stage, TranslateCardDrag } from './lens/types'
import { ArrowSvg } from './lens/ArrowSvg'
import { ARROW_MIN_DRAG_PX, composeAnnotatedImage } from './lens/annotation'
import { HISTORY_MAX, HISTORY_THUMB_SIZE, loadHistoryFromStorage, makeThumbnail, saveHistoryToStorage } from './lens/history'
import { ANCHOR_GAP, DRAG_THRESHOLD, FLOATING_GAP, FLOATING_PADDING, READY_BAR_H, SELECT_REVEAL_DELAY_MS, TRANSITION_MS, clamp, computeMetrics, computeSelectBar, isMacPlatform } from './lens/layout'
import { estimateTokens, formatTokens } from './lens/markdown'
import { ThinkingBlock } from './lens/ThinkingBlock'
import { WebSearchBlock } from './lens/WebSearchBlock'
import { useWindowInteractionFocus } from './utils/windowFocus'

/** 解析 webview hash query：'#lens?mode=translate' → 'translate' */
function readModeFromHash(): Mode {
  if (typeof window === 'undefined') return 'chat'
  const hash = window.location.hash || ''
  const q = hash.indexOf('?')
  if (q < 0) return 'chat'
  const params = new URLSearchParams(hash.slice(q + 1))
  const mode = params.get('mode')
  if (mode === 'translate') return 'translate'
  if (mode === 'translateText') return 'translateText'
  return 'chat'
}

const makeTextRequestId = () => `text-${Date.now()}-${Math.random().toString(36).slice(2)}`

type LensResetFrame = {
  x: number
  y: number
  width?: number
  height?: number
}

type LensResetPayload = {
  frame?: LensResetFrame
  freezeFrameImageId?: string
}

function readLensResetPayload(detail: unknown): LensResetPayload {
  if (!detail || typeof detail !== 'object') return {}
  const frame = (detail as { frame?: unknown }).frame
  const freezeFrameImageId = (detail as { freezeFrameImageId?: unknown }).freezeFrameImageId
  const payload: LensResetPayload = {
    freezeFrameImageId: typeof freezeFrameImageId === 'string' ? freezeFrameImageId : undefined,
  }
  if (!frame || typeof frame !== 'object') return payload
  const { x, y, width, height } = frame as Partial<LensResetFrame>
  if (!Number.isFinite(x) || !Number.isFinite(y)) return payload
  payload.frame = {
    x: x as number,
    y: y as number,
    width: Number.isFinite(width) ? width : undefined,
    height: Number.isFinite(height) ? height : undefined,
  }
  return {
    ...payload,
  }
}

const waitForFrames = (frames: number) => new Promise<void>((resolve) => {
  const step = (remaining: number) => {
    if (remaining <= 0) {
      resolve()
      return
    }
    requestAnimationFrame(() => step(remaining - 1))
  }
  step(frames)
})

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
  const [webSearchAvailable, setWebSearchAvailable] = useState(false)
  const [webSearchEnabled, setWebSearchEnabled] = useState(false)
  const [keepFullscreen, setKeepFullscreen] = useState(() => readModeFromHash() !== 'translateText')
  const [floatingRebased, setFloatingRebased] = useState(false)
  const [mode, setMode] = useState<Mode>(() => readModeFromHash())
  // translate 模式专用：OCR 原文 + 翻译结果 + 计时
  const [translateOriginal, setTranslateOriginal] = useState('')
  const [translateText, setTranslateText] = useState('')
  const [translateError, setTranslateError] = useState('')
  const [translateDurationMs, setTranslateDurationMs] = useState<number | null>(null)
  const [translateNow, setTranslateNow] = useState(() => Date.now())
  const translateStartRef = useRef<number | null>(null)
  const [freezeFrameImageId, setFreezeFrameImageId] = useState('')
  const [freezeFramePreview, setFreezeFramePreview] = useState('')
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
  const [barIntro, setBarIntro] = useState(false)
  // barNoTransition：reset/drag/窗口裁剪切换时临时禁用 transition，避免上次动画在 hide 后续播。
  const [barNoTransition, setBarNoTransition] = useState(true)
  // flyDelta：全屏覆盖模式下 fly 动画用 transform translate 取代 left/top 过渡。
  // left/top 不是 GPU 合成属性，每帧都要走 layout/reflow；Windows 上 webview hide→show 后
  // 合成器刚被唤醒、首个大幅 left/top 过渡极易卡顿（"乱跳"）。改为：left/top 立即 snap 到
  // 最终位置，用 transform: translate(dx, dy) 把视觉位置拉回起点，下一帧再把 delta 过渡到 (0,0)。
  // transform 走合成层，不阻塞主线程，多窗口会话间稳定。
  const [flyDelta, setFlyDelta] = useState<{ x: number; y: number }>({ x: 0, y: 0 })
  const [translateCardDragging, setTranslateCardDragging] = useState(false)
  // capturedFrame：保留最后一次截图选区/窗口的高亮框，作为"已截图"视觉标记，ready/answering 态继续显示
  const [capturedFrame, setCapturedFrame] = useState<CapturedFrame | null>(null)
  const [showCaptureHint, setShowCaptureHint] = useState(false)
  // 箭头标注:仅 stage==='ready' 子模式
  // arrows / draftArrow 坐标系 = capturedFrame 逻辑像素 (左上角为原点)
  const [drawMode, setDrawMode] = useState(false)
  const [arrows, setArrows] = useState<Arrow[]>([])
  // 源码/渲染切换：false=渲染模式(ReactMarkdown)，true=源码模式(原始文本)
  const [sourceMode, setSourceMode] = useState(false)
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
  const [historyPanelH, setHistoryPanelH] = useState(0)
  const [quickActions, setQuickActions] = useState<LensQuickAction[]>([])
  const pendingQuickActionRef = useRef<LensQuickAction | null>(null)

  const rootRef = useRef<HTMLDivElement>(null)
  const inputRef = useRef<HTMLInputElement>(null)
  const historyPanelRef = useRef<HTMLDivElement>(null)
  const historyContentRef = useRef<HTMLDivElement>(null)
  const barRef = useRef<HTMLDivElement>(null)
  const stageRef = useRef<Stage>('select')
  const modeRef = useRef<Mode>(mode)
  const historyOpenRef = useRef(false)
  const drawModeRef = useRef(false)
  const imageIdRef = useRef('')
  const copyTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const floatingRebaseTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const focusReqIdRef = useRef(0)
  const motionSeqRef = useRef(0)
  const selectRevealTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const selectRevealedRef = useRef(false)
  const captureHintEnabledRef = useRef(true)
  const screenshotKeepFullscreenRef = useRef(true)
  const prevStreamingRef = useRef(false)
  const preparingSendRef = useRef(false)
  const answerFinishedRef = useRef(false)
  const lastLensStreamEventRef = useRef('')
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
  const translateCardDragRef = useRef<TranslateCardDrag | null>(null)
  // 答案区滚动容器，stream 时自动滚到底部
  const chatScrollRef = useRef<HTMLDivElement>(null)
  // 浮动模式下保存截图时的全屏 metrics，避免窗口缩小后 answerLayout 被压缩得太小
  const fullscreenMetricsRef = useRef<Metrics | null>(null)
  const requestWindowFocus = useWindowInteractionFocus()

  const t = i18n[lang]
  stageRef.current = stage
  modeRef.current = mode
  historyOpenRef.current = historyOpen
  drawModeRef.current = drawMode

  const finishAnswering = useCallback(() => {
    if (answerFinishedRef.current) return
    answerFinishedRef.current = true
    // 必须在 setStreaming(false) 前置 true：历史持久化 effect 依赖 streaming 变化触发。
    justFinishedStreamRef.current = true
    setStreaming(false)
  }, [])

  const cancelPendingMotion = useCallback(() => {
    motionSeqRef.current++
    if (selectRevealTimerRef.current) {
      clearTimeout(selectRevealTimerRef.current)
      selectRevealTimerRef.current = null
    }
    if (floatingRebaseTimerRef.current) {
      clearTimeout(floatingRebaseTimerRef.current)
      floatingRebaseTimerRef.current = null
    }
  }, [])

  // 选中文本行数：translate 模式不计；空 / 仅空白 → 0（驱动徽章是否显示）
  const selectionLineCount = useMemo(() => {
    if (mode !== 'chat') return 0
    if (!selectionText.trim()) return 0
    return selectionText.split(/\r?\n/).length
  }, [selectionText, mode])

  const loadLensSettings = useCallback(async (curMode: Mode = readModeFromHash()) => {
    try {
      const settings = await api.getSettings()
      setLang((settings.settingsLanguage as Lang) || 'zh')
      setMessageOrder(settings.lens?.messageOrder === 'desc' ? 'desc' : 'asc')
      const webSearch = settings.lens?.webSearch
      const hasWebSearchKey = webSearch?.provider === 'exa'
        ? !!webSearch.exaApiKey?.trim()
        : !!webSearch?.tavilyApiKey?.trim()
      const canUseWebSearch = webSearch?.enabled === true && hasWebSearchKey
      setWebSearchAvailable(canUseWebSearch)
      setWebSearchEnabled(canUseWebSearch && curMode === 'chat')
      screenshotKeepFullscreenRef.current = settings.screenshotTranslation?.keepFullscreenAfterCapture !== false
      setKeepFullscreen(curMode === 'chat' || (curMode === 'translate' && screenshotKeepFullscreenRef.current))
      captureHintEnabledRef.current = settings.lens?.showCaptureHint !== false
      setQuickActions(settings.lens?.quickActions || [])
    } catch (err) { console.error('Failed to load settings', err) }
  }, [])

  // 加载设置：普通 Lens 截图后固定保持全屏覆盖；截图翻译仍读自己的保留全屏配置。
  useEffect(() => {
    void loadLensSettings()
  }, [loadLensSettings])

  const focusLensSurface = useCallback((delays: number[] = [0, 40, 120, 240, 420]) => {
    const requestId = ++focusReqIdRef.current
    const canFocus = () => {
      if (requestId !== focusReqIdRef.current) return false
      if (historyOpenRef.current || capturingRef.current) return false
      if (modeRef.current === 'chat') {
        return stageRef.current === 'select' || stageRef.current === 'ready' || stageRef.current === 'answering'
      }
      return stageRef.current === 'select' || stageRef.current === 'translating' || stageRef.current === 'translated'
    }

    const run = async () => {
      if (!canFocus()) return
      try {
        await getCurrentWindow().setFocus()
      } catch {
        // Native focus can fail briefly while the window is still becoming visible.
      }
      if (!canFocus()) return
      const focusTarget = modeRef.current === 'chat' ? inputRef.current : rootRef.current
      focusTarget?.focus({ preventScroll: true })
      requestAnimationFrame(() => {
        if (!canFocus()) return
        const nextFocusTarget = modeRef.current === 'chat' ? inputRef.current : rootRef.current
        nextFocusTarget?.focus({ preventScroll: true })
      })
    }

    delays.forEach(delay => window.setTimeout(() => { void run() }, delay))
  }, [])

  // select 态进入：刷新所有 state、重算对话栏位置、播放 intro 动画
  const enterSelect = useCallback(async (resetPayload: LensResetPayload = {}) => {
    const curMode = readModeFromHash()
    await loadLensSettings(curMode)
    const resetFrame = resetPayload.frame
    const resetFreezeFrameImageId = resetPayload.freezeFrameImageId ?? ''
    cancelPendingMotion()
    const motionSeq = motionSeqRef.current
    fullscreenMetricsRef.current = null
    // 防御：reset 流程会 setMessages([]) + setStreaming(false)，理论上 messages.length===0 effect 不会进
    // 持久化分支，但显式清零更稳
    justFinishedStreamRef.current = false
    // 用 flushSync 同步提交所有 reset 后的状态：webview show 之前 DOM 必须已经反映新位置，
    // 否则 Rust 的 show() 会先把旧 frame 露出来。
    // barNoTransition 同 frame 一起置 true → bar 从老坐标 snap 到 select 坐标，不回放动画。
    flushSync(() => {
      setBarNoTransition(true)
      setStage(curMode === 'translateText' ? 'translating' : 'select')
      setMode(curMode)
      setKeepFullscreen(curMode === 'chat' || (curMode === 'translate' && screenshotKeepFullscreenRef.current))
      setFloatingRebased(false)
      setHovered(null)
      setDragStart(null)
      setDragCurrent(null)
      setDragging(false)
      setTranslateCardDragging(false)
      setImagePreview('')
      setAppLabel('')
      setInput('')
      setSelectionText('')
      setMessages([])
      setStreaming(false)
      setTranslateOriginal('')
      setTranslateText('')
      setTranslateError('')
      setFreezeFrameImageId(resetFreezeFrameImageId)
      setFreezeFramePreview('')
      const w = resetFrame?.width ?? window.innerWidth
      const h = resetFrame?.height ?? window.innerHeight
      setViewport({ w, h })
      const m = computeMetrics(w, h)
      setBarRect(curMode === 'translateText'
        ? { x: 0, y: 0, width: Math.min(m.READY_W, w) }
        : computeSelectBar(w, h, m))
      setFlyDelta({ x: 0, y: 0 })
      setCapturedFrame(null)
      // 重置 intro：先关再开，下一帧让 transition 从 scale-90 到 scale-100
      setBarIntro(false)
      setShowCaptureHint(false)
      if (resetFrame) setWinOrigin({ x: resetFrame.x, y: resetFrame.y })
    })
    selectRevealedRef.current = false
    imageIdRef.current = ''
    translateCardDragRef.current = null
    focusLensSurface([0, 40, 120])
    if (resetFreezeFrameImageId) {
      void (async () => {
        try {
          const img = await api.explainReadImage(resetFreezeFrameImageId)
          if (motionSeq === motionSeqRef.current && img.success) {
            setFreezeFramePreview(img.data ?? '')
          }
        } catch (err) {
          console.error('Failed to load freeze frame', err)
        }
      })()
    }
    // 重新加载设置：用户在设置面板修改后关闭再打开 Lens，需要读到最新值。
    // 必须放在 reset DOM 之后，避免 await 期间 Rust 已 show 导致旧 ready/answering surface 露出首帧。
    void (async () => {
      try {
        const settings = await api.getSettings()
        if (motionSeq !== motionSeqRef.current) return
        screenshotKeepFullscreenRef.current = settings.screenshotTranslation?.keepFullscreenAfterCapture !== false
        setKeepFullscreen(curMode === 'chat' || (curMode === 'translate' && screenshotKeepFullscreenRef.current))
        captureHintEnabledRef.current = settings.lens?.showCaptureHint !== false
        if (stageRef.current === 'select' && selectRevealedRef.current) {
          setShowCaptureHint(captureHintEnabledRef.current)
        }
        setQuickActions(settings.lens?.quickActions || [])
      } catch (err) { console.error('Failed to reload settings', err) }
    })()
    // 异步 take 走 Rust 端在 lens_request_internal 中暂存的选中文本。
    // token 防御：take 期间用户再开一次 Lens / 关闭，老 promise 落地时 myReq 已过期，丢弃。
    // 仅 chat 模式注入；> 200KB 直接丢弃避免上下文爆炸；trim 后非空才 setSelectionText。
    const myReq = ++selectionReqIdRef.current
    if (curMode === 'translateText') {
      focusLensSurface()
      void (async () => {
        try {
          const text = await api.takeLensSelection()
          if (myReq !== selectionReqIdRef.current) return
          if (text.length > 200_000 || !text.trim()) {
            void api.lensClose()
            return
          }
          const requestId = makeTextRequestId()
          imageIdRef.current = requestId
          setSelectionText(text)
          if (motionSeq === motionSeqRef.current) {
            requestAnimationFrame(() => {
              requestAnimationFrame(() => {
                if (motionSeq !== motionSeqRef.current) return
                selectRevealedRef.current = true
                setShowCaptureHint(false)
                setBarIntro(true)
                setBarNoTransition(false)
              })
            })
          }
          setTranslateOriginal('')
          setTranslateText('')
          setTranslateError('')
          setTranslateDurationMs(null)
          translateStartRef.current = Date.now()
          setTranslateNow(Date.now())
          try {
            const result = await api.lensTranslateText(text, requestId)
            if (!result.success) {
              setTranslateError(result.error || 'Failed')
              if (translateStartRef.current !== null) {
                setTranslateDurationMs(Date.now() - translateStartRef.current)
                translateStartRef.current = null
              }
              setStage('translated')
            }
          } catch (err) {
            setTranslateError(err instanceof Error ? err.message : String(err))
            if (translateStartRef.current !== null) {
              setTranslateDurationMs(Date.now() - translateStartRef.current)
              translateStartRef.current = null
            }
            setStage('translated')
          }
        } catch (err) {
          console.warn('[lens] take selection failed:', err)
          void api.lensClose()
        }
      })()
      return
    }
    if (curMode === 'chat') {
      void (async () => {
        try {
          const text = await api.takeLensSelection()
          if (myReq !== selectionReqIdRef.current || motionSeq !== motionSeqRef.current) return
          if (text.length > 200_000) return
          if (text.trim()) {
            setSelectionText(text)
            focusLensSurface([0, 60, 180])
          }
        } catch (err) {
          console.warn('[lens] take selection failed:', err)
        }
      })()
    }
    selectRevealTimerRef.current = setTimeout(() => {
      selectRevealTimerRef.current = null
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          if (motionSeq !== motionSeqRef.current) return
          // 第二个 raf 同时恢复 transitions 并触发 intro：现在 bar 已经在 select 位置，
          // 只对 transform/opacity 做缩放进入动画，不会回放历史 left/top 过渡。
          selectRevealedRef.current = true
          setShowCaptureHint(captureHintEnabledRef.current)
          setBarIntro(true)
          setBarNoTransition(false)
        })
      })
    }, SELECT_REVEAL_DELAY_MS)
    if (!resetFrame) {
      await waitForFrames(2)
      try {
        const win = getCurrentWindow()
        const [pos, scale] = await Promise.all([win.outerPosition(), win.scaleFactor()])
        const sf = scale || 1
        if (motionSeq === motionSeqRef.current) {
          setWinOrigin({ x: pos.x / sf, y: pos.y / sf })
        }
      } catch (err) { console.error('Failed to read window origin', err) }
    }
    try {
      const list = await api.lensListWindows()
      if (motionSeq === motionSeqRef.current) setWindows(list)
    } catch (err) {
      console.error('Failed to list windows', err)
      if (motionSeq === motionSeqRef.current) setWindows([])
    }
    focusLensSurface()
  }, [cancelPendingMotion, focusLensSurface, loadLensSettings])

  useEffect(() => {
    void enterSelect()
    const handleReset = (event: Event) => {
      void enterSelect(readLensResetPayload((event as CustomEvent).detail))
    }
    window.addEventListener('lens:reset', handleReset)
    return () => {
      window.removeEventListener('lens:reset', handleReset)
      cancelPendingMotion()
    }
  }, [enterSelect, cancelPendingMotion])

  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined
    getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (focused) focusLensSurface([0, 40, 120])
    }).then((dispose) => {
      if (cancelled) dispose()
      else unlisten = dispose
    }).catch(err => console.error('[lens-focus] listen failed:', err))
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [focusLensSurface])

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
    } else if (modeRef.current === 'translateText' && stageRef.current === 'translating') {
      const w = Math.min(metrics.READY_W, viewport.w)
      setBarRect({ x: 0, y: 0, width: w })
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
      try {
        // Persist the image before writing the history row. Otherwise a fast close
        // can delete the temp file and leave an unusable history item behind.
        await api.lensCommitImageToHistory(id)
      } catch (err) {
        console.error('[lens-history] commit failed:', err)
        return
      }
      const thumb = await makeThumbnail(imagePreview, HISTORY_THUMB_SIZE)
      if (cancelled) return
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
        lastLensStreamEventRef.current = ''
        finishAnswering()
        return
      }
      const eventKey = [
        payload.imageId,
        payload.kind,
        payload.delta ?? '',
        payload.reasoningDelta ?? '',
      ].join('\u0000')
      if (eventKey === lastLensStreamEventRef.current) return
      lastLensStreamEventRef.current = eventKey
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
  }, [finishAnswering])

  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined
    api.onLensWebSearch((payload: LensWebSearchPayload) => {
      if (payload.imageId !== imageIdRef.current) return
      setMessages(prev => {
        const last = prev[prev.length - 1]
        if (!last || last.role !== 'assistant') return prev
        return [
          ...prev.slice(0, -1),
          {
            ...last,
            webSearch: {
              status: payload.status,
              query: payload.query,
              reason: payload.reason,
              results: payload.results,
              error: payload.error,
            },
          },
        ]
      })
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
      focusLensSurface([0, 60, 160])
    }, 30)
    return () => clearTimeout(id)
  }, [streaming, mode, historyOpen, focusLensSurface])

  useEffect(() => {
    if (mode === 'chat') return
    if (stage !== 'translating' && stage !== 'translated') return
    focusLensSurface([0, 60, 180])
  }, [mode, stage, focusLensSurface])

  // 关闭前同步重置 state，让 webview surface 在 hide 之前已经是空 select 态。
  // 否则下次 show 时 macOS 会先显示上次的 ready 态 surface 一帧，再被 lens:reset 覆盖 → 闪一下上次内容。
  // barNoTransition：禁用 transition，避免 380ms 动画被 hide 暂停后下次 show 续播。
  const resetBeforeHide = useCallback(() => {
    cancelPendingMotion()
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
      setFreezeFrameImageId('')
      setFreezeFramePreview('')
      setAppLabel('')
      setInput('')
      setSelectionText('')
      setMessages([])
      setStreaming(false)
      setBarRect(computeSelectBar(viewport.w, viewport.h, metrics))
      setFlyDelta({ x: 0, y: 0 })
      setCapturedFrame(null)
      setBarIntro(false)
      setShowCaptureHint(false)
    })
    selectRevealedRef.current = false
    imageIdRef.current = ''
    // 让任何还没落地的 takeLensSelection 老 promise 作废，避免关闭后 setSelectionText 拖回来
    selectionReqIdRef.current++
    focusReqIdRef.current++
  }, [cancelPendingMotion, viewport, metrics])

  const closeAfterReset = useCallback(async () => {
    resetBeforeHide()
    const closeMotionSeq = motionSeqRef.current
    await waitForFrames(2)
    if (closeMotionSeq !== motionSeqRef.current) return
    try { await api.lensClose() } catch (err) { console.error(err) }
  }, [resetBeforeHide])

  // 全局 Esc：流式时取消流 / 否则关闭
  useEffect(() => {
    const handler = async (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      e.preventDefault()
      e.stopPropagation()
      if (preparingSendRef.current) return
      if (drawModeRef.current) return
      if (stageRef.current === 'answering' && streaming) {
        try { await api.lensCancelStream() } catch (err) { console.error(err) }
        setStreaming(false)
        return
      }
      await closeAfterReset()
    }
    window.addEventListener('keydown', handler, true)
    return () => window.removeEventListener('keydown', handler, true)
  }, [streaming, closeAfterReset])

  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined
    api.onLensCloseRequest(() => {
      if (cancelled) return
      void closeAfterReset()
    }).then((dispose) => {
      if (cancelled) dispose()
      else unlisten = dispose
    }).catch(err => console.error(err))
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [closeAfterReset])

  // drawMode 键盘:Cmd+Z 撤销最后一支箭头,Esc 退出 drawMode(arrows 保留)
  useEffect(() => {
    if (!drawMode) return
    const onKey = (e: KeyboardEvent) => {
      // 输入框聚焦时不拦截,让用户继续打字
      const target = e.target as HTMLElement | null
      const isInput = target?.tagName === 'INPUT' || target?.tagName === 'TEXTAREA'

      // Esc:无论焦点在哪都退出 drawMode,并阻止全局 Esc 关掉 Lens
      // (输入栏 autoFocus 时 isInput=true,但 Esc 在输入框里没有合法语义,直接接管)
      if (e.key === 'Escape') {
        e.preventDefault()
        e.stopPropagation()
        e.stopImmediatePropagation()
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
        void closeAfterReset()
      }
    }
    window.addEventListener('blur', handleBlur)
    return () => window.removeEventListener('blur', handleBlur)
  }, [closeAfterReset])

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

  const captureHintText = useMemo(() => {
    if (dragging) return t.lensSelectHintDrag
    if (hovered) return t.lensSelectHintHover.replace('{app}', hovered.owner)
    return t.lensSelectHintIdle
  }, [dragging, hovered, t])

  const handleMouseDown = (e: React.MouseEvent) => {
    if (stage !== 'select') return
    // 点击在对话栏内部时不开始拖动，让输入框/按钮等正常交互
    if (barRef.current?.contains(e.target as Node)) return
    // 历史面板展开时点击外层只关闭面板，不开始拖动/截图
    if (historyOpenRef.current) {
      setHistoryOpen(false)
      return
    }
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
    // 鼠标在对话栏（含历史面板）上方时清除 hover，避免高亮/误截图背后窗口
    if (barRef.current?.contains(e.target as Node)) {
      setHovered(null)
      return
    }
    const gp = clientToGlobal(p)
    setHovered(hitTest(gp))
  }

  const animateFullscreenBarToAnchor = useCallback((
    targetRect: BarRect,
    targetStage: Stage,
    label: string,
    motionSeq: number,
  ) => {
    const startX = barRect.x
    const startY = barRect.y

    flushSync(() => {
      setAppLabel(label)
      setBarNoTransition(true)
      setBarRect(targetRect)
      setFlyDelta({ x: startX - targetRect.x, y: startY - targetRect.y })
      setBarIntro(true)
      setStage(targetStage)
    })

    // Commit the snap+offset first, then allow only transform/opacity to transition.
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        if (motionSeq !== motionSeqRef.current) return
        if (stageRef.current === 'select') return
        setBarNoTransition(false)
        setFlyDelta({ x: 0, y: 0 })
      })
    })
  }, [barRect])

  /** 截图后在前端直接算 bar 位置，让对话栏飞到选区左/右侧。
   *  优先右侧，右侧空间不够再放左侧；都不够时贴大空间一侧。 */
  const flyBarToAnchor = async (
    anchorAbsX: number,
    anchorAbsY: number,
    anchorW: number,
    anchorH: number,
    label: string,
  ) => {
    cancelPendingMotion()
    const motionSeq = motionSeqRef.current
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
      fullscreenMetricsRef.current = metrics
      const finalX = Math.round(targetX)
      const finalY = Math.round(targetY)
      const startX = barRect.x
      const startY = barRect.y
      const floatW = READY_W + FLOATING_PADDING * 2
      const floatH = targetStage === 'ready'
        ? READY_BAR_H + FLOATING_PADDING * 2
        : READY_BAR_H + FLOATING_GAP + metrics.ANSWER_H + FLOATING_PADDING * 2
      const isTranslateMode = mode === 'translate'

      if (isMacPlatform) {
        // macOS:走 AppKit 原生 NSAnimationContext + animator setFrame:。
        // 一次 IPC 触发,Core Animation 在合成器线程按显示器原生刷新率插值,
        // 不再有 JS rAF 每帧打 IPC + 两次独立 AppKit 调用导致的 coalescing 掉帧。
        // 时间曲线 cubic-bezier(0.22, 1, 0.36, 1) 与原 CSS transition / rAF 完全一致。
        const floatX = winOrigin.x + finalX - FLOATING_PADDING
        const floatY = winOrigin.y + finalY - FLOATING_PADDING

        flushSync(() => {
          setAppLabel(label)
          setFloatingRebased(false)
          setBarRect({ x: FLOATING_PADDING, y: FLOATING_PADDING, width: READY_W })
          setFlyDelta({ x: 0, y: 0 })
          setStage(targetStage)
          if (isTranslateMode) {
            // translate 卡片截图前不渲染 → 没"起点位置",禁 transition 避免 (selectX,selectY) → (0,0) 瞬时跳动触发动画
            setBarIntro(false)
            setBarNoTransition(true)
          } else {
            setBarIntro(true)
            setBarNoTransition(false)
          }
        })
        if (isTranslateMode) {
          requestAnimationFrame(() => {
            requestAnimationFrame(() => {
              if (motionSeq === motionSeqRef.current) setBarNoTransition(false)
            })
          })
        }

        if (floatingRebaseTimerRef.current) clearTimeout(floatingRebaseTimerRef.current)
        void api.lensAnimateFloating({
          x: floatX,
          y: floatY,
          width: floatW,
          height: floatH,
          durationMs: TRANSITION_MS,
        }).catch((err: unknown) => console.error('[lens] lensAnimateFloating failed:', err))

        // AppKit 动画在原生侧异步跑;+40ms 余量等 Core Animation 收尾,再切 floatingRebased。
        // 加 motionSeq + stage 守卫防止用户中途触发新会话时把旧会话的尾巴覆盖到新窗口。
        floatingRebaseTimerRef.current = window.setTimeout(() => {
          floatingRebaseTimerRef.current = null
          if (motionSeq !== motionSeqRef.current) return
          if (stageRef.current === 'select') return
          setFloatingRebased(true)
          if (isTranslateMode) setBarIntro(true)
        }, TRANSITION_MS + 40)
      } else {
        // Windows: WebView 始终保持全屏,Rust 用 SetWindowRgn 把窗口可见区域裁剪到 bar 矩形。
        // 不走 macOS 的逐帧实搬窗口路线是因为多次 lens 会话后 WebView2 内部状态会退化导致累积型 jitter。
        flushSync(() => {
          setAppLabel(label)
          setFloatingRebased(false)
          setBarNoTransition(true)
          setBarRect({ x: finalX, y: finalY, width: READY_W })
          setFlyDelta({ x: startX - finalX, y: startY - finalY })
          setStage(targetStage)
          setBarIntro(!isTranslateMode)
        })

        if (floatingRebaseTimerRef.current) clearTimeout(floatingRebaseTimerRef.current)
        requestAnimationFrame(() => {
          requestAnimationFrame(() => {
            if (motionSeq !== motionSeqRef.current) return
            setBarNoTransition(false)
            setFlyDelta({ x: 0, y: 0 })

            floatingRebaseTimerRef.current = window.setTimeout(() => {
              floatingRebaseTimerRef.current = null
              if (motionSeq !== motionSeqRef.current || stageRef.current === 'select') return

              void api.lensSetFloating({ x: finalX - FLOATING_PADDING, y: finalY - FLOATING_PADDING, width: floatW, height: floatH })
                .then(() => {
                  if (motionSeq !== motionSeqRef.current || stageRef.current === 'select') return
                  flushSync(() => {
                    setFloatingRebased(true)
                    setBarIntro(true)
                  })
                  requestAnimationFrame(() => {
                    if (motionSeq === motionSeqRef.current) setBarNoTransition(false)
                  })
                })
                .catch((err: unknown) => {
                  console.error('[lens] lensSetFloating rebase failed:', err)
                  if (motionSeq !== motionSeqRef.current) return
                  flushSync(() => {
                    setFloatingRebased(false)
                    setBarNoTransition(true)
                    setBarRect({ x: finalX, y: finalY, width: READY_W })
                    setBarIntro(true)
                  })
                  requestAnimationFrame(() => {
                    if (motionSeq === motionSeqRef.current) setBarNoTransition(false)
                  })
                })
            }, isTranslateMode ? 0 : TRANSITION_MS + 40)
          })
        })
      }
    } else {
      animateFullscreenBarToAnchor(
        { x: Math.round(targetX), y: Math.round(targetY), width: READY_W },
        targetStage,
        label,
        motionSeq,
      )
    }
    if (mode === 'chat') {
      focusLensSurface([TRANSITION_MS + 20, TRANSITION_MS + 120, TRANSITION_MS + 260])
    } else if (mode === 'translate') {
      focusLensSurface([0, 80, 180, TRANSITION_MS + 80])
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
      freezeFrameImageId: freezeFrameImageId || undefined,
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
      setFreezeFrameImageId('')
      setFreezeFramePreview('')

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

    // 在对话栏区域松开时不触发截图（避免点击历史按钮/条目时误截图）
    if (barRef.current?.contains(e.target as Node)) {
      setDragStart(null)
      setDragCurrent(null)
      setDragging(false)
      return
    }

    setDragStart(null)
    setDragCurrent(null)
    setDragging(false)
    if (hovered) {
      await handleCaptureWindow(hovered)
    }
  }

  const doSend = async (question: string) => {
    if (streaming) return
    setHistoryOpen(false)
    answerFinishedRef.current = false

    // 先进入 sending UI，再做合成/注册，避免这段异步窗口被 Esc 关闭掉。
    const isFirstTurn = messages.length === 0
    const hasScreenshot = !!imageIdRef.current
    const ctx = (isFirstTurn && mode === 'chat' && !hasScreenshot) ? selectionText.trim() : ''
    if (!hasScreenshot && !ctx && !question.trim()) return
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
    lastLensStreamEventRef.current = ''
    preparingSendRef.current = true

    // 默认沿用当前 image_id;若有箭头则先合成 + 注册新图,把后续 ask 切到合成版
    try {
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
      preparingSendRef.current = false
      const qa = pendingQuickActionRef.current
      pendingQuickActionRef.current = null
      const result = await api.lensAsk(effectiveImageId || '', sendMessages, {
        webSearch: mode === 'chat' && webSearchEnabled && webSearchAvailable,
        ...(qa ? { providerId: qa.providerId || undefined, model: qa.model || undefined, quickAction: true } : {}),
      })
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
          return [...prev.slice(0, -1), { ...last, content: result.response! }]
        })
      }
      if (result.success && result.webSearchResults?.length) {
        setMessages(prev => {
          const last = prev[prev.length - 1]
          if (!last || last.role !== 'assistant') return prev
          if (last.webSearch?.results?.length) return prev
          return [
            ...prev.slice(0, -1),
            {
              ...last,
              webSearch: {
                status: 'done',
                results: result.webSearchResults,
              },
            },
          ]
        })
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err)
      setMessages(prev => {
        const last = prev[prev.length - 1]
        if (!last || last.role !== 'assistant') return prev
        return [...prev.slice(0, -1), { ...last, content: `${t.lensError}: ${msg}` }]
      })
    } finally {
      preparingSendRef.current = false
      finishAnswering()
    }
  }

  const handleSend = async () => {
    if (streaming) return
    const question = input.trim()
    setInput('')
    await doSend(question)
  }

  const handleQuickAction = async (qa: LensQuickAction) => {
    if (streaming) return
    if (!imageIdRef.current) return
    pendingQuickActionRef.current = qa
    await doSend(qa.prompt)
  }

  const handleStop = async () => {
    try { await api.lensCancelStream() } catch (err) { console.error(err) }
    // 用户主动取消但已经流出部分内容，也持久化 —— 关掉再开历史能接着问
    finishAnswering()
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
      setCapturedFrame(null)
      setStreaming(false)
      setStage('answering')
    })
    // 老 takeLensSelection promise 失效，避免恢复历史后被新 take 文本污染
    selectionReqIdRef.current++
    focusLensSurface([50, 140, 260])
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
    cancelPendingMotion()
    focusReqIdRef.current++
  }, [cancelPendingMotion])

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

  // 测量 history 面板实际高度,供浮动模式 resize 副作用按需扩窗(不然面板上方/下方溢出会被 OS 裁掉)。
  // useLayoutEffect 确保在浏览器 paint 之前同步算出高度并 setState,resize 副作用立刻拿到新值扩窗,
  // 不会出现"面板已渲染但窗口没扩"的中间帧。
  useLayoutEffect(() => {
    if (historyOpen && historyContentRef.current) {
      setHistoryPanelH(historyContentRef.current.offsetHeight)
    } else {
      setHistoryPanelH(0)
    }
  }, [historyOpen, history])

  // ====== 单一渲染 ======
  const showThumb = stage !== 'select' && (imagePreview || appLabel)
  // 流式期间禁止发送/输入，答完之后可对同一张截图继续问新问题（每次仍为独立 Q&A，自动入历史）
  const sendDisabled = streaming
  // 对话栏（输入框）只在 chat 模式显示；translate 模式只渲染浮动结果卡片
  const showBar = mode === 'chat'
  // translate 浮动卡片：截图后在选区旁出现，加载/完成两态
  const showTranslateCard = (mode === 'translate' || mode === 'translateText') && (stage === 'translating' || stage === 'translated')
  // 浮动布局仅用于截图翻译关闭全屏覆盖、或 translateText 文本翻译卡。
  // 普通 Lens 截图后固定保持全屏 overlay，只移动输入栏。
  // capturedFrame 只在最近一次截图后非空,而 restoreHistory 会清掉它(历史项的选区不再相关);
  // 但此时 lens 窗口仍是浮动小尺寸 → 必须叠加 floatingRebased 才能正确反映"窗口当前在浮动态"。
  const isFloatingLayout = mode === 'translateText' || (!keepFullscreen && (capturedFrame !== null || floatingRebased) && stage !== 'select')
  const stableAnswerHeight = isFloatingLayout
    ? fullscreenMetricsRef.current?.ANSWER_H || metrics.ANSWER_H
    : metrics.ANSWER_H
  const translateCardMaxHeight = mode === 'translateText' || !keepFullscreen
    ? READY_BAR_H + 8 + stableAnswerHeight
    : Math.min(viewport.h - 32, READY_BAR_H + 8 + stableAnswerHeight)
  const translateCardUsesFullscreenMotion = mode === 'translate' && keepFullscreen && !isFloatingLayout
  const translateCardTransitionProperty = barNoTransition || translateCardDragging
    ? 'none'
    : translateCardUsesFullscreenMotion
      ? 'transform, opacity'
      : 'left, top, width, transform, opacity'
  const translateCardTransform = translateCardUsesFullscreenMotion
    ? `translate3d(${flyDelta.x}px, ${flyDelta.y}px, 0) scale(${barIntro ? 1 : 0.92})`
    : barIntro ? 'scale(1)' : 'scale(0.92)'

  const handleTranslateCardDragStart = useCallback((e: ReactPointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return
    e.preventDefault()
    e.stopPropagation()

    if (isFloatingLayout) {
      void api.startDragging().catch(err => console.error('[lens-drag] startDragging failed:', err))
      return
    }

    translateCardDragRef.current = {
      pointerId: e.pointerId,
      startX: e.clientX,
      startY: e.clientY,
      startRect: barRect,
    }
    setTranslateCardDragging(true)
    setBarNoTransition(true)
    e.currentTarget.setPointerCapture(e.pointerId)
  }, [barRect, isFloatingLayout])

  const handleTranslateCardDragMove = useCallback((e: ReactPointerEvent<HTMLDivElement>) => {
    const drag = translateCardDragRef.current
    if (!drag || drag.pointerId !== e.pointerId) return
    e.preventDefault()
    e.stopPropagation()

    const nextX = drag.startRect.x + e.clientX - drag.startX
    const nextY = drag.startRect.y + e.clientY - drag.startY
    const maxX = Math.max(8, viewport.w - drag.startRect.width - 8)
    const maxY = Math.max(8, viewport.h - translateCardMaxHeight - 8)

    setBarRect({
      x: Math.round(clamp(nextX, 8, maxX)),
      y: Math.round(clamp(nextY, 8, maxY)),
      width: drag.startRect.width,
    })
  }, [translateCardMaxHeight, viewport.h, viewport.w])

  const handleTranslateCardDragEnd = useCallback((e: ReactPointerEvent<HTMLDivElement>) => {
    const drag = translateCardDragRef.current
    if (!drag || drag.pointerId !== e.pointerId) return
    e.preventDefault()
    e.stopPropagation()

    translateCardDragRef.current = null
    setTranslateCardDragging(false)
    setBarNoTransition(false)
    try {
      e.currentTarget.releasePointerCapture(e.pointerId)
    } catch {
      // Pointer capture may already be released by the platform.
    }
  }, [])

  const handleTranslateCardLostCapture = useCallback((e: ReactPointerEvent<HTMLDivElement>) => {
    const drag = translateCardDragRef.current
    if (!drag || drag.pointerId !== e.pointerId) return
    translateCardDragRef.current = null
    setTranslateCardDragging(false)
    setBarNoTransition(false)
  }, [])

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

  // 浮动模式下：截图翻译 / 文本翻译的 stage 或布局变化时动态调整窗口尺寸
  useEffect(() => {
    if (keepFullscreen && mode !== 'translateText') return
    if (stage === 'select') return
    if (!floatingRebased && mode !== 'translateText') return

    const x = barRect.x - FLOATING_PADDING
    const y = barRect.y - FLOATING_PADDING
    const w = barRect.width + FLOATING_PADDING * 2
    let h = READY_BAR_H + FLOATING_PADDING * 2

    if (stage === 'answering') {
      h += FLOATING_GAP + answerLayout.height
    }

    // translate 卡片预留空间
    if ((stage === 'translating' || stage === 'translated') && (mode === 'translate' || mode === 'translateText')) {
      h = Math.max(h, READY_BAR_H + FLOATING_GAP + stableAnswerHeight + FLOATING_PADDING * 2)
    }

    // quick action buttons below input bar in ready stage
    if (stage === 'ready' && quickActions.length > 0 && !keepFullscreen) {
      const qaH = Math.ceil(quickActions.length / 3) * 36 + 8
      h += qaH
    }

    // history 面板:浮动模式下面板渲染在 bar 下方(top: 100%+18 = bar bottom + 8),
    // 窗口必须扩到 bar bottom + 8 + 面板高度,否则面板被 OS 裁掉。
    // 全屏模式不需要扩,面板渲染在 bar 上方已有空间。
    if (isFloatingLayout && historyOpen && historyPanelH > 0) {
      h = Math.max(h, READY_BAR_H + FLOATING_GAP + historyPanelH + FLOATING_PADDING * 2)
    }

    // macOS 上窗口已经在 rebase 时搬到屏幕锚点,barRect 是窗口内坐标 (0, 0)。
    // 这里若再传 x/y 会把窗口搬到屏幕 (0, 0)。只传 width/height,让 OS 保持当前 origin。
    // Windows 走 SetWindowRgn 必须传 x/y 才能更新裁剪区。
    if (isMacPlatform) {
      api.lensSetFloating({ width: w, height: h }).catch(err => console.error('[lens-floating] resize failed:', err))
    } else {
      api.lensSetFloating({ x, y, width: w, height: h }).catch(err => console.error('[lens-floating] resize failed:', err))
    }
  }, [stage, answerLayout, barRect, floatingRebased, keepFullscreen, mode, stableAnswerHeight, historyOpen, historyPanelH, isFloatingLayout, quickActions])

  return (
    <div
      ref={rootRef}
      tabIndex={-1}
      className="fixed inset-0 select-none outline-none"
      onPointerEnter={requestWindowFocus}
      onPointerMove={requestWindowFocus}
      onPointerDownCapture={requestWindowFocus}
      onMouseDown={handleMouseDown}
      onMouseMove={handleMouseMove}
      onMouseUp={handleMouseUp}
      data-tauri-drag-region="false"
    >
      {stage === 'select' && freezeFramePreview && (
        <img
          src={freezeFramePreview}
          alt=""
          draggable={false}
          className="absolute inset-0 w-full h-full object-fill pointer-events-none"
        />
      )}

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

      {/* drawMode 关闭时也持续显示已落下的箭头 */}
      {capturedFrame && stage === 'ready' && keepFullscreen && !drawMode && arrows.length > 0 && (
        <svg
          className="absolute pointer-events-none"
          style={{
            left: capturedFrame.x,
            top: capturedFrame.y,
            width: capturedFrame.width,
            height: capturedFrame.height,
            overflow: 'visible',
            zIndex: 9,
          }}
          width={capturedFrame.width}
          height={capturedFrame.height}
        >
          {arrows.map((a, i) => (
            <ArrowSvg key={i} arrow={a} />
          ))}
        </svg>
      )}

      {/* drawMode:在 capturedFrame 矩形内画箭头.透明 div 收事件、SVG 渲染,
          不加 dim、不再贴 imagePreview 背景,直接显示原画面 */}
      {capturedFrame && stage === 'ready' && keepFullscreen && drawMode && (
        <div
          className="absolute"
          style={{
            left: capturedFrame.x,
            top: capturedFrame.y,
            width: capturedFrame.width,
            height: capturedFrame.height,
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
      )}

      {/* select-only：hover 高亮 / drag 选区 / 顶部 hint */}
      {stage === 'select' && (
        <>
          {showCaptureHint && (
            <div className="absolute top-[calc(env(safe-area-inset-top,0px)+36px)] left-0 right-0 flex justify-center pointer-events-none z-30">
              <div className="px-3 py-1.5 rounded-full bg-neutral-950/80 text-white text-[12px] font-medium shadow-[0_8px_24px_rgba(0,0,0,0.24)] ring-1 ring-white/10 backdrop-blur-md">
                {captureHintText}
              </div>
            </div>
          )}
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

      {/* 对话栏 + 答案区：始终渲染，输入栏移动只用 transform，位置/尺寸直接 snap。
          - select：底部居中 680，缩略图槽位用 sparkle 占位
          - ready：飞到选区附近 600，左侧切换为缩略图 + 应用名
          - answering：在对话栏下方 absolute 展开 answer 区（固定 360 高） */}
      {showBar && (
        <div
          ref={barRef}
          className="absolute ease-out"
          onMouseDown={(e) => { if (stage !== 'select') e.stopPropagation() }}
          onMouseMove={(e) => { if (stage !== 'select') e.stopPropagation() }}
          onMouseUp={(e) => { if (stage !== 'select') e.stopPropagation() }}
          onClick={(e) => { if (stage !== 'select') e.stopPropagation() }}
          style={{
            left: barRect.x,
            top: barRect.y,
            width: barRect.width,
            transitionProperty: barNoTransition ? 'none' : 'transform, opacity',
            transitionDuration: barNoTransition ? '0ms' : `${TRANSITION_MS}ms`,
            transitionTimingFunction: 'cubic-bezier(0.22, 1, 0.36, 1)',
            transform: `translate3d(${flyDelta.x}px, ${flyDelta.y}px, 0) scale(${barIntro ? 1 : 0.92})`,
            opacity: barIntro ? 1 : 0,
            willChange: 'transform, opacity',
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
              {stage === 'ready' && keepFullscreen && (
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
            {mode === 'chat' && (
              <button
                type="button"
                onClick={() => {
                  if (!webSearchAvailable || streaming) return
                  setWebSearchEnabled(v => !v)
                }}
                disabled={!webSearchAvailable || streaming}
                title={webSearchAvailable ? t.lensWebSearchToggle : t.lensWebSearchUnavailable}
                className={`shrink-0 w-9 h-9 rounded-lg flex items-center justify-center transition-colors ${
                  webSearchEnabled && webSearchAvailable
                    ? 'bg-blue-500 text-white hover:bg-blue-600'
                    : 'text-neutral-600 dark:text-neutral-300 hover:bg-black/[0.05] dark:hover:bg-white/[0.06]'
                } ${(!webSearchAvailable || streaming) ? 'opacity-40 cursor-not-allowed' : ''}`}
              >
                <Globe size={15} strokeWidth={1.9} />
              </button>
            )}
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
                  ref={historyContentRef}
                  className={`absolute right-0 w-[240px] rounded-xl bg-white dark:bg-neutral-900 shadow-[0_18px_44px_-12px_rgba(0,0,0,0.4)] ring-1 ring-black/[0.06] dark:ring-white/[0.08] overflow-hidden z-50 ${
                    isFloatingLayout ? '' : 'bottom-full mb-2'
                  }`}
                  style={isFloatingLayout
                    // 浮动模式下 lens 窗口只覆盖 bar 矩形,面板若按 bottom-full 渲染到 bar 上方会被 OS 裁掉。
                    // 改为渲染到 bar 下方:从 trigger 容器向下偏移 8 (gap) + 10 (bar 内 trigger 顶部的 padding) = 18 = bar bottom + 8。
                    ? { top: 'calc(100% + 18px)' }
                    : undefined}
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
                              {firstUserQ && (
                                <div className="text-[11.5px] truncate leading-tight text-neutral-800 dark:text-neutral-200">
                                  {firstUserQ}
                                </div>
                              )}
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
            <button
              type="button"
              onClick={(e) => { e.stopPropagation(); api.lensClose() }}
              className="shrink-0 w-7 h-7 rounded-lg flex items-center justify-center
                text-neutral-400 dark:text-neutral-500 hover:text-neutral-600 dark:hover:text-neutral-300
                hover:bg-black/[0.06] dark:hover:bg-white/[0.08]
                transition-colors duration-150 ml-0.5"
              title="Esc"
            >
              <X size={15} strokeWidth={2} />
            </button>
          </div>

          {/* quick action buttons */}
          {stage === 'ready' && quickActions.length > 0 && imageIdRef.current && (
            <div className="mt-2 flex flex-wrap justify-center gap-1.5 px-2">
              {quickActions.map((qa) => (
                <button
                  key={qa.id}
                  type="button"
                  onClick={() => void handleQuickAction(qa)}
                  disabled={streaming}
                  className="px-3 py-1.5 rounded-full text-[12px] font-medium
                    bg-white/90 dark:bg-neutral-800/90 text-neutral-700 dark:text-neutral-200
                    hover:bg-[#D97757]/15 dark:hover:bg-[#D97757]/20
                    ring-1 ring-black/[0.06] dark:ring-white/[0.08]
                    hover:ring-[#D97757]/30 dark:hover:ring-[#D97757]/30
                    transition-all duration-150 backdrop-blur-sm
                    disabled:opacity-40 disabled:cursor-not-allowed
                    shadow-[0_2px_8px_-2px_rgba(0,0,0,0.12)]"
                >
                  {qa.label}
                </button>
              ))}
            </div>
          )}

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
                  <button
                    onClick={() => setSourceMode(v => !v)}
                    title={sourceMode ? t.lensRenderMode : t.lensSourceMode}
                    className="flex items-center gap-1 px-2 py-0.5 text-[10px] text-neutral-500 hover:text-neutral-800 dark:text-neutral-400 dark:hover:text-neutral-100 rounded hover:bg-black/5 dark:hover:bg-white/10 transition-colors"
                  >
                    {sourceMode ? <Eye size={11} /> : <Code size={11} />}
                    <span>{sourceMode ? t.lensRenderMode : t.lensSourceMode}</span>
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
              <div
                ref={chatScrollRef}
                className="h-full overflow-y-auto custom-scrollbar px-3.5 pt-3"
                style={{ paddingBottom: answerLayout.placeAbove ? 12 : 96 }}
              >
                {/* desc 模式下操作按钮放最前（贴最新答案） */}
                {messageOrder === 'desc' && showActions && Actions}
                {ordered.map((m, displayIdx) => {
                  const origIdx = messageOrder === 'desc' ? messages.length - 1 - displayIdx : displayIdx
                  const isUser = m.role === 'user'
                  if (isUser && !m.content.trim()) return null
                  const isLast = origIdx === lastChronoIdx
                  const webSearch = m.webSearch
                  const searchInProgress = webSearch?.status === 'searching'
                  const showWebSearch = Boolean(webSearch && (
                    webSearch.status !== 'skipped' ||
                    Boolean(webSearch.error) ||
                    Boolean(webSearch.results?.length)
                  ))
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
                          {showWebSearch && webSearch && (
                            <WebSearchBlock
                              search={webSearch}
                              labels={{
                                searching: t.lensWebSearchSearching,
                                results: t.lensWebSearchResults,
                                citations: t.lensWebSearchCitations,
                                noResults: t.lensWebSearchNoResults,
                                error: t.lensWebSearchError,
                                skipped: t.lensWebSearchSkipped,
                              }}
                              onOpen={(url) => void api.openExternal(url).catch(err => console.error(err))}
                            />
                          )}
                          {m.content ? (
                            sourceMode ? (
                              <pre className="not-prose whitespace-pre-wrap break-words text-[12.5px] leading-6 font-mono bg-neutral-100 dark:bg-neutral-800/60 rounded-lg p-3">
                                {m.content}
                              </pre>
                            ) : (
                              <ReactMarkdown remarkPlugins={[remarkMath]} rehypePlugins={[rehypeKatex]}>
                                {m.content}
                              </ReactMarkdown>
                            )
                          ) : isLast && streaming && !m.reasoning && !searchInProgress ? (
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
            maxHeight: translateCardMaxHeight,
            transitionProperty: translateCardTransitionProperty,
            transitionDuration: barNoTransition || translateCardDragging ? '0ms' : `${TRANSITION_MS}ms`,
            transitionTimingFunction: 'cubic-bezier(0.22, 1, 0.36, 1)',
            transform: translateCardTransform,
            opacity: barIntro ? 1 : 0,
          }}
          data-tauri-drag-region="false"
        >
          {/* 顶部缩略图 + 应用名 + 状态徽章（耗时 / token 估算） */}
          <div
            className="flex items-center gap-2.5 px-3.5 py-2.5 border-b border-black/[0.05] dark:border-white/[0.06] cursor-move select-none"
            onPointerDown={handleTranslateCardDragStart}
            onPointerMove={handleTranslateCardDragMove}
            onPointerUp={handleTranslateCardDragEnd}
            onPointerCancel={handleTranslateCardDragEnd}
            onLostPointerCapture={handleTranslateCardLostCapture}
          >
            {mode !== 'translateText' && (
              <div className="shrink-0 w-8 h-8 rounded-lg overflow-hidden ring-1 ring-black/[0.06] dark:ring-white/[0.06] bg-neutral-100 dark:bg-neutral-800 flex items-center justify-center">
                {imagePreview ? (
                  <img src={imagePreview} alt="snap" className="w-full h-full object-cover" />
                ) : (
                  <ImageIcon size={12} className="text-neutral-400" />
                )}
              </div>
            )}
            <span className="text-[12.5px] font-medium text-neutral-700 dark:text-neutral-300 truncate flex-1">
              {mode === 'translateText' ? t.selectedText : (appLabel || t.lensScreenshotOf.replace('：', '').replace(':', ''))}
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
              maxHeight: mode === 'translateText' || !keepFullscreen
                ? stableAnswerHeight
                : Math.min(viewport.h - 110, stableAnswerHeight)
            }}>
            {translateError ? (
              translateError === 'rapidocr_models_missing' ? (
                <div className="text-[12.5px] text-amber-700 dark:text-amber-300 leading-6 whitespace-pre-wrap break-words">
                  {t.rapidOcrModelsMissing}
                </div>
              ) : (
                <div className="text-[12.5px] text-red-500 leading-6 whitespace-pre-wrap break-words">
                  {t.lensError}: {translateError}
                </div>
              )
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
                {translateOriginal && mode !== 'translateText' && (
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
