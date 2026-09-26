import { useContext, useEffect, useMemo, useRef, useState } from 'react'
import { ChevronRight } from 'lucide-react'
import { ChatDisclosureBody } from './ChatDisclosureBody'
import { ReasoningPreviewContext } from './reasoningPreview'

type ReasoningBlockProps = {
  reasoning: string
  /** 思维链正在流式写入 */
  streaming?: boolean
  /** 当前轮仍在生成，重挂时恢复最近思考的预览。 */
  previewActive?: boolean
  /** 用户主动展开全文时，让包含它的过程组保留阅读状态。 */
  onExpand?: () => void
  /** 已知思考耗时，用于流式完成后继续展示 */
  durationMs?: number | null
}

function formatThinkingDuration(durationMs: number | null | undefined): string {
  if (durationMs == null || !Number.isFinite(durationMs) || durationMs <= 0) return ''
  const totalSeconds = Math.max(1, Math.round(durationMs / 1000))
  if (totalSeconds < 60) return `${totalSeconds}s`
  const minutes = Math.floor(totalSeconds / 60)
  const seconds = totalSeconds % 60
  return seconds > 0 ? `${minutes}m ${seconds}s` : `${minutes}m`
}

export function ReasoningBlock({ reasoning, streaming = false, previewActive = streaming, onExpand, durationMs = null }: ReasoningBlockProps) {
  const collapsible = reasoning.trim().length > 0
  const previewEnabled = useContext(ReasoningPreviewContext)
  const [expanded, setExpanded] = useState(false)
  const [sawLivePreview, setSawLivePreview] = useState(false)
  const [liveDurationMs, setLiveDurationMs] = useState(0)
  const durationStartedAtRef = useRef<number | null>(null)
  const scrollRef = useRef<HTMLDivElement>(null)
  const contentRef = useRef<HTMLDivElement>(null)
  const previewRef = useRef<HTMLSpanElement>(null)
  const previewTextRef = useRef<HTMLSpanElement>(null)
  const { visibleReasoning, latestLine } = useMemo(() => {
    const text = reasoning.trimEnd()
    const lineStart = Math.max(text.lastIndexOf('\n'), text.lastIndexOf('\r')) + 1
    return { visibleReasoning: text, latestLine: text.slice(lineStart).trim() }
  }, [reasoning])

  // Remember only a preview shown in this mounted conversation. Loading old
  // history stays collapsed; finishing a live thought must not shrink the row.
  if (previewActive && collapsible && previewEnabled && !sawLivePreview) setSawLivePreview(true)
  const open = collapsible && previewEnabled && expanded
  const showPreview = collapsible && previewEnabled && !open && (previewActive || sawLivePreview)

  useEffect(() => {
    if (!showPreview) return
    const viewport = previewRef.current
    const text = previewTextRef.current
    if (!viewport || !text) return
    // Follow the latest characters, including when the available width changes.
    // Only the current line is mounted here; full text is mounted on expansion.
    const observer = new ResizeObserver(() => {
      const end = Math.max(0, viewport.scrollWidth - viewport.clientWidth)
      if (Math.abs(viewport.scrollLeft - end) > 1) viewport.scrollLeft = end
    })
    observer.observe(viewport)
    observer.observe(text)
    return () => observer.disconnect()
  }, [showPreview])

  useEffect(() => {
    if (!streaming || !collapsible) {
      durationStartedAtRef.current = null
      return
    }

    if (durationStartedAtRef.current == null) {
      durationStartedAtRef.current = Date.now() - (durationMs ?? 0)
    }

    const updateDuration = () => {
      const startedAt = durationStartedAtRef.current
      if (startedAt == null) return
      setLiveDurationMs(Date.now() - startedAt)
    }
    updateDuration()
    const interval = window.setInterval(updateDuration, 1000)
    return () => window.clearInterval(interval)
  }, [collapsible, durationMs, streaming])

  useEffect(() => {
    if (!open) return
    const scrollBox = scrollRef.current
    const content = contentRef.current
    if (!scrollBox || !content) return
    let following = true
    let lastHeight = -1
    const trackScroll = () => {
      following = scrollBox.scrollHeight - scrollBox.scrollTop - scrollBox.clientHeight <= 24
    }
    // ResizeObserver runs after layout. Only newly occupied lines need a scroll
    // write, not every token or every width-only notification. Completion keeps
    // this observer (and the reader's follow intent) attached to the same node.
    const observer = new ResizeObserver(([entry]) => {
      const height = entry.borderBoxSize?.[0]?.blockSize ?? entry.contentRect.height
      const grew = height > lastHeight
      lastHeight = height
      if (grew && following) scrollBox.scrollTop = scrollBox.scrollHeight
    })
    observer.observe(content)
    scrollBox.addEventListener('scroll', trackScroll, { passive: true })
    return () => {
      observer.disconnect()
      scrollBox.removeEventListener('scroll', trackScroll)
    }
  }, [open])

  const titleClass =
    'mb-1 flex h-6 w-full min-w-0 items-center gap-2 text-left text-[11.5px] font-medium text-neutral-700 transition-colors dark:text-neutral-200'
  const thinkingDuration = formatThinkingDuration(durationMs ?? liveDurationMs)
  const titleText = streaming ? 'Thinking…' : 'Thought'
  const label = <span className="inline-flex shrink-0 items-baseline gap-1.5 whitespace-nowrap">
    <span className={streaming ? 'reasoning-shimmer-text' : undefined}>{titleText}</span>
    {thinkingDuration && <span className="text-[11px] font-normal text-neutral-400 dark:text-neutral-500">{thinkingDuration}</span>}
  </span>

  return (
    <section
      aria-label="Thinking"
      className="mb-3"
    >
      {collapsible ? (
        <button
          type="button"
          onClick={() => {
            setExpanded(!open)
            if (!open) onExpand?.()
          }}
          className={`${titleClass} hover:text-neutral-900 dark:hover:text-neutral-50`}
          aria-expanded={open}
          title={open ? '收起完整思考' : '展开完整思考'}
          data-chat-disclosure
          data-tauri-drag-region="false"
        >
          {label}
          {showPreview && <>
            <span aria-hidden="true" className="shrink-0 text-neutral-400">·</span>
            <span ref={previewRef} data-testid="reasoning-preview" className="reasoning-line-preview">
              <span ref={previewTextRef} className="reasoning-line-text">{latestLine}</span>
            </span>
          </>}
          <ChevronRight size={12} aria-hidden="true" className={`shrink-0 text-neutral-400 ${open ? 'rotate-90' : ''}`} />
        </button>
      ) : (
        <div className={titleClass}>
          {label}
        </div>
      )}

      <ChatDisclosureBody open={open} animate={previewEnabled}>
        {() => collapsible && (
          <div
            ref={scrollRef}
            data-testid="reasoning-scroll"
            className="reasoning-scroll-box custom-scrollbar"
          >
            <div ref={contentRef} data-testid="reasoning-text" className="reasoning-plain-text">
              {visibleReasoning}
            </div>
          </div>
        )}
      </ChatDisclosureBody>
    </section>
  )
}
