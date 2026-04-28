import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { flushSync } from 'react-dom'
import { Loader2, Copy, Check, Square, Image as ImageIcon, Sparkles, ArrowUp } from 'lucide-react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { api, type CoworkStreamPayload, type CoworkWindowInfo } from './api/tauri'
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

const READY_W = 600
const READY_BAR_H = 56            // 对话栏单行高度
const ANSWER_H = 360               // answering 时答案区高度（对话栏下方）
const SELECT_W = 680
const SELECT_BOTTOM_OFFSET = 110   // 距 webview 底部
const DRAG_THRESHOLD = 5
const TRANSITION_MS = 380

/**
 * 计算 select 态对话栏在 webview 内的位置（webview 全屏，所以用 viewport 大小）
 */
const computeSelectBar = (): BarRect => ({
  x: Math.round(window.innerWidth / 2 - SELECT_W / 2),
  y: Math.round(window.innerHeight - SELECT_BOTTOM_OFFSET - READY_BAR_H),
  width: SELECT_W,
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
  const [answer, setAnswer] = useState('')
  const [streaming, setStreaming] = useState(false)
  const [copied, setCopied] = useState(false)
  const [lang, setLang] = useState<Lang>('zh')
  const [barRect, setBarRect] = useState<BarRect>(() => computeSelectBar())
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

  const inputRef = useRef<HTMLInputElement>(null)
  const stageRef = useRef<Stage>('select')
  const imageIdRef = useRef('')
  const copyTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  // capture 期间 macOS screencapture 可能短暂让 cowork webview 失焦 → 触发 blur 误关闭。
  // 这个 ref 标记"截图进行中"，blur handler 看到就跳过。
  const capturingRef = useRef(false)

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
    setAnswer('')
    setStreaming(false)
    imageIdRef.current = ''
    setMode(readMode())
    setBarRect(computeSelectBar())
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

  // viewport resize：select 态重算底部位置
  useEffect(() => {
    const onResize = () => {
      if (stageRef.current === 'select') setBarRect(computeSelectBar())
    }
    window.addEventListener('resize', onResize)
    return () => window.removeEventListener('resize', onResize)
  }, [])

  // 监听 cowork-stream 事件追加 delta
  useEffect(() => {
    let unlisten: (() => void) | undefined
    api.onCoworkStream((payload: CoworkStreamPayload) => {
      if (payload.imageId !== imageIdRef.current) return
      if (payload.done) {
        setStreaming(false)
        return
      }
      if (payload.delta) setAnswer(prev => prev + payload.delta)
    }).then((dispose) => {
      unlisten = dispose
    }).catch(err => console.error(err))
    return () => { unlisten?.() }
  }, [])

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
      setAnswer('')
      setStreaming(false)
      setBarRect(computeSelectBar())
      setCapturedFrame(null)
      setBarIntro(false)
    })
    imageIdRef.current = ''
  }, [])

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

  /** 截图后 cowork 默认模式：调 resolveAnchor 拿目标 webview 内坐标，setBarRect → CSS transition 飞入 */
  const flyBarToAnchor = async (
    anchorAbsX: number,
    anchorAbsY: number,
    anchorW: number,
    anchorH: number,
    label: string,
  ) => {
    let target: { targetX: number; targetY: number } | null = null
    try {
      target = await api.coworkResolveAnchor(anchorAbsX, anchorAbsY, anchorW, anchorH)
    } catch (err) {
      console.error('coworkResolveAnchor failed', err)
    }
    flushSync(() => {
      setAppLabel(label)
      if (target) {
        setBarRect({ x: Math.round(target.targetX), y: Math.round(target.targetY), width: READY_W })
      }
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
    const question = input
    const id = imageIdRef.current
    setInput('')
    setAnswer('')
    flushSync(() => {
      setStage('answering')
      setStreaming(true)
    })
    try {
      const result = await api.coworkAsk(id || '', question)
      if (result.success && result.response && answer.length === 0) {
        setAnswer(result.response)
      } else if (!result.success) {
        setAnswer(`${t.coworkError}: ${result.error}`)
      }
    } catch (err) {
      setAnswer(`${t.coworkError}: ${err instanceof Error ? err.message : String(err)}`)
    } finally {
      setStreaming(false)
    }
  }

  const handleStop = async () => {
    try { await api.coworkCancelStream() } catch (err) { console.error(err) }
    setStreaming(false)
  }

  const handleCopy = async () => {
    if (!answer) return
    const ok = await copyToClipboard(answer)
    if (!ok) return
    setCopied(true)
    if (copyTimeoutRef.current) clearTimeout(copyTimeoutRef.current)
    copyTimeoutRef.current = setTimeout(() => setCopied(false), 2000)
  }

  useEffect(() => () => {
    if (copyTimeoutRef.current) clearTimeout(copyTimeoutRef.current)
  }, [])

  // ====== 单一渲染 ======
  const showThumb = stage !== 'select' && (imagePreview || appLabel)
  const sendDisabled = !input.trim() || stage === 'answering'
  // 是否显示底部对话栏：cowork 默认模式始终显示；explain 模式仅 select 态隐藏（截图后由 explain 大窗口接管）
  const showBar = mode !== 'explain'

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
              disabled={stage === 'answering'}
              placeholder={t.coworkAskPlaceholder}
              className="flex-1 bg-transparent text-[16px] text-neutral-900 dark:text-white placeholder-neutral-500 dark:placeholder-neutral-400 focus:outline-none disabled:opacity-60"
            />
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

          {/* answer 区：absolute 展开在对话栏下方 8px，answering 态显示 */}
          <div
            className="absolute left-0 right-0 rounded-2xl overflow-hidden window-frosted transition-all ease-out"
            style={{
              top: 'calc(100% + 8px)',
              height: stage === 'answering' ? ANSWER_H : 0,
              opacity: stage === 'answering' ? 1 : 0,
              transitionDuration: `${TRANSITION_MS}ms`,
              pointerEvents: stage === 'answering' ? 'auto' : 'none',
            }}
          >
            {stage === 'answering' && (
              <div className="h-full overflow-y-auto custom-scrollbar px-3.5 py-3">
                <div className="prose prose-sm dark:prose-invert max-w-none text-[13.5px] leading-7 text-neutral-800 dark:text-neutral-200">
                  {answer ? (
                    <ReactMarkdown remarkPlugins={[remarkMath]} rehypePlugins={[rehypeKatex]}>
                      {answer}
                    </ReactMarkdown>
                  ) : streaming ? (
                    <div className="flex items-center gap-2 text-neutral-500 dark:text-neutral-400">
                      <Loader2 className="animate-spin" size={14} />
                      <span className="text-[12px]">{t.coworkAsking}</span>
                    </div>
                  ) : null}
                </div>
                {answer && (
                  <div className="flex items-center gap-1 mt-2.5">
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
                )}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  )
}
