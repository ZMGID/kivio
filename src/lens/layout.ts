import type { BarRect, Metrics } from './types'

export const READY_BAR_H = 56
export const ANCHOR_GAP = 12
export const DRAG_THRESHOLD = 5
export const TRANSITION_MS = 380
export const SELECT_REVEAL_DELAY_MS = 80
// 浮动卡四周留白：给 lens-floating-surface 的投影留出渲染空间。
// Windows 走 SetWindowRgn 把全屏覆盖裁成 card 矩形再外扩这个值，=0 时投影会被整圈裁掉。
export const FLOATING_PADDING = 24
export const FLOATING_GAP = 8

// macOS 上 lens_set_floating 走的是 set_position + set_size,会真的把 OS 窗口搬到 (x, y)。
// Windows 上走 SetWindowRgn 只裁剪可见区域,窗口本身始终全屏。
// 两边对 barRect 的语义因此不同:macOS rebase 后 barRect 必须是窗口内坐标 (0,0),
// Windows 仍然是屏幕(全屏窗口本地)坐标 (finalX, finalY)。
export const isMacPlatform = typeof navigator !== 'undefined' && /Mac|iPhone|iPad|iPod/i.test(navigator.userAgent)

export const clamp = (value: number, min: number, max: number) => Math.max(min, Math.min(max, value))

/** 多屏适配：基于当前 viewport 算"比例 + 上下限"，不同分辨率/屏幕大小都能落到舒适区间。 */
export const computeMetrics = (vw: number, vh: number): Metrics => ({
  READY_W: Math.round(Math.max(420, Math.min(560, vw * 0.34))),
  SELECT_W: Math.round(Math.max(440, Math.min(640, vw * 0.42))),
  ANSWER_H: Math.round(Math.max(220, Math.min(480, vh * 0.45))),
  SELECT_BOTTOM_OFFSET: Math.round(Math.max(80, Math.min(160, vh * 0.13))),
})

/** chat 模式截图后输入栏宽度：与 select 态一致，避免缩略图/应用名挤占后发送按钮溢出。 */
export const computeChatBarWidth = (m: Metrics) => m.SELECT_W

type AnchoredBarParams = {
  viewportWidth: number
  viewportHeight: number
  anchorX: number
  anchorY: number
  anchorWidth: number
  anchorHeight: number
  barWidth: number
  sideContentHeight: number
}

/**
 * 计算截图后的输入栏位置：优先右、左，横向都放不下时回退到下、上。
 *
 * 横向放置时为后续答案区预留 `sideContentHeight`；上下回退只要求输入栏本身
 * 完整位于选区外，避免大选区把 ready 输入栏压回截图内部。
 */
export const computeAnchoredBar = ({
  viewportWidth,
  viewportHeight,
  anchorX,
  anchorY,
  anchorWidth,
  anchorHeight,
  barWidth,
  sideContentHeight,
}: AnchoredBarParams): BarRect => {
  const margin = 16
  const maxX = Math.max(margin, viewportWidth - barWidth - margin)
  const maxSideY = Math.max(margin, viewportHeight - sideContentHeight - margin)
  const centeredX = clamp(anchorX + anchorWidth / 2 - barWidth / 2, margin, maxX)
  const centeredSideY = clamp(
    anchorY + anchorHeight / 2 - READY_BAR_H / 2,
    margin,
    maxSideY,
  )

  const rightX = anchorX + anchorWidth + ANCHOR_GAP
  if (rightX + barWidth <= viewportWidth - margin) {
    return { x: rightX, y: centeredSideY, width: barWidth }
  }

  const leftX = anchorX - ANCHOR_GAP - barWidth
  if (leftX >= margin) {
    return { x: leftX, y: centeredSideY, width: barWidth }
  }

  const belowY = anchorY + anchorHeight + ANCHOR_GAP
  if (belowY + READY_BAR_H <= viewportHeight - margin) {
    return { x: centeredX, y: belowY, width: barWidth }
  }

  const aboveY = anchorY - ANCHOR_GAP - READY_BAR_H
  if (aboveY >= margin) {
    return { x: centeredX, y: aboveY, width: barWidth }
  }

  // 选区几乎铺满整个 viewport 时不存在完全无交叠的位置；保留在可用空间较大的一侧。
  const spaceBelow = viewportHeight - (anchorY + anchorHeight)
  const spaceAbove = anchorY
  const fallbackY = spaceBelow >= spaceAbove
    ? Math.max(margin, viewportHeight - READY_BAR_H - margin)
    : margin
  return { x: centeredX, y: fallbackY, width: barWidth }
}

/** 计算 select 态对话栏在 webview 内的位置（webview 全屏，所以用 viewport 大小） */
export const computeSelectBar = (vw: number, vh: number, m: Metrics): BarRect => ({
  x: Math.round(vw / 2 - m.SELECT_W / 2),
  y: Math.round(vh - m.SELECT_BOTTOM_OFFSET - READY_BAR_H),
  width: m.SELECT_W,
})
