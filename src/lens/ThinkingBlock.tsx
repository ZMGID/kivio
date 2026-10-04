import { useEffect, useMemo, useRef, useState } from 'react'
import { Loader2, Brain, ChevronDown } from 'lucide-react'
import { estimateTokens, formatTokens } from '../utils/tokens'

/** 思维链区块（Claude Code 风格）：默认折叠，header 显示耗时 + token 估算。点击展开/收起。 */
export function ThinkingBlock({
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
    <div className="not-prose mb-2 rounded-lg border border-neutral-900/[0.06] bg-neutral-900/[0.025]">
      <button
        type="button"
        onClick={() => setOpen(o => !o)}
        className="w-full flex items-center gap-1.5 px-2.5 py-1.5 text-[11.5px] text-neutral-500 dark:text-neutral-400 hover:text-neutral-700 transition-colors"
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
