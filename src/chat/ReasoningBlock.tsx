import { useEffect, useMemo, useRef, useState } from 'react'
import { ChevronDown, Loader2 } from 'lucide-react'

const COLLAPSE_LINE_LIMIT = 3
const CHARS_PER_LINE = 60

function reasoningExceedsLineLimit(text: string): boolean {
  const trimmed = text.trim()
  if (!trimmed) return false
  if (trimmed.split(/\r?\n/).length > COLLAPSE_LINE_LIMIT) return true
  return Math.ceil(trimmed.length / CHARS_PER_LINE) > COLLAPSE_LINE_LIMIT
}

/** 折叠时只展示末尾三行（或尾部字符），流式更新始终跟最新内容 */
function collapsedReasoningPreview(text: string): { preview: string; truncated: boolean } {
  const trimmed = text.trimEnd()
  if (!trimmed) return { preview: '', truncated: false }
  const lines = trimmed.split(/\r?\n/)
  if (lines.length > COLLAPSE_LINE_LIMIT) {
    return {
      preview: lines.slice(-COLLAPSE_LINE_LIMIT).join('\n'),
      truncated: true,
    }
  }
  const maxChars = COLLAPSE_LINE_LIMIT * CHARS_PER_LINE
  if (trimmed.length > maxChars) {
    return { preview: trimmed.slice(-maxChars), truncated: true }
  }
  return { preview: trimmed, truncated: false }
}

type ReasoningBlockProps = {
  reasoning: string
  /** 思维链正在流式写入 */
  streaming?: boolean
}

export function ReasoningBlock({ reasoning, streaming = false }: ReasoningBlockProps) {
  const collapsible = useMemo(() => reasoningExceedsLineLimit(reasoning), [reasoning])
  const collapsedPreview = useMemo(
    () => collapsedReasoningPreview(reasoning),
    [reasoning],
  )
  const [open, setOpen] = useState(false)
  const [contentPulse, setContentPulse] = useState(false)
  const userExpandedRef = useRef(false)

  const showCollapsed = collapsible && !open

  useEffect(() => {
    if (!streaming) return
    setContentPulse(true)
    const timer = window.setTimeout(() => setContentPulse(false), 220)
    return () => window.clearTimeout(timer)
  }, [reasoning, streaming])

  useEffect(() => {
    if (!streaming && collapsible && !userExpandedRef.current) {
      setOpen(false)
    }
  }, [streaming, collapsible])

  const titleClass =
    'mb-1 flex w-full items-center gap-1 text-left text-[11px] font-medium text-neutral-400 transition-colors dark:text-neutral-500'
  const bodyClass = [
    'whitespace-pre-wrap text-sm leading-relaxed text-neutral-400 dark:text-neutral-500',
    streaming ? 'opacity-95' : 'opacity-90',
    contentPulse ? 'reasoning-stream-tail' : '',
  ].join(' ')

  const handleToggle = () => {
    userExpandedRef.current = true
    setOpen((value) => !value)
  }

  return (
    <section
      aria-label="思考过程"
      className={`mb-3 border-l pl-3 transition-colors duration-300 ${
        streaming
          ? 'border-neutral-300 dark:border-neutral-600'
          : 'border-neutral-200 dark:border-neutral-700'
      }`}
    >
      {collapsible ? (
        <button
          type="button"
          onClick={handleToggle}
          className={`${titleClass} hover:text-neutral-600 dark:hover:text-neutral-300`}
          aria-expanded={open}
          data-tauri-drag-region="false"
        >
          {streaming ? (
            <Loader2 size={11} strokeWidth={2} className="shrink-0 animate-spin" />
          ) : null}
          <span>思考过程</span>
          {streaming && showCollapsed ? (
            <span className="font-normal text-neutral-400 dark:text-neutral-500">· 生成中</span>
          ) : null}
          <ChevronDown
            size={12}
            strokeWidth={2}
            className={`ml-auto shrink-0 transition-transform duration-300 ${open ? 'rotate-180' : ''}`}
          />
        </button>
      ) : (
        <div className={titleClass}>
          {streaming ? (
            <Loader2 size={11} strokeWidth={2} className="shrink-0 animate-spin" />
          ) : null}
          <span>思考过程</span>
          {streaming ? (
            <span className="font-normal text-neutral-400 dark:text-neutral-500">· 生成中</span>
          ) : null}
        </div>
      )}

      <div className={bodyClass}>
        {showCollapsed && collapsedPreview.truncated ? (
          <span className="mr-0.5 opacity-50">…</span>
        ) : null}
        {showCollapsed ? collapsedPreview.preview : reasoning}
      </div>
    </section>
  )
}
