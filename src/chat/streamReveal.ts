import { createContext, useEffect, useLayoutEffect, useRef, useState } from 'react'

/**
 * 回答是否仍在出字。由流式气泡（MessageBubble）提供；为 true 时 ChatMarkdown 把新到的文字
 * 按节奏逐步显示，而不是随网络包整块跳出。
 */
export const LiveRevealContext = createContext(false)

// 显示落后于到达的目标时长：积压文字大约在这段时间内显示完。按实际到达间隔自适应——
// 包越稀疏、越成块，节奏放得越缓，让两包之间也一直在出字，而不是「停—跳—停」。
const DEFAULT_LAG_MS = 200
const MIN_LAG_MS = 120
const MAX_LAG_MS = 700
// 叠加在按积压计算的速度之上的底速（字/毫秒）：尾巴不会越爬越慢，一包 300 字约半秒显示完。
const MIN_RATE = 0.1
// 一次性涌入大量文字（恢复、切回窗口、粘贴式输出）时不逐字追：直接跳到只剩这么多积压。
const MAX_BACKLOG = 4_000
const BACKLOG_KEEP = 600

/** 下一帧应显示到的长度。纯函数：速度 = 积压 / lagMs + 底速，积压越多走得越快。 */
export function nextRevealLength(shown: number, target: number, dtMs: number, lagMs: number): number {
  const backlog = target - shown
  if (backlog <= 0) return target
  if (backlog > MAX_BACKLOG) return target - BACKLOG_KEEP
  const rate = backlog / Math.max(1, lagMs) + MIN_RATE
  return Math.min(target, shown + Math.max(1, Math.round(rate * Math.max(0, dtMs))))
}

/** 两次显示更新的最小间隔。长文每次重渲更贵，降低更新频率但保持匀速。 */
export function revealTickMs(length: number): number {
  return length < 8_000 ? 0 : length < 24_000 ? 33 : length < 60_000 ? 50 : 80
}

/** 根据相邻两次到达的间隔（指数平均）给出显示落后时长。 */
export function revealLagMs(arrivalIntervalMs: number | null): number {
  if (arrivalIntervalMs == null) return DEFAULT_LAG_MS
  return Math.min(MAX_LAG_MS, Math.max(MIN_LAG_MS, arrivalIntervalMs * 1.3))
}

/** 不在代理对中间截断，否则半个 emoji 会显示成乱码。 */
function safeEnd(text: string, end: number): number {
  if (end <= 0 || end >= text.length) return Math.max(0, Math.min(end, text.length))
  const code = text.charCodeAt(end - 1)
  return code >= 0xd800 && code <= 0xdbff ? end + 1 : end
}

let pacingEnabled = import.meta.env.MODE !== 'test'
/** 组件测试默认关闭节奏，断言立即可见的文字；节奏本身的测试显式打开。 */
export function setRevealPacingForTest(enabled: boolean): void {
  pacingEnabled = enabled
}

function prefersReducedMotion(): boolean {
  return typeof window !== 'undefined' && window.matchMedia?.('(prefers-reduced-motion: reduce)').matches === true
}

/**
 * 返回当前应显示的文字。
 * - 挂载时已有的文字立即显示（打开进行中的对话不会重新「打一遍字」）；
 * - live 期间只对之后追加的部分按节奏显示；
 * - 不再 live（完成、停止、出错）或内容被整段替换时，立即显示全文；
 * - 减少动态效果时不做节奏。
 */
export function useRevealedText(text: string, live: boolean): string {
  const pacing = live && pacingEnabled && !prefersReducedMotion()
  const [shownLength, setShownLength] = useState(text.length)
  const visibleRef = useRef(text)
  const textRef = useRef(text)
  const shownRef = useRef(text.length)
  const frameRef = useRef(0)
  const lastTickRef = useRef(0)
  const arrivalRef = useRef<{ at: number; length: number; intervalMs: number | null }>({ at: 0, length: text.length, intervalMs: null })

  // 只有前缀增长才按节奏显示；整段替换（重试丢弃、改写段落）直接显示新文。已经显示过的
  // 文字永不回缩。
  const extendsVisible = text.startsWith(visibleRef.current)
  const end = pacing && extendsVisible
    ? safeEnd(text, Math.max(visibleRef.current.length, Math.min(shownLength, text.length)))
    : text.length
  const visible = end >= text.length ? text : text.slice(0, end)
  textRef.current = text

  useLayoutEffect(() => {
    visibleRef.current = visible
    shownRef.current = visible.length
  })

  // 记录到达节奏：相邻两次增长的间隔做指数平均。
  useEffect(() => {
    const arrival = arrivalRef.current
    if (text.length <= arrival.length) {
      arrival.length = text.length
      return
    }
    const now = performance.now()
    if (arrival.at > 0) {
      const interval = Math.min(2_000, now - arrival.at)
      arrival.intervalMs = arrival.intervalMs == null ? interval : arrival.intervalMs * 0.7 + interval * 0.3
    }
    arrival.at = now
    arrival.length = text.length
  }, [text.length])

  useEffect(() => {
    if (!pacing || shownRef.current >= text.length || frameRef.current) return
    const step = (now: number) => {
      frameRef.current = 0
      const target = textRef.current.length
      if (shownRef.current >= target) return
      const last = lastTickRef.current || now - 16
      if (now - last >= revealTickMs(target)) {
        const next = nextRevealLength(shownRef.current, target, now - last, revealLagMs(arrivalRef.current.intervalMs))
        lastTickRef.current = now
        shownRef.current = next
        setShownLength(next)
      }
      if (shownRef.current < textRef.current.length) frameRef.current = requestAnimationFrame(step)
    }
    lastTickRef.current = performance.now()
    frameRef.current = requestAnimationFrame(step)
  }, [pacing, text.length])

  useEffect(() => () => {
    if (frameRef.current) cancelAnimationFrame(frameRef.current)
    frameRef.current = 0
  }, [])

  return visible
}
