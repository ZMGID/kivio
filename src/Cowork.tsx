import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { flushSync } from 'react-dom'
import { Loader2, Copy, Check, Square, Image as ImageIcon, Sparkles, ArrowUp, History as HistoryIcon, ChevronDown } from 'lucide-react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { api, type CoworkStreamPayload, type CoworkWindowInfo, type ExplainMessage } from './api/tauri'
import ReactMarkdown from 'react-markdown'
import remarkMath from 'remark-math'
import rehypeKatex from 'rehype-katex'
import 'katex/dist/katex.min.css'
import { i18n, type Lang } from './settings/i18n'
import { copyToClipboard } from './utils/clipboard'

type Stage = 'select' | 'ready' | 'answering'
type Point = { x: number; y: number }
type BarRect = { x: number; y: number; width: number }
type CapturedFrame = { x: number; y: number; width: number; height: number; label: string }
type HistoryItem = {
  id: string                   // imageId（恢复时复用，重新提问会用同一张图）
  imagePreview: string         // base64 data URL
  appLabel: string
  messages: ExplainMessage[]   // 完整多轮对话
  capturedFrame: CapturedFrame | null
  timestamp: number
}

const HISTORY_MAX = 20

const READY_BAR_H = 56            // 对话栏单行高度（与字号绑定，不随屏幕变）
const ANCHOR_GAP = 12              // 对话栏与选区之间的水平间距
const DRAG_THRESHOLD = 5
const TRANSITION_MS = 380

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

/**
 * Cowork 模式：单 webview 三态机，统一 DOM。
 * - select：webview 全屏 + 灰幕 + hover 应用窗口高亮 + 区域 drag + 底部对话栏（纯文字直发）
 * - ready：截图后对话栏 CSS transition 飞到选区附近，加缩略图，输入聚焦
 * - answering：对话栏下方展开 answer 区（透明背景，对话栏不动）
 *
 * 关键：webview 始终全屏，整个过渡靠 CSS。后端 cowork_resolve_anchor 仅算目标坐标，不缩窗口。
 * explain 模式（hash mode=explain）：截图后调 explainOpenAtAnchor 打开 explain 大窗口，本 webview 由后端 hide。
 */
export default function Cowork() {
  const [stage, setStage] = useState<Stage>('select')
  const [windows, setWindows] = useState<CoworkWindowInfo[]>([])
  const [hovered, setHovered] = useState<CoworkWindowInfo | null>(null)
  const [winOrigin, setWinOrigin] = useState<{ x: number; y: number }>({ x: 0, y: 0 })
  const [dragStart, setDragStart] = useState<Point | null>(null)
  const [dragCurrent, setDragCurrent] = useState<Point | null>(null)
  const [dragging, setDragging] = useState(false)
  const [imagePreview, setImagePreview] = useState('')
  const [appLabel, setAppLabel] = useState('')
  const [input, setInput] = useState('')
  const [messages, setMessages] = useState<ExplainMessage[]>([])
  const [streaming, setStreaming] = useState(false)
  const [copied, setCopied] = useState(false)
  const [lang, setLang] = useState<Lang>('zh')
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
  // capturedFrame：保留最后一次截图选区/窗口的高亮框，作为"已截图"视觉标记，ready/answering 态继续显示
  const [capturedFrame, setCapturedFrame] = useState<CapturedFrame | null>(null)
  // mode：从 hash 读取（#cowork 或 #cowork?mode=explain）
  const readMode = (): 'cowork' | 'explain' => {
    const m = window.location.hash.match(/[?&]mode=([^&]+)/)
    return m?.[1] === 'explain' ? 'explain' : 'cowork'
  }
  const [mode, setMode] = useState<'cowork' | 'explain'>(readMode)
  // 内存历史：单次 app 生命周期保留，esc/hide 不清空
  const [history, setHistory] = useState<HistoryItem[]>([])
  const [historyOpen, setHistoryOpen] = useState(false)

  const inputRef = useRef<HTMLInputElement>(null)
  const historyPanelRef = useRef<HTMLDivElement>(null)
  const stageRef = useRef<Stage>('select')
  const imageIdRef = useRef('')
  const copyTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  // capture 期间 macOS screencapture 可能短暂让 cowork webview 失焦 → 触发 blur 误关闭。
  // 这个 ref 标记"截图进行中"，blur handler 看到就跳过。
  const capturingRef = useRef(false)
  // 答案区滚动容器，stream 时自动滚到底部
  const chatScrollRef = useRef<HTMLDivElement>(null)

  const t = i18n[lang]
  stageRef.current = stage

  // 加载语言
  useEffect(() => {
    void (async () => {
      try {
        const settings = await api.getSettings()
        setLang((settings.settingsLanguage as Lang) || 'zh')
      } catch (err) { console.error('Failed to load lang', err) }
    })()
  }, [])

  // select 态进入：刷新所有 state、重算对话栏位置、播放 intro 动画
  const enterSelect = useCallback(async () => {
    setStage('select')
    setHovered(null)
    setDragStart(null)
    setDragCurrent(null)
    setDragging(false)
    setImagePreview('')
    setAppLabel('')
    setInput('')
    setMessages([])
    setStreaming(false)
    imageIdRef.current = ''
    setMode(readMode())
    {
      const w = window.innerWidth
      const h = window.innerHeight
      setViewport({ w, h })
      setBarRect(computeSelectBar(w, h, computeMetrics(w, h)))
    }
    setCapturedFrame(null)
    // 重置 intro：先关再开，下一帧让 transition 从 scale-90 到 scale-100
    setBarIntro(false)
    requestAnimationFrame(() => {
      requestAnimationFrame(() => setBarIntro(true))
    })
    try {
      const win = getCurrentWindow()
      const [pos, scale] = await Promise.all([win.outerPosition(), win.scaleFactor()])
      const sf = scale || 1
      setWinOrigin({ x: pos.x / sf, y: pos.y / sf })
    } catch (err) { console.error('Failed to read window origin', err) }
    try {
      const list = await api.coworkListWindows()
      setWindows(list)
    } catch (err) {
      console.error('Failed to list windows', err)
      setWindows([])
    }
    void api.showWindow()
  }, [])

  useEffect(() => {
    void enterSelect()
    const handleReset = () => { void enterSelect() }
    window.addEventListener('cowork:reset', handleReset)
    return () => window.removeEventListener('cowork:reset', handleReset)
  }, [enterSelect])

  // viewport resize（拔显示器 / 切分辨率 / DPI 变更）→ 更新 viewport state，触发 metrics 重算
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
  useEffect(() => {
    if (streaming) return
    if (!imageIdRef.current || messages.length === 0) return
    const hasAssistant = messages.some(m => m.role === 'assistant' && m.content)
    if (!hasAssistant) return
    setHistory(prev => {
      const filtered = prev.filter(h => h.id !== imageIdRef.current)
      const next: HistoryItem = {
        id: imageIdRef.current,
        imagePreview,
        appLabel,
        messages,
        capturedFrame,
        timestamp: Date.now(),
      }
      return [next, ...filtered].slice(0, HISTORY_MAX)
    })
  }, [streaming, messages, imagePreview, appLabel, capturedFrame])

  // 监听 cowork-stream 事件追加 delta 到最后一条 assistant 消息
  useEffect(() => {
    let unlisten: (() => void) | undefined
    api.onCoworkStream((payload: CoworkStreamPayload) => {
      if (payload.imageId !== imageIdRef.current) return
      if (payload.done) {
        setStreaming(false)
        return
      }
      if (payload.delta) {
        setMessages(prev => {
          const last = prev[prev.length - 1]
          if (!last || last.role !== 'assistant') return prev
          return [...prev.slice(0, -1), { ...last, content: last.content + payload.delta }]
        })
      }
    }).then((dispose) => {
      unlisten = dispose
    }).catch(err => console.error(err))
    return () => { unlisten?.() }
  }, [])

  // messages 变化时滚动到底部（追新答案 / 新提问都自动滚动）
  useEffect(() => {
    const el = chatScrollRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [messages])

  // 关闭前同步重置 state，让 webview surface 在 hide 之前已经是空 select 态。
  // 否则下次 show 时 macOS 会先显示上次的 ready 态 surface 一帧，再被 cowork:reset 覆盖 → 闪一下上次内容。
  const resetBeforeHide = useCallback(() => {
    flushSync(() => {
      setStage('select')
      setHovered(null)
      setDragStart(null)
      setDragCurrent(null)
      setDragging(false)
      setImagePreview('')
      setAppLabel('')
      setInput('')
      setMessages([])
      setStreaming(false)
      setBarRect(computeSelectBar(viewport.w, viewport.h, metrics))
      setCapturedFrame(null)
      setBarIntro(false)
    })
    imageIdRef.current = ''
  }, [viewport, metrics])

  // 全局 Esc：流式时取消流 / 否则关闭
  useEffect(() => {
    const handler = async (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      if (stageRef.current === 'answering' && streaming) {
        try { await api.coworkCancelStream() } catch (err) { console.error(err) }
        setStreaming(false)
        return
      }
      resetBeforeHide()
      try { await api.coworkClose() } catch (err) { console.error(err) }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [streaming, resetBeforeHide])

  // select 态切到其他应用 → 自动收起灰幕。
  // 注意：截图过程中 screencapture 可能让 cowork 短暂失焦，capturingRef 防止误关。
  useEffect(() => {
    const handleBlur = () => {
      if (capturingRef.current) return
      if (stageRef.current === 'select') {
        resetBeforeHide()
        void api.coworkClose()
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
  const hitTest = (gp: Point): CoworkWindowInfo | null => {
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

  /** 截图后 cowork 默认模式：在前端直接算 bar 位置，让对话栏飞到选区左/右侧（不再上下出现）。
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

    flushSync(() => {
      setAppLabel(label)
      setBarRect({ x: Math.round(targetX), y: Math.round(targetY), width: READY_W })
      setStage('ready')
    })
    setTimeout(() => inputRef.current?.focus(), TRANSITION_MS + 20)
  }

  const handleCaptureWindow = async (info: CoworkWindowInfo) => {
    capturingRef.current = true
    let result
    try {
      result = await api.coworkCaptureWindow(info.id)
    } finally {
      // 截图本身完成后释放 flag；后续 React 渲染 / resolveAnchor 不影响 blur 误判
      capturingRef.current = false
    }
    if (!result.success || !result.imageId) {
      console.error('coworkCaptureWindow failed:', result.error)
      void enterSelect()
      return
    }
    const newId = result.imageId
    imageIdRef.current = newId

    if (mode === 'explain') {
      try {
        await api.explainOpenAtAnchor({
          imageId: newId,
          anchorX: Math.round(info.x),
          anchorY: Math.round(info.y),
          anchorWidth: Math.round(info.width),
          anchorHeight: Math.round(info.height),
        })
      } catch (err) {
        console.error('explainOpenAtAnchor failed', err)
        try { await api.coworkClose() } catch { /* ignore */ }
      }
      return
    }

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
    capturingRef.current = true
    let result
    try {
      result = await api.coworkCaptureRegion(params)
    } finally {
      capturingRef.current = false
    }
    if (!result.success || !result.imageId) {
      console.error('coworkCaptureRegion failed:', result.error)
      void enterSelect()
      return
    }
    const newId = result.imageId
    imageIdRef.current = newId

    if (mode === 'explain') {
      try {
        await api.explainOpenAtAnchor({
          imageId: newId,
          anchorX: params.absoluteX,
          anchorY: params.absoluteY,
          anchorWidth: params.width,
          anchorHeight: params.height,
        })
      } catch (err) {
        console.error('explainOpenAtAnchor failed', err)
        try { await api.coworkClose() } catch { /* ignore */ }
      }
      return
    }

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
    const id = imageIdRef.current
    setHistoryOpen(false)
    setInput('')

    const userMsg: ExplainMessage = { role: 'user', content: question }
    const placeholder: ExplainMessage = { role: 'assistant', content: '' }
    // sendMessages：发给后端的 history（保留前面对话上下文 + 本次提问，最后一条是 user 提问）
    const sendMessages: ExplainMessage[] = [...messages, userMsg]
    flushSync(() => {
      setMessages([...sendMessages, placeholder])
      setStage('answering')
      setStreaming(true)
    })
    try {
      const result = await api.coworkAsk(id || '', sendMessages)
      if (!result.success) {
        const errText = `${t.coworkError}: ${result.error}`
        setMessages(prev => {
          const last = prev[prev.length - 1]
          if (!last || last.role !== 'assistant') return prev
          return [...prev.slice(0, -1), { role: 'assistant', content: errText }]
        })
      } else if (result.response) {
        // 非流式：把完整答案塞进占位 assistant；流式情况已在 onCoworkStream 累积，避免覆盖
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
        return [...prev.slice(0, -1), { role: 'assistant', content: `${t.coworkError}: ${msg}` }]
      })
    } finally {
      setStreaming(false)
    }
  }

  const handleStop = async () => {
    try { await api.coworkCancelStream() } catch (err) { console.error(err) }
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
  const restoreHistory = (item: HistoryItem) => {
    setHistoryOpen(false)
    imageIdRef.current = item.id
    flushSync(() => {
      setImagePreview(item.imagePreview)
      setAppLabel(item.appLabel)
      setInput('')
      setMessages(item.messages)
      setCapturedFrame(item.capturedFrame)
      setStreaming(false)
      setStage('answering')
    })
    setTimeout(() => inputRef.current?.focus(), 50)
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
  // 是否显示底部对话栏：cowork 默认模式始终显示；explain 模式仅 select 态隐藏（截图后由 explain 大窗口接管）
  const showBar = mode !== 'explain'

  // 答案区展开方向 + 高度自适应：
  // 1) 下方空间够 ANSWER_H → 向下，目标高
  // 2) 上方空间够 → 向上，目标高
  // 3) 都不够 → 选大的那侧，高度收缩为该侧可用空间（最少 180，避免太矮）
  const answerLayout = useMemo(() => {
    const target = metrics.ANSWER_H
    const spaceBelow = viewport.h - (barRect.y + READY_BAR_H + 8) - 16
    const spaceAbove = barRect.y - 8 - 16
    if (spaceBelow >= target) return { placeAbove: false, height: target }
    if (spaceAbove >= target) return { placeAbove: true, height: target }
    if (spaceAbove > spaceBelow) {
      return { placeAbove: true, height: Math.max(180, spaceAbove) }
    }
    return { placeAbove: false, height: Math.max(180, spaceBelow) }
  }, [barRect, metrics, viewport.h])

  return (
    <div
      className="fixed inset-0 select-none"
      onMouseDown={handleMouseDown}
      onMouseMove={handleMouseMove}
      onMouseUp={handleMouseUp}
      data-tauri-drag-region="false"
    >
      {/* 灰幕：select 态显示，截图后 fade out */}
      <div
        className="absolute inset-0 transition-opacity ease-out pointer-events-none"
        style={{
          backgroundColor: 'rgba(0,0,0,0.4)',
          transitionDuration: `${TRANSITION_MS}ms`,
          opacity: stage === 'select' && !hoverRect && !dragRect ? 1 : 0,
        }}
      />

      {/* 已截图框：截完保留显示作为视觉标记（橙色边框 + 浅外发光，无挖洞遮罩） */}
      {capturedFrame && stage !== 'select' && (
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
              {t.coworkScreenshotOf} {capturedFrame.label}
            </div>
          )}
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
                  boxShadow:
                    '0 0 0 9999px rgba(0,0,0,0.4), 0 0 16px 2px rgba(217,119,87,0.45)',
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
                  {t.coworkScreenshotOf} {hovered.owner}
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
                boxShadow:
                  '0 0 0 9999px rgba(0,0,0,0.5), 0 0 16px 2px rgba(217,119,87,0.45)',
              }}
            />
          )}
          <div className="absolute top-6 left-1/2 -translate-x-1/2 px-3 py-1.5 rounded-full bg-neutral-900/80 backdrop-blur text-white text-[12px] font-medium pointer-events-none">
            {dragging
              ? t.coworkSelectHintDrag
              : hovered
                ? t.coworkSelectHintHover.replace('{app}', hovered.owner)
                : t.coworkSelectHintIdle}
          </div>
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
            transitionProperty: 'left, top, width, transform, opacity',
            transitionDuration: `${TRANSITION_MS}ms`,
            transitionTimingFunction: 'cubic-bezier(0.22, 1, 0.36, 1)',
            transform: barIntro ? 'scale(1)' : 'scale(0.92)',
            opacity: barIntro ? 1 : 0,
          }}
        >
          {/* 输入栏卡片 */}
          <div
            className="flex items-center gap-3 pl-4 pr-2 py-2 rounded-[18px] bg-white dark:bg-neutral-900 shadow-[0_18px_50px_-12px_rgba(0,0,0,0.4)] ring-1 ring-black/[0.04] dark:ring-white/[0.06] cursor-default"
            data-tauri-drag-region="false"
          >
            {showThumb ? (
              <div className="shrink-0 flex items-center gap-2.5">
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
              <Sparkles size={20} strokeWidth={1.75} className="shrink-0 text-[#D97757]" />
            )}
            <input
              ref={inputRef}
              autoFocus
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={(e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); void handleSend() } }}
              disabled={streaming}
              placeholder={t.coworkAskPlaceholder}
              className="flex-1 bg-transparent text-[16px] text-neutral-900 dark:text-white placeholder-neutral-500 dark:placeholder-neutral-400 focus:outline-none disabled:opacity-60"
            />
            {/* History dropdown：按钮 + 弹出面板（容器作为 ref，点击外部关闭） */}
            <div ref={historyPanelRef} className="relative shrink-0">
              <button
                type="button"
                onClick={() => setHistoryOpen(o => !o)}
                className="flex items-center gap-1 h-9 px-2.5 rounded-lg text-neutral-600 dark:text-neutral-300 hover:bg-black/[0.05] dark:hover:bg-white/[0.06] transition-colors"
                title={t.coworkHistory}
              >
                <HistoryIcon size={15} strokeWidth={1.75} />
                {history.length > 0 && (
                  <span className="text-[11px] font-medium tabular-nums text-neutral-500 dark:text-neutral-400">{history.length}</span>
                )}
                <ChevronDown size={13} strokeWidth={2} className={`transition-transform ${historyOpen ? 'rotate-180' : ''}`} />
              </button>
              {historyOpen && (
                <div
                  className="absolute right-0 bottom-full mb-2 w-[240px] rounded-xl bg-white dark:bg-neutral-900 shadow-[0_18px_44px_-12px_rgba(0,0,0,0.4)] ring-1 ring-black/[0.06] dark:ring-white/[0.08] overflow-hidden"
                >
                  <div className="max-h-[200px] overflow-y-auto custom-scrollbar py-1">
                    {history.length === 0 ? (
                      <div className="px-2.5 py-1.5 text-[11px] text-neutral-400 dark:text-neutral-500">
                        {t.coworkNoHistory}
                      </div>
                    ) : (
                      history.map(item => {
                        const firstUserQ = item.messages.find(m => m.role === 'user')?.content ?? ''
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
              <span>↵ {t.coworkHintSend}</span>
              <span>·</span>
              <span>esc {t.coworkHintEsc}</span>
            </div>
          )}

          {/* answer 区：absolute 展开在对话栏上方或下方（自适应空间），渲染整个 chat list（多轮对话） */}
          <div
            className="absolute left-0 right-0 rounded-2xl overflow-hidden window-frosted transition-all ease-out"
            style={{
              top: answerLayout.placeAbove ? undefined : 'calc(100% + 8px)',
              bottom: answerLayout.placeAbove ? 'calc(100% + 8px)' : undefined,
              height: stage === 'answering' ? answerLayout.height : 0,
              opacity: stage === 'answering' ? 1 : 0,
              transitionDuration: `${TRANSITION_MS}ms`,
              pointerEvents: stage === 'answering' ? 'auto' : 'none',
            }}
          >
            {stage === 'answering' && (
              <div ref={chatScrollRef} className="h-full overflow-y-auto custom-scrollbar px-3.5 py-3">
                {messages.map((m, i) => {
                  const isUser = m.role === 'user'
                  const isLast = i === messages.length - 1
                  return (
                    <div key={i} className={`mb-3 ${isUser ? 'flex justify-end' : ''}`}>
                      {isUser ? (
                        <div className="px-3 py-2 rounded-2xl bg-[#D97757]/15 dark:bg-[#D97757]/20 text-[13.5px] text-neutral-800 dark:text-neutral-100 max-w-[88%] whitespace-pre-wrap break-words">
                          {m.content}
                        </div>
                      ) : (
                        <div className="prose prose-sm dark:prose-invert max-w-none text-[13.5px] leading-7 text-neutral-800 dark:text-neutral-200">
                          {m.content ? (
                            <ReactMarkdown remarkPlugins={[remarkMath]} rehypePlugins={[rehypeKatex]}>
                              {m.content}
                            </ReactMarkdown>
                          ) : isLast && streaming ? (
                            <div className="not-prose flex items-center gap-2 text-neutral-500 dark:text-neutral-400">
                              <Loader2 className="animate-spin" size={14} />
                              <span className="text-[12px]">{t.coworkAsking}</span>
                            </div>
                          ) : null}
                        </div>
                      )}
                    </div>
                  )
                })}
                {/* 操作按钮：仅当最后一条 assistant 有内容时显示 */}
                {(() => {
                  const last = messages[messages.length - 1]
                  if (!last || last.role !== 'assistant' || !last.content) return null
                  return (
                    <div className="flex items-center gap-1">
                      <button
                        onClick={() => void handleCopy()}
                        className="flex items-center gap-1 px-2 py-0.5 text-[10px] text-neutral-500 hover:text-neutral-800 dark:text-neutral-400 dark:hover:text-neutral-100 rounded hover:bg-black/5 dark:hover:bg-white/10 transition-colors"
                      >
                        {copied ? <Check size={11} /> : <Copy size={11} />}
                        <span>{copied ? t.coworkCopied : t.coworkCopy}</span>
                      </button>
                      {streaming && (
                        <button
                          onClick={() => void handleStop()}
                          className="flex items-center gap-1 px-2 py-0.5 text-[10px] text-neutral-500 hover:text-red-500 dark:text-neutral-400 rounded hover:bg-black/5 dark:hover:bg-white/10 transition-colors"
                        >
                          <Square size={10} strokeWidth={2.5} fill="currentColor" />
                          <span>{t.coworkStop}</span>
                        </button>
                      )}
                    </div>
                  )
                })()}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  )
}
