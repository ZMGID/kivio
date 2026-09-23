import { useEffect, useState, type ReactNode } from 'react'
import {
  isMessageNavigationEagerHydrate,
  useMessageNavigationEagerHydrate,
} from './messageNavigationStore'

type IdleWindow = Window & {
  requestIdleCallback?: (callback: () => void, options?: { timeout: number }) => number
  cancelIdleCallback?: (handle: number) => void
}

/**
 * Heavy chat content is allowed to hydrate after the surrounding text becomes
 * interactive. Keep the fallback's real layout while waiting for an idle slice:
 * an intrinsic-size substitute would corrupt virtual-row measurements offscreen.
 *
 * 例外：会话切换覆盖层 / 消息导航 settle 期间强制同步 hydrate —— 否则占位高度
 * 在跳转后才撑开，virtualizer 二次纠正就是「抽一下」。
 */
export function ChatHeavyIsland({
  children,
  fallback,
  minHeight = 96,
  delayMs = 120,
  eager = false,
}: {
  children: ReactNode
  fallback: ReactNode
  minHeight?: number
  delayMs?: number
  eager?: boolean
}) {
  // 模块 flag 给 useState 初始化器用：navigate 同帧 scrollToIndex 挂上的新岛
  // 还没走到 React 订阅，也必须直接以 hydrated 起步。
  const navigationEager = useMessageNavigationEagerHydrate()
  const shouldEager = eager || navigationEager || isMessageNavigationEagerHydrate()
  const [hydrated, setHydrated] = useState(() => eager || isMessageNavigationEagerHydrate())

  useEffect(() => {
    if (shouldEager) {
      setHydrated(true)
      return
    }
    let cancelled = false
    const idleWindow = window as IdleWindow
    let idleId: number | undefined
    let timeoutId: number | undefined
    const hydrate = () => {
      if (!cancelled) setHydrated(true)
    }

    if (idleWindow.requestIdleCallback && import.meta.env.MODE !== 'test') {
      idleId = idleWindow.requestIdleCallback(hydrate, { timeout: delayMs })
    } else {
      timeoutId = window.setTimeout(hydrate, delayMs)
    }

    return () => {
      cancelled = true
      if (idleId !== undefined) idleWindow.cancelIdleCallback?.(idleId)
      if (timeoutId !== undefined) window.clearTimeout(timeoutId)
    }
  }, [delayMs, shouldEager])

  return (
    <div
      data-chat-heavy-island="true"
      data-chat-heavy-hydrated={hydrated ? 'true' : 'false'}
      style={{
        // Virtualization already bounds mounted rows. Layout the cheap fallback
        // at its real height so scrolling cannot shrink a cached long code block
        // to minHeight and then expand it again during hydration.
        minHeight: hydrated ? undefined : minHeight,
      }}
    >
      {hydrated ? children : fallback}
    </div>
  )
}
