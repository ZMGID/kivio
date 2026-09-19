import { useCallback, useEffect, useLayoutEffect, useState } from 'react'
import { measureChatSurface } from '../chatPerformanceProbe'
import { chatWindowMinSize } from '../chatWindowGeometry'
import {
  getRememberedChatSidebarCollapsed,
  getRememberedSidebarWidth,
  rememberChatSidebarCollapsed,
  rememberChatSize,
  rememberSidebarWidth,
} from '../persistence'
import { isTauriRuntime } from '../utils'

/**
 * 左侧栏折叠 / 宽度：localStorage 往返、CSS 变量、以及（非最大化时）窗口 min-size。
 */
export function useSidebarLayout() {
  const [collapsed, setCollapsedState] = useState(() => getRememberedChatSidebarCollapsed())
  const [width, setWidthState] = useState(() => getRememberedSidebarWidth())

  const setCollapsed = useCallback((next: boolean) => {
    const finish = measureChatSurface(
      'sidebar-collapse',
      document.querySelector('.chat-window-shell'),
      next ? 'collapsed' : 'expanded',
    )
    setCollapsedState(next)
    rememberChatSidebarCollapsed(next)
    if (typeof requestAnimationFrame === 'function') {
      requestAnimationFrame(() => requestAnimationFrame(finish))
    } else {
      finish()
    }
  }, [])

  const collapse = useCallback(() => {
    setCollapsed(true)
  }, [setCollapsed])

  const setWidth = useCallback((nextWidth: number) => {
    setWidthState(nextWidth)
    rememberSidebarWidth(nextWidth)
  }, [])

  useLayoutEffect(() => {
    const shell = document.querySelector('.chat-window-shell')
    if (shell instanceof HTMLElement) {
      shell.style.setProperty('--chat-sidebar-width', `${width}px`)
    }
  }, [width])

  useEffect(() => {
    if (!isTauriRuntime()) return
    let cancelled = false
    void (async () => {
      const { getCurrentWindow } = await import('@tauri-apps/api/window')
      const { LogicalSize } = await import('@tauri-apps/api/dpi')
      const min = chatWindowMinSize(collapsed, width)
      const win = getCurrentWindow()
      // 最大化/全屏时不要动 min-size 或 size：Windows 上 setMinSize 会触发重排、把最大化状态取消掉
      // （表现为切换侧边栏后窗口退出最大化）。尺寸约束对铺满屏幕的窗口也没意义，等恢复到可调窗口再应用。
      if ((await win.isMaximized()) || (await win.isFullscreen())) return
      if (cancelled) return
      await win.setMinSize(new LogicalSize(min.width, min.height))
      if (cancelled) return

      if (!collapsed) {
        const scaleFactor = await win.scaleFactor()
        const size = await win.innerSize()
        const logical = size.toLogical(scaleFactor)
        if (logical.width < min.width) {
          const nextHeight = Math.max(logical.height, min.height)
          await win.setSize(new LogicalSize(min.width, nextHeight))
          rememberChatSize(min.width, nextHeight)
        }
      }
    })().catch((err) => {
      console.error('[Chat] Failed to update window min size:', err)
    })

    return () => {
      cancelled = true
    }
  }, [collapsed, width])

  return { collapsed, width, setCollapsed, collapse, setWidth }
}
