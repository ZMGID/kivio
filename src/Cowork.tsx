import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { flushSync } from 'react-dom'
import { X, Loader2, Copy, Check, Square, Image as ImageIcon, Sparkles, ArrowUp } from 'lucide-react'
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

const READY_W = 600
const ANSWERING_H = 420
const DRAG_THRESHOLD = 5

/**
 * Cowork 模式：单 webview 三态
 * - select：全屏遮罩 + hover 应用窗口高亮 + 标签 + 区域 drag + 底部装饰对话栏（支持纯文字直发）
 * - ready：小悬浮 600×72，左缩略 + 输入框
 * - answering：扩展 600×420，下半流式答案
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
  const [riseKey, setRiseKey] = useState(0)
  // frameKey：仅在截图后 setAnchor 完成时 +1，触发整窗 fade-in 遮掩 backdrop-filter 首帧白闪。
  // 与 riseKey 区分：ready→answering 时不应让整窗再 fade。
  const [frameKey, setFrameKey] = useState(0)
  // mode：从 hash 读取（#cowork 或 #cowork?mode=explain）。
  // 'explain' 时：截图完成后调 explainOpenAtAnchor 打开截图讲解大窗口而非进 ready 悬浮栏。
  // 'cowork'（默认）：现有 cowork 流程。
  const mode = useMemo<'cowork' | 'explain'>(() => {
    const m = window.location.hash.match(/[?&]mode=([^&]+)/)
    return m?.[1] === 'explain' ? 'explain' : 'cowork'
  }, [])

  const inputRef = useRef<HTMLInputElement>(null)
  const stageRef = useRef<Stage>('select')
  const imageIdRef = useRef('')
  const copyTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)

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

  // select 态进入时：刷新窗口起点 + 拉窗口列表 + 后端 dispatch cowork:reset
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

  // 全局 Esc：select 态取消 / 其他态关闭
  useEffect(() => {
    const handler = async (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      if (stageRef.current === 'answering' && streaming) {
        try { await api.coworkCancelStream() } catch (err) { console.error(err) }
        setStreaming(false)
        return
      }
      try { await api.coworkClose() } catch (err) { console.error(err) }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [streaming])

  // select 态切到其他应用 → 自动收起（避免遗留全屏遮罩）。
  // ready/answering 是正常使用态，用户点击其他应用看截图/输入是预期行为，不取消。
  useEffect(() => {
    const handleBlur = () => {
      if (stageRef.current === 'select') {
        void api.coworkClose()
      }
    }
    window.addEventListener('blur', handleBlur)
    return () => window.removeEventListener('blur', handleBlur)
  }, [])

  /** 把 webview client 坐标转成全局逻辑坐标（与 CGWindow bounds 同坐标系）。
   * 用 Tauri 的 outerPosition + scaleFactor 算的 winOrigin，绕开 macOS 上 window.screenX/Y 不可靠的问题。 */
  const clientToGlobal = (p: Point): Point => ({
    x: winOrigin.x + p.x,
    y: winOrigin.y + p.y,
  })

  /** 命中检测：找第一个包含该全局 logical 坐标的窗口（list 顺序大致 z-top → bottom） */
  const hitTest = (gp: Point): CoworkWindowInfo | null => {
    for (const w of windows) {
      if (gp.x >= w.x && gp.x < w.x + w.width && gp.y >= w.y && gp.y < w.y + w.height) {
        return w
      }
    }
    return null
  }

  // 拖动选区矩形（webview 坐标）
  const dragRect = useMemo(() => {
    if (!dragStart || !dragCurrent) return null
    const x = Math.min(dragStart.x, dragCurrent.x)
    const y = Math.min(dragStart.y, dragCurrent.y)
    const w = Math.abs(dragCurrent.x - dragStart.x)
    const h = Math.abs(dragCurrent.y - dragStart.y)
    return { x, y, width: w, height: h }
  }, [dragStart, dragCurrent])

  // hover window 高亮区（webview 坐标）
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
    // 未按下：hover 检测
    const gp = clientToGlobal(p)
    setHovered(hitTest(gp))
  }

  const handleCaptureWindow = async (info: CoworkWindowInfo) => {
    const result = await api.coworkCaptureWindow(info.id)
    if (!result.success || !result.imageId) {
      console.error('coworkCaptureWindow failed:', result.error)
      void enterSelect()
      return
    }
    const newId = result.imageId
    imageIdRef.current = newId

    // explain 模式：不进 ready 态，把图片交给 explain 大窗口；当前 cowork 窗口由后端 hide
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

    // cowork 模式：进 ready 悬浮栏
    // 先 hide 窗口避免 ready 态 h-screen w-screen 白色磨砂玻璃在窗口缩小前覆盖整屏（白闪）
    try { await getCurrentWindow().hide() } catch (err) { console.error(err) }
    flushSync(() => {
      setAppLabel(info.owner)
      setStage('ready')
    })
    void (async () => {
      try {
        const img = await api.explainReadImage(newId)
        if (img.success) setImagePreview(img.data ?? '')
      } catch (err) { console.error(err) }
    })()
    try {
      await api.coworkSetAnchor(
        Math.round(info.x),
        Math.round(info.y),
        Math.round(info.width),
        Math.round(info.height),
      )
    } catch (err) {
      console.error('setAnchor failed', err)
      // 兜底：anchor 失败时窗口已被 hide 但前端切到 ready 态，没法恢复 → 直接关闭
      try { await api.coworkClose() } catch { /* ignore */ }
      return
    }
    setRiseKey(k => k + 1)
    setFrameKey(k => k + 1)
    setTimeout(() => inputRef.current?.focus(), 80)
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
    const result = await api.coworkCaptureRegion(params)
    if (!result.success || !result.imageId) {
      console.error('coworkCaptureRegion failed:', result.error)
      void enterSelect()
      return
    }
    const newId = result.imageId
    imageIdRef.current = newId

    // explain 模式：交给 explain 大窗口
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

    // cowork 模式：进 ready 悬浮栏
    // 同 handleCaptureWindow：先 hide，避免全屏白闪
    try { await getCurrentWindow().hide() } catch (err) { console.error(err) }
    flushSync(() => {
      setAppLabel('')
      setStage('ready')
    })
    void (async () => {
      try {
        const img = await api.explainReadImage(newId)
        if (img.success) setImagePreview(img.data ?? '')
      } catch (err) { console.error(err) }
    })()
    try {
      await api.coworkSetAnchor(params.absoluteX, params.absoluteY, params.width, params.height)
    } catch (err) {
      console.error('setAnchor failed', err)
      try { await api.coworkClose() } catch { /* ignore */ }
      return
    }
    setRiseKey(k => k + 1)
    setFrameKey(k => k + 1)
    setTimeout(() => inputRef.current?.focus(), 80)
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
    if (stage === 'select') {
      try {
        await api.coworkPositionBottom(READY_W, ANSWERING_H, 110)
      } catch (err) { console.error(err) }
    } else {
      try { await api.coworkResize(READY_W, ANSWERING_H) } catch (err) { console.error(err) }
    }
    flushSync(() => {
      setStage('answering')
      setStreaming(true)
    })
    setRiseKey(k => k + 1)
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

  const handleClose = async () => {
    try {
      if (streaming) await api.coworkCancelStream()
      await api.coworkClose()
    } catch (err) { console.error(err) }
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

  // ====== select 态渲染 ======
  if (stage === 'select') {
    return (
      <div
        className="fixed inset-0 cursor-crosshair select-none"
        onMouseDown={handleMouseDown}
        onMouseMove={handleMouseMove}
        onMouseUp={handleMouseUp}
        data-tauri-drag-region="false"
      >
        {!hoverRect && !dragRect && <div className="absolute inset-0 bg-black/40" />}

        {/* hover 窗口高亮：品牌橙色边框 + 微弱橙色 glow + 黑色"挖洞"遮罩 */}
        {hoverRect && (
          <>
            <div
              className="absolute border-[2px] border-[#D97757] rounded-md"
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

        {/* 拖动选区高亮：与 hover 同款橙色 */}
        {dragRect && dragging && (
          <div
            className="absolute border-[2px] border-[#D97757] rounded-sm"
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

        {/* 顶部提示：按 dragging / hovered / idle 三态切换文案 */}
        <div className="absolute top-6 left-1/2 -translate-x-1/2 px-3 py-1.5 rounded-full bg-neutral-900/80 backdrop-blur text-white text-[12px] font-medium pointer-events-none">
          {dragging
            ? t.coworkSelectHintDrag
            : hovered
              ? t.coworkSelectHintHover.replace('{app}', hovered.owner)
              : t.coworkSelectHintIdle}
        </div>

        {/* 底部对话栏：与 Claude Desktop quick entry 同款。
            既支持先选窗口/区域截图再问，也支持直接输入纯文字提问（不截图）。
            explain 模式下隐藏：截图讲解必须有图，纯文字无意义。 */}
        {mode !== 'explain' && (
        <div
          className="absolute bottom-[110px] left-1/2 -translate-x-1/2 w-[680px] animate-cowork-rise"
          onMouseDown={(e) => e.stopPropagation()}
          onMouseMove={(e) => e.stopPropagation()}
          onMouseUp={(e) => e.stopPropagation()}
          onClick={(e) => e.stopPropagation()}
        >
          <div
            className="flex items-center gap-3 pl-5 pr-2 py-2 rounded-[18px] bg-white dark:bg-neutral-900 shadow-[0_18px_50px_-12px_rgba(0,0,0,0.4)] ring-1 ring-black/[0.04] dark:ring-white/[0.06] cursor-default"
            data-tauri-drag-region="false"
          >
            <Sparkles size={20} strokeWidth={1.75} className="shrink-0 text-[#D97757]" />
            <input
              ref={inputRef}
              autoFocus
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={(e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); void handleSend() } }}
              placeholder={t.coworkAskPlaceholder}
              className="flex-1 bg-transparent text-[16px] text-neutral-900 dark:text-white placeholder-neutral-500 dark:placeholder-neutral-400 focus:outline-none"
            />
            <button
              type="button"
              onClick={() => void handleSend()}
              disabled={!input.trim()}
              className={`shrink-0 w-10 h-10 rounded-xl flex items-center justify-center transition-all duration-150 active:scale-95 ${
                input.trim()
                  ? 'bg-[#D97757] hover:bg-[#C56646] hover:scale-105'
                  : 'bg-neutral-200 dark:bg-neutral-700 cursor-not-allowed'
              }`}
            >
              <ArrowUp
                size={18}
                strokeWidth={2.25}
                className={input.trim() ? 'text-white' : 'text-neutral-400 dark:text-neutral-500'}
              />
            </button>
          </div>
          {/* 键盘提示 */}
          <div className="mt-2 flex justify-center gap-3 text-[11px] text-white/70 pointer-events-none select-none">
            <span>↵ {t.coworkHintSend}</span>
            <span>·</span>
            <span>esc {t.coworkHintEsc}</span>
          </div>
        </div>
        )}
      </div>
    )
  }

  // ====== ready / answering 共用悬浮栏 ======
  // key={frame-${frameKey}}：截图后 setAnchor 完成时 +1，让根 div 重新挂载触发 fade-in，
  // 遮掩 webview show 那一帧 backdrop-filter 还没合成时的 frosted 白底闪。
  return (
    <div
      key={`frame-${frameKey}`}
      className="h-screen w-screen flex flex-col rounded-2xl overflow-hidden window-frosted relative animate-cowork-frame-fade"
    >
      {/* 顶部 drag bar */}
      <div className="absolute top-0 left-0 right-0 h-6 z-10" data-tauri-drag-region />

      {/* 关闭按钮 */}
      <button
        onClick={handleClose}
        className="absolute top-1.5 right-2 z-20 p-1 text-neutral-400 hover:text-neutral-700 dark:text-neutral-500 dark:hover:text-neutral-200 rounded-md hover:bg-black/5 dark:hover:bg-white/10 transition-all"
        title={t.coworkClose}
      >
        <X size={13} strokeWidth={2} />
      </button>

      {/* 输入栏 */}
      <div
        key={`bar-${riseKey}`}
        className="flex items-center gap-3 pl-4 pr-2 pt-2.5 pb-2 shrink-0 animate-cowork-rise"
      >
        {imagePreview || appLabel ? (
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
          onClick={() => void handleSend()}
          disabled={!input.trim() || stage === 'answering'}
          className={`shrink-0 w-10 h-10 rounded-xl flex items-center justify-center transition-all duration-150 active:scale-95 ${
            input.trim() && stage !== 'answering'
              ? 'bg-[#D97757] hover:bg-[#C56646] hover:scale-105'
              : 'bg-neutral-200 dark:bg-neutral-700 cursor-not-allowed'
          }`}
        >
          <ArrowUp
            size={18}
            strokeWidth={2.25}
            className={input.trim() && stage !== 'answering' ? 'text-white' : 'text-neutral-400 dark:text-neutral-500'}
          />
        </button>
      </div>

      {/* answering 答案区 */}
      {stage === 'answering' && (
        <div
          key={`ans-${riseKey}`}
          className="flex-1 overflow-y-auto custom-scrollbar px-3.5 pb-3 border-t border-black/[0.06] dark:border-white/[0.06] animate-cowork-rise"
        >
          <div className="pt-2.5 prose prose-sm dark:prose-invert max-w-none text-[13.5px] leading-7 text-neutral-800 dark:text-neutral-200">
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
  )
}
