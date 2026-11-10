import { useEffect, useLayoutEffect, useRef } from 'react'

const SHIFT_DURATION_MS = 320
const SHIFT_EASING = 'cubic-bezier(0.22, 1, 0.36, 1)' // --kv-ease-standard

type Rect = { left: number; width: number }

function currentTranslateX(element: HTMLElement): number {
  const transform = getComputedStyle(element).transform
  if (!transform || transform === 'none') return 0
  try {
    return new DOMMatrixReadOnly(transform).m41
  } catch {
    return 0
  }
}

/**
 * 侧栏 / 右侧面板开合时，主区内容从旧中心平滑移到新中心。
 *
 * 面板开合只切换一次布局（不逐帧改主区宽度，见 chat-01-main.css「左右侧面板开合」）。
 * 布局切换后，这里把主区当前可见视图用 transform 放回原中心，再动画回 0：阅读列不再
 * 瞬间跳动，且只动合成层，不触发聊天内容重排。
 *
 * - 只在面板可见性变化、且视图未同时切换时播放（切到设置页等由页面自己的入场负责）；
 * - 拖拽调宽、窗口缩放只更新记录的位置，不播放；
 * - 连续开合从当前视觉位置接着走，不回跳；
 * - 减少动态效果时不播放。
 *
 * @param row 承载「侧栏 | 主区 | 右侧面板」的行元素；主区宿主为其 `.chat-panel-shift-host` 子元素。
 * @param panels 面板可见性的签名，变化即视为一次开合。
 * @param view 当前视图；与 panels 同时变化时不播放。
 */
export function usePanelShiftMotion(row: HTMLElement | null, panels: string, view: string): void {
  const lastRectRef = useRef<Rect | null>(null)
  const lastPanelsRef = useRef(panels)
  const lastViewRef = useRef(view)
  const animationRef = useRef<Animation | null>(null)

  // 记录主区当前几何。拖拽调宽 / 窗口缩放都会触发，保证下一次开合的「旧中心」是新鲜的。
  useEffect(() => {
    const host = row?.querySelector<HTMLElement>(':scope > .chat-panel-shift-host')
    if (!host || typeof ResizeObserver === 'undefined') return
    const record = () => {
      const rect = host.getBoundingClientRect()
      lastRectRef.current = { left: rect.left, width: rect.width }
    }
    record()
    const observer = new ResizeObserver(record)
    observer.observe(host)
    return () => observer.disconnect()
  }, [row])

  useLayoutEffect(() => {
    const panelsChanged = lastPanelsRef.current !== panels
    const viewChanged = lastViewRef.current !== view
    lastPanelsRef.current = panels
    lastViewRef.current = view
    const host = row?.querySelector<HTMLElement>(':scope > .chat-panel-shift-host')
    if (!host) return
    const before = lastRectRef.current
    const rect = host.getBoundingClientRect()
    lastRectRef.current = { left: rect.left, width: rect.width }
    if (!panelsChanged || viewChanged || !before) return
    if (typeof window !== 'undefined' && window.matchMedia?.('(prefers-reduced-motion: reduce)').matches) return
    const target = host.querySelector<HTMLElement>(':scope > :not([hidden])')
    if (!target || typeof target.animate !== 'function') return

    let visualOffset = 0
    const running = animationRef.current
    if (running) {
      visualOffset = currentTranslateX(target)
      animationRef.current = null
      running.cancel()
    }
    const dx = before.left + before.width / 2 + visualOffset - (rect.left + rect.width / 2)
    if (Math.abs(dx) < 2) {
      host.classList.remove('is-panel-shifting')
      return
    }
    host.classList.add('is-panel-shifting')
    const animation = target.animate(
      [{ transform: `translateX(${dx}px)` }, { transform: 'translateX(0)' }],
      { duration: SHIFT_DURATION_MS, easing: SHIFT_EASING },
    )
    animationRef.current = animation
    const finish = () => {
      if (animationRef.current !== animation) return
      animationRef.current = null
      host.classList.remove('is-panel-shifting')
    }
    animation.addEventListener('finish', finish)
    animation.addEventListener('cancel', finish)
  }, [panels, row, view])

  useEffect(() => () => {
    animationRef.current?.cancel()
    animationRef.current = null
  }, [])
}
