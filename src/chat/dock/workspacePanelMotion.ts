// 工作区面板浮层的进出场：复用全局 chat-motion-popover / -out / -menu-cascade，reduced-motion 由全局兜底。
import { useEffect, useState } from 'react'

/** 退场动画（--kv-dur-fast = 150ms）播完再卸载。 */
const EXIT_MS = 170

/** open 变 false 后保留挂载一个退场动画的时长。 */
export function usePresence(open: boolean, exitMs = EXIT_MS) {
  const [present, setPresent] = useState(open)
  useEffect(() => {
    if (open) {
      setPresent(true)
      return
    }
    const timer = window.setTimeout(() => setPresent(false), exitMs)
    return () => window.clearTimeout(timer)
  }, [open, exitMs])
  return { present: open || present, closing: !open && present }
}

export function motionClass(closing: boolean) {
  return closing ? 'chat-motion-popover-out pointer-events-none' : 'chat-motion-popover chat-motion-menu-cascade'
}
