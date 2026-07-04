import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'
import { flushSync } from 'react-dom'
import { Loader2, Copy, Check, Square, Image as ImageIcon, ArrowUp, History as HistoryIcon, ChevronDown, MousePointer2, Code, Eye, MessageSquarePlus } from 'lucide-react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { api, type LensStreamPayload, type LensTranslateStreamPayload, type LensReplaceLine, type LensReplaceStreamPayload, type LensWindowInfo, type ExplainMessage, type LensWebSearchPayload } from './api/tauri'
import { ChatMarkdown } from './chat/ChatMarkdown'
import { i18n, normalizeLang, type Lang } from './settings/i18n'
import { copyToClipboard } from './utils/clipboard'

import type { Arrow, BarRect, CapturedFrame, HistoryItem, Metrics, Mode, Point, Stage, TranslateCardDrag } from './lens/types'
import { ArrowSvg } from './lens/ArrowSvg'
import { ReplaceTranslateOverlay } from './lens/ReplaceTranslateOverlay'
import { ARROW_MIN_DRAG_PX, composeAnnotatedImage } from './lens/annotation'
import { HISTORY_MAX, HISTORY_THUMB_SIZE, loadHistoryFromStorage, makeThumbnail, saveHistoryToStorage } from './lens/history'
import { ANCHOR_GAP, DRAG_THRESHOLD, FLOATING_GAP, FLOATING_PADDING, READY_BAR_H, SELECT_REVEAL_DELAY_MS, TRANSITION_MS, clamp, computeChatBarWidth, computeMetrics, computeSelectBar, isMacPlatform } from './lens/layout'
import { estimateTokens, formatTokens } from './utils/tokens'
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
  if (mode === 'replace') return 'replace'
  return 'chat'
}

function keepFullscreenForMode(curMode: Mode, screenshotKeepFullscreen: boolean): boolean {
  return curMode === 'chat'
    || curMode === 'replace'
    || (curMode === 'translate' && screenshotKeepFullscreen)
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

const LENS_HIDE_IDLE_TIMEOUT_MS = 120

const waitForVisibleIdle = (timeout = LENS_HIDE_IDLE_TIMEOUT_MS) => new Promise<void>((resolve) => {
  const idleWindow = window as Window & {
    requestIdleCallback?: (cb: () => void, options?: { timeout?: number }) => number
  }
  if (idleWindow.requestIdleCallback) {
    idleWindow.requestIdleCallback(() => resolve(), { timeout })
    return
  }
  window.setTimeout(resolve, timeout)
})

/**
 * Lens 模式：單 webview 三態機，統一 DOM。
 * - select：webview 全螢幕 + 灰幕 + hover 應用視窗高亮 + 區域 drag + 底部對話欄（純文字直髮）
 * - ready：截圖後對話欄 CSS transition 飛到選區附近，加縮圖，輸入聚焦
 * - answering：對話欄下方展開 answer 區（透明背景，對話欄不動）
 *
 * 關鍵：webview 始終全螢幕，整個過渡靠 CSS。後端 lens_resolve_anchor 僅算目標座標，不縮視窗。
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
  // Lens 啟動前 Rust 端抓到的選中文字：作為本次會話的上下文字首
  // 僅在首輪 chat 訊息傳送時拼接進 prompt；徽章靜態顯示行數；次輪不再注入。
  const [selectionText, setSelectionText] = useState('')
  const [messages, setMessages] = useState<ExplainMessage[]>([])
  const [streaming, setStreaming] = useState(false)
  const [copied, setCopied] = useState(false)
  const [lang, setLang] = useState<Lang>('zh-TW')
  const [messageOrder, setMessageOrder] = useState<'asc' | 'desc'>('asc')
  const [webSearchAvailable, setWebSearchAvailable] = useState(false)
  const [webSearchEnabled, setWebSearchEnabled] = useState(false)
  const [keepFullscreen, setKeepFullscreen] = useState(() => readModeFromHash() !== 'translateText')
  const [floatingRebased, setFloatingRebased] = useState(false)
  const [mode, setMode] = useState<Mode>(() => readModeFromHash())
  const [surfaceDormant, setSurfaceDormant] = useState(false)
  // translate 模式專用：OCR 原文 + 翻譯結果 + 計時
  const [translateOriginal, setTranslateOriginal] = useState('')
  const [translateText, setTranslateText] = useState('')
  const [translateError, setTranslateError] = useState('')
  const [replaceLines, setReplaceLines] = useState<LensReplaceLine[]>([])
  const [replacePhase, setReplacePhase] = useState<'ocr' | 'translating' | 'done' | ''>('')
  const [replaceError, setReplaceError] = useState('')
  const [translateDurationMs, setTranslateDurationMs] = useState<number | null>(null)
  const [translateNow, setTranslateNow] = useState(() => Date.now())
  const translateStartRef = useRef<number | null>(null)
  const [freezeFrameImageId, setFreezeFrameImageId] = useState('')
  const [freezeFramePreview, setFreezeFramePreview] = useState('')
  // 凍結幀用 canvas 渲染：backing store 取圖片原生解析度，保證全螢幕幀在屏上按裝置畫素 1:1
  // 柵格化（繞過透明 overlay 下 WebView2 把全螢幕 <img> 以低光柵倍率放大導致的發虛）。
  const freezeCanvasRef = useRef<HTMLCanvasElement>(null)
  // viewport 大小：監聽 resize（拔顯示器/系統縮放變化都會觸發），所有相對尺寸由此重算
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
  // barIntro：select 態首次顯示時給對話欄加一次 scale-up 進入動畫；之後切換都靠 transition
  const [barIntro, setBarIntro] = useState(false)
  // barNoTransition：reset/drag/視窗裁剪下換時臨時停用 transition，避免上次動畫在 hide 後續播。
  const [barNoTransition, setBarNoTransition] = useState(true)
  // flyDelta：全螢幕覆蓋模式下 fly 動畫用 transform translate 取代 left/top 過渡。
  // left/top 不是 GPU 合成屬性，每幀都要走 layout/reflow；Windows 上 webview hide→show 後
  // 合成器剛被喚醒、首個大幅 left/top 過渡極易卡頓（"亂跳"）。改為：left/top 立即 snap 到
  // 最終位置，用 transform: translate(dx, dy) 把視覺位置拉回起點，下一幀再把 delta 過渡到 (0,0)。
  // transform 走合成層，不阻塞主執行緒，多視窗會話間穩定。
  const [flyDelta, setFlyDelta] = useState<{ x: number; y: number }>({ x: 0, y: 0 })
  const [translateCardDragging, setTranslateCardDragging] = useState(false)
  // capturedFrame：保留最後一次截圖選區/視窗的高亮框，作為"已截圖"視覺標記，ready/answering 態繼續顯示
  const [capturedFrame, setCapturedFrame] = useState<CapturedFrame | null>(null)
  const [showCaptureHint, setShowCaptureHint] = useState(false)
  // 箭頭標註:僅 stage==='ready' 子模式
  // arrows / draftArrow 座標系 = capturedFrame 邏輯畫素 (左上角為原點)
  const [drawMode, setDrawMode] = useState(false)
  const [arrows, setArrows] = useState<Arrow[]>([])
  // 原始碼/渲染切換：false=渲染模式(ChatMarkdown)，true=原始碼模式(原始文字)
  const [sourceMode, setSourceMode] = useState(false)
  const [draftArrow, setDraftArrow] = useState<Arrow | null>(null)
  // 任何 stage 切換時強制清掉 draw 子模式 + 已落箭頭
  useEffect(() => {
    if (stage !== 'ready') {
      setDrawMode(false)
      setArrows([])
      setDraftArrow(null)
    }
  }, [stage])
  // 凍結幀繪製：把 data URL 畫進 canvas，backing store = 圖片原生畫素，CSS 鋪滿 viewport，
  // 使全螢幕凍結幀按裝置畫素 1:1 顯示，與即時桌面同等清晰（避免 <img> 被重取樣發虛）。
  // 全螢幕態（select 及 keepFullscreen 的 ready/answering）整段會話都保留作背景，直到關閉 Lens。
  useEffect(() => {
    if (!freezeFramePreview) return
    if (stage !== 'select' && !keepFullscreen) return
    const canvas = freezeCanvasRef.current
    if (!canvas) return
    let cancelled = false
    const img = new Image()
    img.onload = () => {
      if (cancelled) return
      if (canvas.width !== img.naturalWidth) canvas.width = img.naturalWidth
      if (canvas.height !== img.naturalHeight) canvas.height = img.naturalHeight
      const ctx = canvas.getContext('2d')
      if (!ctx) return
      ctx.clearRect(0, 0, canvas.width, canvas.height)
      ctx.drawImage(img, 0, 0)
    }
    img.src = freezeFramePreview
    return () => {
      cancelled = true
    }
  }, [stage, keepFullscreen, freezeFramePreview])
  // 記憶體歷史：單次 app 生命週期保留，esc/hide 不清空
  const [history, setHistory] = useState<HistoryItem[]>(loadHistoryFromStorage)
  const [historyOpen, setHistoryOpen] = useState(false)
  const [historyPanelH, setHistoryPanelH] = useState(0)

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
  // 只在真正"開啟/重入 Lens 會話"（enterSelect）時自增，與動畫用的 motionSeqRef 區分開。
  // closeAfterReset 用它判斷"等待隱藏期間是否有新會話開啟"，避免被關閉自身的
  // setStage('select') 副作用（會再次 bump motionSeqRef）誤判而跳過 lensClose。
  const lensOpenSeqRef = useRef(0)
  const selectRevealTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const selectRevealedRef = useRef(false)
  const captureHintEnabledRef = useRef(true)
  const sendToChatRef = useRef(true)
  const screenshotKeepFullscreenRef = useRef(true)
  // 快速翻譯結果卡寬度（截圖翻譯 + 選中文字翻譯共用，來自設定，預設 480）
  const cardWidthRef = useRef(480)
  const prevStreamingRef = useRef(false)
  const preparingSendRef = useRef(false)
  const answerFinishedRef = useRef(false)
  const lastLensStreamEventRef = useRef('')
  // Stream 真實結束（成功 / 錯誤 / 使用者主動取消）後才置 true，
  // 讓歷史持久化 effect 只在這一次 rerun 觸發 push；restoreHistory / enterSelect / resetBeforeHide 防禦性清零，
  // 避免恢復歷史時 setMessages 觸發 effect 把恢復的對話又當新條目寫一遍歷史。
  const justFinishedStreamRef = useRef(false)
  // capture 期間 macOS screencapture 可能短暫讓 lens webview 失焦 → 觸發 blur 誤關閉。
  // 這個 ref 標記"截圖進行中"，blur handler 看到就跳過。
  const capturingRef = useRef(false)
  // selectionText 非同步 take 的重入 token：每次 enterSelect / resetBeforeHide / restoreHistory 都 +1，
  // 老請求看到 myReq !== current 直接丟棄，避免 take 完成時已經進入新會話被錯誤注入。
  const selectionReqIdRef = useRef(0)
  const translateCardDragRef = useRef<TranslateCardDrag | null>(null)
  // 答案區滾動容器，stream 時自動滾到底部
  const chatScrollRef = useRef<HTMLDivElement>(null)
  // 浮動模式下儲存截圖時的全螢幕 metrics，避免視窗縮小後 answerLayout 被壓縮得太小
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
    // 必須在 setStreaming(false) 前置 true：歷史持久化 effect 依賴 streaming 變化觸發。
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

  // 選中文字行數：translate 模式不計；空 / 僅空白 → 0（驅動徽章是否顯示）
  const selectionLineCount = useMemo(() => {
    if (mode !== 'chat') return 0
    if (!selectionText.trim()) return 0
    return selectionText.split(/\r?\n/).length
  }, [selectionText, mode])

  const loadLensSettings = useCallback(async (curMode: Mode = readModeFromHash()) => {
    try {
      const settings = await api.getSettings()
      setLang(normalizeLang(settings.settingsLanguage))
      setMessageOrder(settings.lens?.messageOrder === 'desc' ? 'desc' : 'asc')
      const webSearch = settings.lens?.webSearch
      const hasWebSearchKey = webSearch?.provider === 'exa'
        ? !!webSearch.exaApiKey?.trim()
        : !!webSearch?.tavilyApiKey?.trim()
      const canUseWebSearch = webSearch?.enabled === true && hasWebSearchKey
      setWebSearchAvailable(canUseWebSearch)
      setWebSearchEnabled(canUseWebSearch && curMode === 'chat')
      screenshotKeepFullscreenRef.current = settings.screenshotTranslation?.keepFullscreenAfterCapture !== false
      cardWidthRef.current = settings.screenshotTranslation?.cardWidth ?? 480
      setKeepFullscreen(keepFullscreenForMode(curMode, screenshotKeepFullscreenRef.current))
      captureHintEnabledRef.current = settings.lens?.showCaptureHint !== false
      sendToChatRef.current = settings.lens?.sendToChat !== false
    } catch (err) { console.error('Failed to load settings', err) }
  }, [])

  // 載入設定：普通 Lens 截圖後固定保持全螢幕覆蓋；截圖翻譯仍讀自己的保留全螢幕配置。
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
      // 複用視窗時原生 first responder 可能沒落到內部 WKWebView，導致"第二次開啟"要手點一下才聚焦。
      // 這裡讓原生 makeKeyWindow + makeFirstResponder(WKWebView)——非啟用方式，**不調
      // getCurrentWindow().setFocus()**：tao 的 set_focus 會 `[NSApp activateIgnoringOtherApps:YES]`
      // 啟用整個 app，從而把別屏上的 Chat 主視窗拽到前臺造成跳屏。非啟用 panel 只需 makeKeyWindow
      // 即可拿到鍵盤，無需啟用 app。本函式本就帶多次重試([0,40,120,240,420])磨平復用聚焦不穩定。
      try {
        await api.lensFocusWebview()
      } catch {
        // ignore：非 macOS no-op，或視窗正在關閉
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

  // select 態進入：重新整理所有 state、重算對話欄位置、播放 intro 動畫
  const enterSelect = useCallback(async (resetPayload: LensResetPayload = {}) => {
    const curMode = readModeFromHash()
    await loadLensSettings(curMode)
    const resetFrame = resetPayload.frame
    const resetFreezeFrameImageId = resetPayload.freezeFrameImageId ?? ''
    cancelPendingMotion()
    // 標記一次真正的會話開啟/重入：pending 的 closeAfterReset 看到它變化即放棄隱藏。
    lensOpenSeqRef.current++
    const motionSeq = motionSeqRef.current
    fullscreenMetricsRef.current = null
    // 防禦：reset 流程會 setMessages([]) + setStreaming(false)，理論上 messages.length===0 effect 不會進
    // 持久化分支，但顯式清零更穩
    justFinishedStreamRef.current = false
    // 用 flushSync 同步提交所有 reset 後的狀態：webview show 之前 DOM 必須已經反映新位置，
    // 否則 Rust 的 show() 會先把舊 frame 露出來。
    // barNoTransition 同 frame 一起置 true → bar 從老座標 snap 到 select 座標，不回放動畫。
    flushSync(() => {
      setBarNoTransition(true)
      setSurfaceDormant(false)
      setStage(curMode === 'translateText' ? 'translating' : 'select')
      setMode(curMode)
      setKeepFullscreen(keepFullscreenForMode(curMode, screenshotKeepFullscreenRef.current))
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
      setReplaceLines([])
      setReplacePhase('')
      setReplaceError('')
      setFreezeFrameImageId(resetFreezeFrameImageId)
      setFreezeFramePreview('')
      const w = resetFrame?.width ?? window.innerWidth
      const h = resetFrame?.height ?? window.innerHeight
      setViewport({ w, h })
      const m = computeMetrics(w, h)
      setBarRect(curMode === 'translateText'
        ? { x: FLOATING_PADDING, y: FLOATING_PADDING, width: Math.min(cardWidthRef.current, w) }
        : computeSelectBar(w, h, m))
      setFlyDelta({ x: 0, y: 0 })
      setCapturedFrame(null)
      // 重置 intro：先關再開，下一幀讓 transition 從 scale-90 到 scale-100
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
    // 重新載入設定：使用者在設定面板修改後關閉再開啟 Lens，需要讀到最新值。
    // 必須放在 reset DOM 之後，避免 await 期間 Rust 已 show 導致舊 ready/answering surface 露出首幀。
    void (async () => {
      try {
        const settings = await api.getSettings()
        if (motionSeq !== motionSeqRef.current) return
        screenshotKeepFullscreenRef.current = settings.screenshotTranslation?.keepFullscreenAfterCapture !== false
      cardWidthRef.current = settings.screenshotTranslation?.cardWidth ?? 480
        setKeepFullscreen(keepFullscreenForMode(curMode, screenshotKeepFullscreenRef.current))
        captureHintEnabledRef.current = settings.lens?.showCaptureHint !== false
        if (stageRef.current === 'select' && selectRevealedRef.current) {
          setShowCaptureHint(captureHintEnabledRef.current)
        }
      } catch (err) { console.error('Failed to reload settings', err) }
    })()
    // 非同步 take 走 Rust 端在 lens_request_internal 中暫存的選中文字。
    // token 防禦：take 期間使用者再開一次 Lens / 關閉，老 promise 落地時 myReq 已過期，丟棄。
    // 僅 chat 模式注入；> 200KB 直接丟棄避免上下文爆炸；trim 後非空才 setSelectionText。
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
      setReplaceLines([])
      setReplacePhase('')
      setReplaceError('')
          setTranslateDurationMs(null)
          translateStartRef.current = Date.now()
          setTranslateNow(Date.now())
          try {
            if (myReq !== selectionReqIdRef.current || motionSeq !== motionSeqRef.current) return
            const result = await api.lensTranslateText(text, requestId)
            if (myReq !== selectionReqIdRef.current || motionSeq !== motionSeqRef.current) return
            if (!result.success) {
              setTranslateError(result.error || 'Failed')
              if (translateStartRef.current !== null) {
                setTranslateDurationMs(Date.now() - translateStartRef.current)
                translateStartRef.current = null
              }
              setStage('translated')
            }
          } catch (err) {
            if (myReq !== selectionReqIdRef.current || motionSeq !== motionSeqRef.current) return
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
          // 第二個 raf 同時恢復 transitions 並觸發 intro：現在 bar 已經在 select 位置，
          // 只對 transform/opacity 做縮放進入動畫，不會回放歷史 left/top 過渡。
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
    // 冷掛載 與 複用收到 lens:reset 走同一路徑：主動 take 後端暫存的復位載荷（frame +
    // freezeFrameImageId）再 enterSelect。後端保證每次開啟只會觸發其中一個（冷建立→掛載；
    // 複用→事件），所以每次開啟只跑一次 enterSelect，不會雙跑把 take-once 的選區吞掉。
    const consumeAndEnter = async () => {
      let payload: LensResetPayload = {}
      try {
        const raw = await api.lensTakeResetPayload()
        if (raw) payload = readLensResetPayload(JSON.parse(raw))
      } catch (err) {
        console.error('[lens] take reset payload failed', err)
      }
      void enterSelect(payload)
    }
    void consumeAndEnter()
    const handleReset = () => {
      void consumeAndEnter()
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

  // viewport resize（拔顯示器 / 切解析度 / DPI 變更，以及浮動模式下 raf 同步動畫的逐幀縮放）
  // 都觸發 'resize' 事件 → 更新 viewport state，讓相對尺寸 metrics 重算。
  // 注意：浮動模式 rebase 已經在 flyBarToAnchor 裡通過同步動畫完成，不再在 resize handler 裡搶佔 barRect。
  useEffect(() => {
    const onResize = () => {
      setViewport({ w: window.innerWidth, h: window.innerHeight })
    }
    window.addEventListener('resize', onResize)
    return () => window.removeEventListener('resize', onResize)
  }, [])

  // viewport 或 metrics 變化時，select 態重算底部 bar 位置（ready/answering 態保持當前飛入位置不動，避免對話中閃跳）
  useEffect(() => {
    if (stageRef.current === 'select') {
      setBarRect(computeSelectBar(viewport.w, viewport.h, metrics))
    } else if (modeRef.current === 'translateText' && stageRef.current === 'translating') {
      const w = Math.min(cardWidthRef.current, viewport.w)
      setBarRect({ x: FLOATING_PADDING, y: FLOATING_PADDING, width: w })
    }
  }, [viewport, metrics])

  // 流式結束（streaming → false 且有任意 assistant 回答）時把當前會話推入歷史。
  // 按 imageId 去重：同一張截圖多輪對話作為單條歷史持續更新到最前。
  // translate 模式不入對話歷史（OCR+翻譯是一次性任務，無對話語義）。
  // 縮圖壓縮到 96x96 jpeg 再寫歷史，避免 localStorage 被幾 MB 的 base64 撐爆。
  useEffect(() => {
    // 只在真實"流剛結束"路徑觸發：handleSend / handleStop 的 finally 會先置 ref 再 setStreaming(false)。
    // restoreHistory / enterSelect / resetBeforeHide 呼叫前會顯式清零 ref，避免恢復歷史時 effect 誤觸發。
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

  // history 任意變化：1) 同步 localStorage  2) 偵測淘汰並刪除磁碟上對應的 PNG
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

  // 監聽 lens-stream 事件：把 reasoning_delta / delta 累積到最後一條 assistant 訊息
  // StrictMode 雙掛載下 listen 是 async：cleanup 時 unlisten 可能還沒賦值，需要 cancelled 旗標
  // 讓 promise resolve 時立即 dispose，否則會留下"幽靈 listener"導致每個事件觸發 N 次（字元重複）
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

  // messages 變化時自動滾動：正序滾到底（看新內容），倒序滾到頂（最新在頂）
  useEffect(() => {
    const el = chatScrollRef.current
    if (!el) return
    if (messageOrder === 'desc') el.scrollTop = 0
    else el.scrollTop = el.scrollHeight
  }, [messages, messageOrder])

  // Windows WebView2 在 input disabled/read-write 切換後容易丟 caret；回答結束後顯式還焦點。
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

  // 關閉前同步重置 state，讓 webview surface 在 hide 之前已經是空 select 態。
  // 否則下次 show 時 macOS 會先顯示上次的 ready 態 surface 一幀，再被 lens:reset 覆蓋 → 閃一下上次內容。
  // barNoTransition：停用 transition，避免 380ms 動畫被 hide 暫停後下次 show 續播。
  const releaseFreezeCanvas = useCallback(() => {
    const canvas = freezeCanvasRef.current
    if (!canvas) return
    const ctx = canvas.getContext('2d')
    ctx?.clearRect(0, 0, canvas.width, canvas.height)
    canvas.width = 0
    canvas.height = 0
  }, [])

  const resetBeforeHide = useCallback(() => {
    cancelPendingMotion()
    releaseFreezeCanvas()
    fullscreenMetricsRef.current = null
    translateStartRef.current = null
    // 防禦：和 enterSelect 同理 —— reset 路徑不該走持久化
    justFinishedStreamRef.current = false
    flushSync(() => {
      setBarNoTransition(true)
      setSurfaceDormant(true)
      setStage('select')
      setWindows([])
      setFloatingRebased(false)
      setHovered(null)
      setDragStart(null)
      setDragCurrent(null)
      setDragging(false)
      setTranslateCardDragging(false)
      setImagePreview('')
      setFreezeFrameImageId('')
      setFreezeFramePreview('')
      setAppLabel('')
      setInput('')
      setSelectionText('')
      setMessages([])
      setStreaming(false)
      setCopied(false)
      setTranslateOriginal('')
      setTranslateText('')
      setTranslateError('')
      setReplaceLines([])
      setReplacePhase('')
      setReplaceError('')
      setTranslateDurationMs(null)
      setBarRect(computeSelectBar(viewport.w, viewport.h, metrics))
      setFlyDelta({ x: 0, y: 0 })
      setCapturedFrame(null)
      setDrawMode(false)
      setArrows([])
      setDraftArrow(null)
      setSourceMode(false)
      setHistoryOpen(false)
      setHistoryPanelH(0)
      setBarIntro(false)
      setShowCaptureHint(false)
    })
    selectRevealedRef.current = false
    imageIdRef.current = ''
    translateCardDragRef.current = null
    // 讓任何還沒落地的 takeLensSelection 老 promise 作廢，避免關閉後 setSelectionText 拖回來
    selectionReqIdRef.current++
    focusReqIdRef.current++
  }, [cancelPendingMotion, releaseFreezeCanvas, viewport, metrics])

  const closeAfterReset = useCallback(async () => {
    // 記下關閉開始時的"會話代次"。resetBeforeHide 會 setStage('select') 進而觸發動畫
    // effect 再次 bump motionSeqRef，所以不能用 motionSeqRef 當守衛（會被自身副作用誤判）。
    // 只有 enterSelect（真正的新會話開啟/重入）才會改 lensOpenSeqRef——這才是該放棄隱藏的訊號。
    const closeOpenSeq = lensOpenSeqRef.current
    try { await api.lensCancelStream() } catch (err) { console.error(err) }
    resetBeforeHide()
    await waitForFrames(2)
    await waitForVisibleIdle()
    if (closeOpenSeq !== lensOpenSeqRef.current) return
    try { await api.lensClose() } catch (err) { console.error(err) }
  }, [resetBeforeHide])

  // 全域 Esc：流式時取消流 / 否則關閉
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

  // drawMode 鍵盤:Cmd+Z 撤銷最後一支箭頭,Esc 退出 drawMode(arrows 保留)
  useEffect(() => {
    if (!drawMode) return
    const onKey = (e: KeyboardEvent) => {
      // 輸入框聚焦時不攔截,讓使用者繼續打字
      const target = e.target as HTMLElement | null
      const isInput = target?.tagName === 'INPUT' || target?.tagName === 'TEXTAREA'

      // Esc:無論焦點在哪都退出 drawMode,並阻止全域 Esc 關掉 Lens
      // (輸入欄 autoFocus 時 isInput=true,但 Esc 在輸入框裡沒有合法語義,直接接管)
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

  // select 態切到其他應用 → 自動收起灰幕。
  // 注意：截圖過程中 screencapture 可能讓 lens 短暫失焦，capturingRef 防止誤關。
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

  /** webview client 座標 → 全域邏輯座標（與 CGWindow bounds 同坐標系） */
  const clientToGlobal = (p: Point): Point => ({
    x: winOrigin.x + p.x,
    y: winOrigin.y + p.y,
  })

  /** 命中偵測：找第一個包含該全域座標的應用視窗 */
  const hitTest = (gp: Point): LensWindowInfo | null => {
    for (const w of windows) {
      if (gp.x >= w.x && gp.x < w.x + w.width && gp.y >= w.y && gp.y < w.y + w.height) {
        return w
      }
    }
    return null
  }

  // 拖動選區矩形（webview 內座標）
  const dragRect = useMemo(() => {
    if (!dragStart || !dragCurrent) return null
    const x = Math.min(dragStart.x, dragCurrent.x)
    const y = Math.min(dragStart.y, dragCurrent.y)
    const w = Math.abs(dragCurrent.x - dragStart.x)
    const h = Math.abs(dragCurrent.y - dragStart.y)
    return { x, y, width: w, height: h }
  }, [dragStart, dragCurrent])

  // hover 高亮區（webview 內座標）
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
    // 點選在對話欄內部時不開始拖動，讓輸入框/按鈕等正常互動
    if (barRef.current?.contains(e.target as Node)) return
    // 歷史面板展開時點選外層只關閉面板，不開始拖動/截圖
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
    // 滑鼠在對話欄（含歷史面板）上方時清除 hover，避免高亮/誤截圖背後視窗
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

  /** 截圖後在前端直接算 bar 位置，讓對話欄飛到選區左/右側。
   *  優先右側，右側空間不夠再放左側；都不夠時貼大空間一側。 */
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
    const barW = mode === 'chat' ? computeChatBarWidth(metrics) : Math.min(cardWidthRef.current, vw - FLOATING_PADDING * 2)
    const ANSWER_H = metrics.ANSWER_H

    const rightStart = ax + anchorW + ANCHOR_GAP
    const spaceRight = vw - rightStart - 16
    const spaceLeft = ax - ANCHOR_GAP - 16

    let targetX: number
    if (spaceRight >= barW) {
      targetX = rightStart
    } else if (spaceLeft >= barW) {
      targetX = ax - barW - ANCHOR_GAP
    } else {
      // 左右都放不下完整 bar：貼空間更大的一側螢幕邊
      targetX = spaceRight >= spaceLeft ? vw - barW - 16 : 16
    }

    // 垂直：與選區中心對齊；總高度需容納 bar + 8 + answer 區
    const totalH = READY_BAR_H + 8 + ANSWER_H
    let targetY = ay + anchorH / 2 - READY_BAR_H / 2
    if (targetY + totalH > vh - 16) targetY = vh - totalH - 16
    if (targetY < 16) targetY = 16

    if (targetX < 16) targetX = 16
    if (targetX + barW > vw - 16) targetX = vw - barW - 16

    // translate 模式截完直接進 translating；chat 模式進 ready 等使用者提問
    const targetStage: Stage = (mode === 'translate' || mode === 'replace') ? 'translating' : 'ready'

    if (!keepFullscreen) {
      fullscreenMetricsRef.current = metrics
      const finalX = Math.round(targetX)
      const finalY = Math.round(targetY)
      const startX = barRect.x
      const startY = barRect.y
      const floatW = barW + FLOATING_PADDING * 2
      const floatH = targetStage === 'ready'
        ? READY_BAR_H + FLOATING_PADDING * 2
        : READY_BAR_H + FLOATING_GAP + metrics.ANSWER_H + FLOATING_PADDING * 2
      const isTranslateMode = mode === 'translate'

      if (isMacPlatform) {
        // macOS:走 AppKit 原生 NSAnimationContext + animator setFrame:。
        // 一次 IPC 觸發,Core Animation 在合成器執行緒按顯示器原生重新整理率插值,
        // 不再有 JS rAF 每幀打 IPC + 兩次獨立 AppKit 呼叫導致的 coalescing 掉幀。
        // 時間曲線 cubic-bezier(0.22, 1, 0.36, 1) 與原 CSS transition / rAF 完全一致。
        const floatX = winOrigin.x + finalX - FLOATING_PADDING
        const floatY = winOrigin.y + finalY - FLOATING_PADDING

        flushSync(() => {
          setAppLabel(label)
          setFloatingRebased(false)
          setBarRect({ x: FLOATING_PADDING, y: FLOATING_PADDING, width: barW })
          setFlyDelta({ x: 0, y: 0 })
          setStage(targetStage)
          if (isTranslateMode) {
            // translate 卡片截圖前不渲染 → 沒"起點位置",禁 transition 避免 (selectX,selectY) → (0,0) 瞬時跳動觸發動畫
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

        // AppKit 動畫在原生側非同步跑;+40ms 餘量等 Core Animation 收尾,再切 floatingRebased。
        // 加 motionSeq + stage 守衛防止使用者中途觸發新會話時把舊會話的尾巴覆蓋到新視窗。
        floatingRebaseTimerRef.current = window.setTimeout(() => {
          floatingRebaseTimerRef.current = null
          if (motionSeq !== motionSeqRef.current) return
          if (stageRef.current === 'select') return
          setFloatingRebased(true)
          if (isTranslateMode) setBarIntro(true)
        }, TRANSITION_MS + 40)
      } else {
        // Windows: WebView 始終保持全螢幕,Rust 用 SetWindowRgn 把視窗可見區域裁剪到 bar 矩形。
        // 不走 macOS 的逐幀實搬視窗路線是因為多次 lens 會話後 WebView2 內部狀態會退化導致累積型 jitter。
        flushSync(() => {
          setAppLabel(label)
          setFloatingRebased(false)
          setBarNoTransition(true)
          setBarRect({ x: finalX, y: finalY, width: barW })
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
                    setBarRect({ x: finalX, y: finalY, width: barW })
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
        { x: Math.round(targetX), y: Math.round(targetY), width: barW },
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

  /** translate 模式：截完立即調 OCR + 翻譯。
   *  流式：lens-translate-stream 事件累積 original/translated；done 事件結束並鎖定耗時
   *  非流式：API 返回完整結果一次性灌入（也通過事件，後端在兩步完成後 emit 一次完整 delta） */
  const runTranslate = useCallback(async (id: string) => {
    const translateSeq = motionSeqRef.current
    setTranslateOriginal('')
    setTranslateText('')
    setTranslateError('')
    setTranslateDurationMs(null)
    translateStartRef.current = Date.now()
    setTranslateNow(Date.now())
    try {
      const r = await api.lensTranslate(id)
      if (translateSeq !== motionSeqRef.current || imageIdRef.current !== id) return
      if (!r.success) {
        // 失敗兜底：done 事件應該已經帶 error 了，但補一刀防止前端漏 done
        setTranslateError(r.error || 'Failed')
        if (translateStartRef.current !== null) {
          setTranslateDurationMs(Date.now() - translateStartRef.current)
          translateStartRef.current = null
        }
        setStage('translated')
      }
      // 成功路徑：等 lens-translate-stream 的 done 事件觸發 stage / 計時（避免事件還沒到 stage 就跳，或反之文字還沒到完成態）
    } catch (err) {
      if (translateSeq !== motionSeqRef.current || imageIdRef.current !== id) return
      setTranslateError(err instanceof Error ? err.message : String(err))
      if (translateStartRef.current !== null) {
        setTranslateDurationMs(Date.now() - translateStartRef.current)
        translateStartRef.current = null
      }
      setStage('translated')
    }
  }, [])

  const runReplaceTranslate = useCallback(async (id: string) => {
    const translateSeq = motionSeqRef.current
    setReplaceLines([])
    setReplacePhase('')
    setReplaceError('')
    setTranslateDurationMs(null)
    translateStartRef.current = Date.now()
    setTranslateNow(Date.now())
    try {
      const r = await api.lensReplaceTranslate(id)
      if (translateSeq !== motionSeqRef.current || imageIdRef.current !== id) return
      if (!r.success) {
        setReplaceError(r.error || 'Failed')
        setReplacePhase('done')
        if (translateStartRef.current !== null) {
          setTranslateDurationMs(Date.now() - translateStartRef.current)
          translateStartRef.current = null
        }
        setStage('translated')
      }
    } catch (err) {
      if (translateSeq !== motionSeqRef.current || imageIdRef.current !== id) return
      setReplaceError(err instanceof Error ? err.message : String(err))
      setReplacePhase('done')
      if (translateStartRef.current !== null) {
        setTranslateDurationMs(Date.now() - translateStartRef.current)
        translateStartRef.current = null
      }
      setStage('translated')
    }
  }, [])

  // lens-replace-stream 事件監聽
  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined
    api.onLensReplaceStream((payload: LensReplaceStreamPayload) => {
      if (payload.imageId !== imageIdRef.current) return
      if (payload.error) setReplaceError(payload.error)
      if (payload.lines?.length) setReplaceLines(payload.lines)
      if (payload.phase) setReplacePhase(payload.phase)
      if (payload.phase === 'done') {
        if (translateStartRef.current !== null) {
          setTranslateDurationMs(Date.now() - translateStartRef.current)
          translateStartRef.current = null
        }
        setStage('translated')
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

  // lens-translate-stream 事件監聽（與 lens-stream 同款 cancelled 旗標處理 StrictMode 雙掛）
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

  // translating 期間每秒刷一次，header 走秒
  useEffect(() => {
    if (stage !== 'translating') return
    const id = setInterval(() => setTranslateNow(Date.now()), 1000)
    return () => clearInterval(id)
  }, [stage])

  const handleCaptureWindow = async (info: LensWindowInfo) => {
    // 見 handleCaptureRegion：用 lensOpenSeqRef 而非 motionSeqRef，否則 flyBarToAnchor 後守衛必然失敗。
    const captureOpenSeq = lensOpenSeqRef.current
    // capturingRef 全程 true，避免 macOS screencapture 短暫讓 lens webview 失焦時觸發 blur handler 誤關
    capturingRef.current = true
    try {
      const result = await api.lensCaptureWindow(info.id)
      if (captureOpenSeq !== lensOpenSeqRef.current) return
      if (!result.success || !result.imageId) {
        console.error('lensCaptureWindow failed:', result.error)
        void enterSelect()
        return
      }
      const newId = result.imageId
      imageIdRef.current = newId

      // 記錄截圖框（webview 內座標）作為已截視覺標記，截完保留顯示
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
      if (captureOpenSeq !== lensOpenSeqRef.current) return
      if (mode === 'translate') void runTranslate(newId)
      else if (mode === 'replace') void runReplaceTranslate(newId)
    } finally {
      capturingRef.current = false
    }
  }

  const handleCaptureRegion = async (rect: { x: number; y: number; width: number; height: number }) => {
    // 用 lensOpenSeqRef 做"截圖期間是否開了新會話"的守衛：flyBarToAnchor 內部會 cancelPendingMotion
    // 把 motionSeqRef++，若用 motionSeq 守衛則 flyBar 後必然不等、runTranslate 永不觸發（截圖翻譯卡住）。
    // lensOpenSeqRef 只有 enterSelect（真正新會話）才 bump，flyBar 不動它。
    const captureOpenSeq = lensOpenSeqRef.current
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
    // capturingRef 全程 true 直到 flyBarToAnchor 完成（同 handleCaptureWindow 註釋）
    capturingRef.current = true
    try {
      const result = await api.lensCaptureRegion(params)
      if (captureOpenSeq !== lensOpenSeqRef.current) return
      if (!result.success || !result.imageId) {
        console.error('lensCaptureRegion failed:', result.error)
        void enterSelect()
        return
      }
      const newId = result.imageId
      imageIdRef.current = newId
      setFreezeFrameImageId('')
      // 不清 freezeFramePreview：截圖後仍把凍結幀作為全螢幕背景保留，直到按 Esc 關閉 Lens
      // （enterSelect / resetBeforeHide 會在重開 / 隱藏時清理）。

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
      if (captureOpenSeq !== lensOpenSeqRef.current) return
      if (mode === 'translate') void runTranslate(newId)
      else if (mode === 'replace') void runReplaceTranslate(newId)
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

    // 在對話欄區域鬆開時不觸發截圖（避免點選歷史按鈕/條目時誤截圖）
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

    // 先進入 sending UI，再做合成/註冊，避免這段非同步視窗被 Esc 關閉掉。
    const isFirstTurn = messages.length === 0
    const hasScreenshot = !!imageIdRef.current
    const ctx = (isFirstTurn && mode === 'chat' && !hasScreenshot) ? selectionText.trim() : ''
    if (!hasScreenshot && !ctx && !question.trim()) return
    const userContent = ctx
      ? (lang.startsWith('zh')
          ? `[已選文字]\n${ctx}\n\n[使用者問題]\n${question}`
          : `[Selected Text]\n${ctx}\n\n[Question]\n${question}`)
      : question

    const transferToChat = mode === 'chat' && sendToChatRef.current !== false
    if (transferToChat) {
      // 傳送到 AI 客戶端：不要切到 'answering'（那會讓視窗高度加上 answer 區 → 浮窗展開）。
      // 用 streaming 顯示忙碌、preparingSendRef 守衛 Esc（見 Esc 處理），浮窗保持緊湊直接交接。
      setStreaming(true)
      preparingSendRef.current = true
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
        const result = await api.lensSendToChat(effectiveImageId || '', userContent)
        if (!result.success) {
          console.error('[lens-chat] send failed:', result.error)
          setStreaming(false)
          setStage('ready')
          return
        }
        await closeAfterReset()
      } catch (err) {
        console.error('[lens-chat] handoff failed:', err)
        setStreaming(false)
        setStage('ready')
      } finally {
        preparingSendRef.current = false
      }
      return
    }

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

    // 預設沿用當前 image_id;若有箭頭則先合成 + 註冊新圖,把後續 ask 切到合成版
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
      const result = await api.lensAsk(effectiveImageId || '', sendMessages, {
        webSearch: mode === 'chat' && webSearchEnabled && webSearchAvailable,
      })
      if (!result.success) {
        const errText = `${t.lensError}: ${result.error}`
        setMessages(prev => {
          const last = prev[prev.length - 1]
          if (!last || last.role !== 'assistant') return prev
          return [...prev.slice(0, -1), { role: 'assistant', content: errText }]
        })
      } else if (result.response) {
        // 非流式:把完整答案塞進佔位 assistant;流式情況已在 onLensStream 累積,避免覆蓋
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

  const handleStop = async () => {
    try { await api.lensCancelStream() } catch (err) { console.error(err) }
    // 使用者主動取消但已經流出部分內容，也持久化 —— 關掉再開歷史能接著問
    finishAnswering()
  }

  const handleCopy = async () => {
    // 複製最後一條 assistant 訊息
    const lastAssistant = [...messages].reverse().find(m => m.role === 'assistant' && m.content)
    if (!lastAssistant) return
    const ok = await copyToClipboard(lastAssistant.content)
    if (!ok) return
    setCopied(true)
    if (copyTimeoutRef.current) clearTimeout(copyTimeoutRef.current)
    copyTimeoutRef.current = setTimeout(() => setCopied(false), 2000)
  }

  // 「在 AI 客戶端繼續」：把 Lens 浮窗內的完整多輪歷史 + 截圖同步到客戶端成為一個新會話，然後關閉 Lens。
  // 僅在「傳送到 AI 客戶端」關閉 + 已有完成問答時顯示（見下方按鈕顯隱條件）。
  const handleContinueInChat = async () => {
    if (streaming) return
    // 只帶最終 content（不帶 reasoning/web 搜尋狀態），保持順序。
    const history = messages
      .filter(m => m.content.trim().length > 0)
      .map(m => ({ role: m.role, content: m.content }))
    if (history.length === 0) return
    setStreaming(true)
    preparingSendRef.current = true
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
          }
        } catch (err) {
          console.warn('[lens-chat] compose annotated image failed, fallback to original:', err)
        }
      }
      const result = await api.lensSendHistoryToChat(effectiveImageId || '', history)
      if (!result.success) {
        console.error('[lens-chat] continue-in-chat failed:', result.error)
        setStreaming(false)
        return
      }
      await closeAfterReset()
    } catch (err) {
      console.error('[lens-chat] continue-in-chat handoff failed:', err)
      setStreaming(false)
    } finally {
      preparingSendRef.current = false
    }
  }

  // 點選歷史項：把當前會話恢復到該 item（image / appLabel / messages / capturedFrame）
  // 取消任何正在跑的流，避免後端繼續 emit delta 灌入新恢復的 messages（如果新舊 imageId 巧合相同會汙染）
  const restoreHistory = (item: HistoryItem) => {
    setHistoryOpen(false)
    if (streaming) {
      void api.lensCancelStream().catch(err => console.error(err))
    }
    imageIdRef.current = item.id
    // 防禦：恢復歷史 setMessages 會觸發持久化 effect，但本路徑不是"流剛結束"，不該 push 重複條目
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
    // 老 takeLensSelection promise 失效，避免恢復歷史後被新 take 文字汙染
    selectionReqIdRef.current++
    focusLensSurface([50, 140, 260])
  }

  // 相對時間字串（"剛剛" / "3 分鐘前"）
  const relTime = (ts: number): string => {
    const diff = Date.now() - ts
    const m = Math.floor(diff / 60000)
    if (m < 1) return lang.startsWith('zh') ? '剛剛' : 'just now'
    if (m < 60) return lang.startsWith('zh') ? `${m} 分鐘前` : `${m}m ago`
    const h = Math.floor(m / 60)
    if (h < 24) return lang.startsWith('zh') ? `${h} 小時前` : `${h}h ago`
    return lang.startsWith('zh') ? `${Math.floor(h / 24)} 天前` : `${Math.floor(h / 24)}d ago`
  }

  useEffect(() => () => {
    if (copyTimeoutRef.current) clearTimeout(copyTimeoutRef.current)
    cancelPendingMotion()
    focusReqIdRef.current++
  }, [cancelPendingMotion])

  // 點選 history 面板外部 → 關閉
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

  // 測量 history 面板實際高度,供浮動模式 resize 副作用按需擴窗(不然面板上方/下方溢位會被 OS 裁掉)。
  // useLayoutEffect 確保在瀏覽器 paint 之前同步算出高度並 setState,resize 副作用立刻拿到新值擴窗,
  // 不會出現"面板已渲染但視窗沒擴"的中間幀。
  useLayoutEffect(() => {
    if (historyOpen && historyContentRef.current) {
      setHistoryPanelH(historyContentRef.current.offsetHeight)
    } else {
      setHistoryPanelH(0)
    }
  }, [historyOpen, history])

  // ====== 單一渲染 ======
  const showThumb = stage !== 'select' && (imagePreview || appLabel)
  // 流式期間禁止傳送/輸入，答完之後可對同一張截圖繼續問新問題（每次仍為獨立 Q&A，自動入歷史）
  const sendDisabled = streaming
  // 對話欄（輸入框）只在 chat 模式顯示；translate 模式只渲染浮動結果卡片
  const showBar = mode === 'chat'
  // translate 浮動卡片：截圖後在選區旁出現，載入/完成兩態
  const showTranslateCard = (mode === 'translate' || mode === 'translateText') && (stage === 'translating' || stage === 'translated')
  const showReplaceOverlay = mode === 'replace' && capturedFrame && (stage === 'translating' || stage === 'translated')
  const replaceStatusLabel = replaceError
    ? (replaceError === 'rapidocr_models_missing' ? t.rapidOcrModelsMissing : replaceError)
    : replacePhase === 'ocr'
      ? t.replaceTranslateStatusOcr
      : replacePhase === 'translating'
        ? t.replaceTranslateStatusTranslating
        : t.replaceTranslateStatusDone
  // 浮動佈局僅用於截圖翻譯關閉全螢幕覆蓋、或 translateText 文字翻譯卡。
  // 普通 Lens 截圖後固定保持全螢幕 overlay，只移動輸入欄。
  // capturedFrame 只在最近一次截圖後非空,而 restoreHistory 會清掉它(歷史項的選區不再相關);
  // 但此時 lens 視窗仍是浮動小尺寸 → 必須疊加 floatingRebased 才能正確反映"視窗當前在浮動態"。
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

  // 答案區展開方向 + 高度自適應：
  // 1) 下方空間夠 ANSWER_H → 向下，目標高
  // 2) 上方空間夠 → 向上，目標高
  // 3) 都不夠 → 選大的那側，高度收縮為該側可用空間（最少 180，避免太矮）
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

  // 浮動模式下：截圖翻譯 / 文字翻譯的 stage 或佈局變化時動態調整視窗尺寸
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

    // translate 卡片預留空間
    if ((stage === 'translating' || stage === 'translated') && (mode === 'translate' || mode === 'translateText')) {
      h = Math.max(h, READY_BAR_H + FLOATING_GAP + stableAnswerHeight + FLOATING_PADDING * 2)
    }

    // history 面板:浮動模式下面板渲染在 bar 下方(top: 100%+18 = bar bottom + 8),
    // 視窗必須擴到 bar bottom + 8 + 面板高度,否則面板被 OS 裁掉。
    // 全螢幕模式不需要擴,面板渲染在 bar 上方已有空間。
    if (isFloatingLayout && historyOpen && historyPanelH > 0) {
      h = Math.max(h, READY_BAR_H + FLOATING_GAP + historyPanelH + FLOATING_PADDING * 2)
    }

    // macOS 上視窗已經在 rebase 時搬到螢幕錨點,barRect 是視窗內座標 (0, 0)。
    // 這裡若再傳 x/y 會把視窗搬到螢幕 (0, 0)。只傳 width/height,讓 OS 保持當前 origin。
    // translateText 是天生小窗(開窗即貼遊標 set_size),同樣只改尺寸——絕不 SetWindowRgn,
    // 否則透明無邊框 WebView2 + region 會渲染出黑塊 + 原生標題欄(tauri#14764)。
    // Windows 的截圖翻譯 / lens 全螢幕→浮動才走 SetWindowRgn(必須傳 x/y 更新裁剪區)。
    if (isMacPlatform || mode === 'translateText') {
      api.lensSetFloating({ width: w, height: h }).catch(err => console.error('[lens-floating] resize failed:', err))
    } else {
      api.lensSetFloating({ x, y, width: w, height: h }).catch(err => console.error('[lens-floating] resize failed:', err))
    }
  }, [stage, answerLayout, barRect, floatingRebased, keepFullscreen, mode, stableAnswerHeight, historyOpen, historyPanelH, isFloatingLayout])

  if (surfaceDormant) {
    return (
      <div
        ref={rootRef}
        tabIndex={-1}
        aria-hidden
        className="fixed left-0 top-0 w-px h-px pointer-events-none select-none opacity-0 outline-none"
        data-tauri-drag-region="false"
      />
    )
  }

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
      {(stage === 'select' || keepFullscreen) && freezeFramePreview && (
        <canvas
          ref={freezeCanvasRef}
          aria-hidden
          className="absolute inset-0 w-full h-full pointer-events-none"
        />
      )}

      {/* select 態全螢幕覆蓋層：完全透明，僅用於捕獲滑鼠事件，不再加黑色蒙層 */}
      <div
        className="absolute inset-0 transition-opacity ease-out pointer-events-none"
        style={{
          backgroundColor: 'transparent',
          transitionDuration: `${TRANSITION_MS}ms`,
          opacity: stage === 'select' && !hoverRect && !dragRect ? 1 : 0,
        }}
      />

      {/* 已截圖框：截完保留顯示作為視覺標記（橙色邊框 + 淺外發光，無挖洞遮罩） */}
      {/* 浮動模式下不顯示高亮框 */}
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
        </>
      )}

      {showReplaceOverlay && capturedFrame && imagePreview && (
        <ReplaceTranslateOverlay
          frame={capturedFrame}
          imagePreview={imagePreview}
          lines={replaceLines}
          phase={replacePhase}
          error={replaceError || undefined}
          statusLabel={replaceStatusLabel}
          escHint={t.replaceTranslateEscHint}
          freezeCanvasRef={freezeCanvasRef}
        />
      )}

      {/* drawMode 關閉時也持續顯示已落下的箭頭 */}
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

      {/* drawMode:在 capturedFrame 矩形內畫箭頭.透明 div 收事件、SVG 渲染,
          不加 dim、不再貼 imagePreview 背景,直接顯示原畫面 */}
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
            // 瀏覽器主動釋放捕獲(例如系統對話方塊打斷),清掉 draft
            e.stopPropagation()
            setDraftArrow(null)
            try { (e.currentTarget as HTMLDivElement).releasePointerCapture(e.pointerId) } catch { /* 已被釋放,忽略 */ }
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

      {/* select-only：hover 高亮 / drag 選區 / 頂部 hint */}
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

      {/* 對話欄 + 答案區：始終渲染，輸入欄移動只用 transform，位置/尺寸直接 snap。
          - select：底部居中 680，縮圖槽位用 sparkle 佔位
          - ready：飛到選區附近 600，左側切換為縮圖 + 應用名
          - answering：在對話欄下方 absolute 展開 answer 區（固定 360 高） */}
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
          {/* 輸入欄卡片 */}
          <div
            className="flex w-full min-w-0 items-center gap-2.5 pl-4 pr-2 py-2 rounded-[18px] bg-white dark:bg-neutral-900 border border-black/[0.07] dark:border-white/[0.08] lens-floating-surface cursor-default overflow-visible"
            data-tauri-drag-region="false"
          >
            <div className="flex min-w-0 shrink items-center gap-2">
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
                    <span className="text-[13px] font-medium text-neutral-800 dark:text-neutral-200 max-w-[72px] truncate">{appLabel}</span>
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
                  title={lang.startsWith('zh') ? `已選取 ${selectionLineCount} 行` : `${selectionLineCount} lines selected`}
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
                // IME 合成中（中/日/韓選詞按回車）跳過 — isComposing 官方訊號 + keyCode 229 兜底
                if (e.nativeEvent.isComposing || e.keyCode === 229) return
                e.preventDefault()
                void handleSend()
              }}
              readOnly={streaming}
              aria-disabled={streaming}
              placeholder={t.lensAskPlaceholder}
              className={`min-w-0 flex-1 bg-transparent text-[16px] text-neutral-900 dark:text-white placeholder-neutral-500 dark:placeholder-neutral-400 focus:outline-none ${streaming ? 'opacity-60' : ''}`}
            />
            {/* ponytail: 網路搜尋按鈕已隱藏；webSearchEnabled 仍由 line ~312 在可用時自動開啟，功能不變 */}
            {/* History dropdown：按鈕 + 彈出面板（容器作為 ref，點選外部關閉） */}
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
                    // 浮動模式下 lens 視窗只覆蓋 bar 矩形,面板若按 bottom-full 渲染到 bar 上方會被 OS 裁掉。
                    // 改為渲染到 bar 下方:從 trigger 容器向下偏移 8 (gap) + 10 (bar 內 trigger 頂部的 padding) = 18 = bar bottom + 8。
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
                        // 首條 user 訊息可能含 [已選文字]\n...\n\n[使用者問題]\n... 的拼接形式（chat 啟動注入），
                        // 歷史預覽只顯示問題原文，剝掉 marker 段
                        const firstUserRaw = item.messages.find(m => m.role === 'user')?.content ?? ''
                        const zhMarker = '[使用者問題]\n'
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
                                {item.appLabel ? `${item.appLabel} · ` : ''}{turns > 1 ? `${turns} 輪 · ` : ''}{relTime(item.timestamp)}
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

          {/* select 態鍵盤提示（在對話欄卡片下方） */}
          {stage === 'select' && (
            <div className="mt-2 flex justify-center gap-3 text-[11px] text-white/70 pointer-events-none">
              <span>↵ {t.lensHintSend}</span>
              <span>·</span>
              <span>esc {t.lensHintEsc}</span>
            </div>
          )}

          {/* answer 區：absolute 展開在對話欄上方或下方（自適應空間），渲染整個 chat list（多輪對話） */}
          <div
            className="absolute left-0 right-0 rounded-2xl overflow-hidden window-frosted lens-floating-surface transition-all ease-out select-text"
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
              // 顯示順序：desc 反轉陣列（新在頂）；isLast 始終基於原陣列末尾索引（最新的）
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
                  {/* 「傳送到 AI 客戶端」關閉時：把當前完整多輪歷史 + 截圖轉交客戶端繼續聊。
                      僅 chat 模式、非流式、且已有完成問答（showActions 保證）時顯示。 */}
                  {mode === 'chat' && sendToChatRef.current === false && !streaming && (
                    <button
                      onClick={() => void handleContinueInChat()}
                      title={t.lensContinueInChat}
                      className="flex items-center gap-1 px-2 py-0.5 text-[10px] text-neutral-500 hover:text-[#D97757] dark:text-neutral-400 dark:hover:text-[#D97757] rounded hover:bg-black/5 dark:hover:bg-white/10 transition-colors"
                    >
                      <MessageSquarePlus size={11} />
                      <span>{t.lensContinueInChat}</span>
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
                {/* desc 模式下操作按鈕放最前（貼最新答案） */}
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
                        <div>
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
                              <ChatMarkdown content={m.content} variant="lens" />
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
                {/* asc 模式下操作按鈕在末尾 */}
                {messageOrder === 'asc' && showActions && Actions}
              </div>
              )
            })()}
          </div>
        </div>
      )}

      {/* translate 模式浮動結果卡：原文 + 譯文，複用 barRect 錨點。
          外層 select-none 用 select-text 覆蓋，讓使用者可選中複製部分文字。 */}
      {showTranslateCard && (
        <div
          className="absolute ease-out rounded-2xl bg-white dark:bg-neutral-900 border border-black/[0.07] dark:border-white/[0.08] lens-floating-surface overflow-hidden select-text"
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
          {/* 頂部縮圖 + 應用名 + 狀態徽章（耗時 / token 估算） */}
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

          {/* 內容區 */}
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
                {/* 譯文區（主體）：合併模式下分隔符前的所有 delta 都屬於這塊，先於原文出現 */}
                {translateText ? (
                  <ChatMarkdown content={translateText} variant="lens" />
                ) : (
                  <div className="space-y-2">
                    <div className="h-3.5 rounded bg-gradient-to-r from-neutral-200 via-neutral-100 to-neutral-200 dark:from-neutral-800 dark:via-neutral-700 dark:to-neutral-800 bg-[length:200%_100%] animate-[shimmer_1.4s_linear_infinite]" />
                    <div className="h-3.5 rounded bg-gradient-to-r from-neutral-200 via-neutral-100 to-neutral-200 dark:from-neutral-800 dark:via-neutral-700 dark:to-neutral-800 bg-[length:200%_100%] animate-[shimmer_1.4s_linear_infinite] w-[88%]" />
                    <div className="h-3.5 rounded bg-gradient-to-r from-neutral-200 via-neutral-100 to-neutral-200 dark:from-neutral-800 dark:via-neutral-700 dark:to-neutral-800 bg-[length:200%_100%] animate-[shimmer_1.4s_linear_infinite] w-[72%]" />
                  </div>
                )}
                {/* 原文區（參考）：分隔符之後的 delta 才到這裡，置於譯文下方小字灰色 */}
                {translateOriginal && mode !== 'translateText' && (
                  <>
                    <div className="border-t border-black/[0.05] dark:border-white/[0.06] -mx-3.5 my-3" />
                    <ChatMarkdown content={translateOriginal} variant="lens-muted" />
                  </>
                )}
              </>
            )}
          </div>

          {/* 底部操作欄：複製譯文 */}
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
