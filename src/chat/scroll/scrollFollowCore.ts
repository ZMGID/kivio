// 底部钉住的滚动跟随核心（移植自参考实现 LiveAgent scrollFollowCore）。
//
// 放弃"猜这次滚动是不是用户"的思路，改成：
// - 只在明确的用户输入时解除跟随（滚轮上滚、触摸拖动、历史键、真实指针拖动）；
//   裸滚动帧（程序钉底回声、virtua 重测量、DPR 取整）永不解除跟随。
// - 靠位置驱动重新贴底：落到物理底部、或在手势 latch 内向下到达重排区，才重新跟随。
// - 跟随中出现 gap 用"再钉一次"纠正，而不是判定为离开。
// - ResizeObserver（contentGrowth）不改变跟随状态，只在跟随时钉底 —— 内容增长会撑开
//   gap 但没有用户输入，若当成"滚走"会在流式刷新时撕掉刚恢复的跟随。
//
// 纯函数、无 DOM 依赖：useScrollFollow 收集每次事件的事实并执行 pin 效果。
//
// 相比参考实现新增 "release" 事件：Kivio 的消息导航器点击跳转到上方消息时，需要主动
// 脱离跟随，否则跟随中的纠正器会把视口又钉回底部。

// "在底部"的容差。分数 devicePixelRatio（Windows 125%/150% 缩放、缩放的 webview）会把
// scrollTop 钉在 scrollHeight - clientHeight 前 1-3px，2px 阈值正好卡在边界上永远无法重贴。
export const BOTTOM_ATTACH_THRESHOLD_PX = 8

// 底部预留空白带：用户自然停在"底部"时会距物理底部几十 px，手势 latch 内向下到达此区即算回到底部。
export const BOTTOM_REATTACH_ZONE_PX = 192

// gap 在此 slop 内的抖动是布局噪声（虚拟列表测量补偿、DPR 取整），不算滚动方向。
export const DIRECTION_SLOP_PX = 1

// 指针按下移动超过此距离才算拖动 —— 静态点击加一次布局回声不会被读成"拖离底部"。
export const POINTER_DRAG_SLOP_PX = 4

// 贴底侧手势 latch。仅由朝底部的滚轮/触摸/键输入武装，可被连续向下滚动帧延长（触屏惯性无输入事件）。
export const GESTURE_LATCH_MS = 500

export type FollowConfig = {
  attachThresholdPx: number
  reattachZonePx: number
  directionSlopPx: number
  latchMs: number
}

export const DEFAULT_FOLLOW_CONFIG: FollowConfig = {
  attachThresholdPx: BOTTOM_ATTACH_THRESHOLD_PX,
  reattachZonePx: BOTTOM_REATTACH_ZONE_PX,
  directionSlopPx: DIRECTION_SLOP_PX,
  latchMs: GESTURE_LATCH_MS,
}

export type FollowState = {
  following: boolean
  pointerHeld: boolean
  pointerDragging: boolean
  dragTowardBottom: boolean | null
  latchUntil: number
  lastGap: number
}

export function createFollowState(): FollowState {
  return {
    following: true,
    pointerHeld: false,
    pointerDragging: false,
    dragTowardBottom: null,
    latchUntil: 0,
    lastGap: 0,
  }
}

export type FollowEvent =
  | {
      type: 'wheel'
      deltaX: number
      deltaY: number
      gap: number
      hasOverflow: boolean
      nestedCanConsume: boolean
      now: number
    }
  | {
      type: 'touchMove'
      fingerMovedDown: boolean | null
      gap: number
      hasOverflow: boolean
      now: number
    }
  | { type: 'scroll'; gap: number; now: number }
  | { type: 'pointerDown' }
  | { type: 'pointerDragStart' }
  | { type: 'pointerRelease'; gap: number }
  | { type: 'historyKey'; hasOverflow: boolean; now: number }
  | { type: 'followKey'; now: number }
  | { type: 'contentGrowth'; gap: number }
  | { type: 'forceFollow' }
  | { type: 'release' }

export type FollowStep = {
  state: FollowState
  // hook 的副作用：立即 scrollTop = scrollHeight。
  pin: boolean
}

export function isAtBottom(gap: number, config: FollowConfig = DEFAULT_FOLLOW_CONFIG) {
  return gap <= config.attachThresholdPx
}

// 触控板横向平移（宽代码块、表格）每帧带几 px 纵向漂移；只有以纵向为主的手势才能改变跟随状态。
export function isDominantVerticalWheel(deltaX: number, deltaY: number) {
  return Math.abs(deltaY) > Math.abs(deltaX)
}

export function reduceFollowEvent(
  state: FollowState,
  event: FollowEvent,
  config: FollowConfig = DEFAULT_FOLLOW_CONFIG,
): FollowStep {
  switch (event.type) {
    case 'wheel': {
      if (!isDominantVerticalWheel(event.deltaX, event.deltaY)) {
        return { state, pin: false }
      }
      if (event.deltaY < 0) {
        const next = { ...state, latchUntil: 0 }
        if (event.hasOverflow && !event.nestedCanConsume) {
          next.following = false
        }
        return { state: next, pin: false }
      }
      const next = { ...state, latchUntil: event.now + config.latchMs }
      if (!state.following && isAtBottom(event.gap, config)) {
        next.following = true
        return { state: next, pin: true }
      }
      return { state: next, pin: false }
    }

    case 'touchMove': {
      const movedAway = event.fingerMovedDown !== false
      const next = {
        ...state,
        latchUntil: movedAway ? 0 : event.now + config.latchMs,
      }
      if (event.hasOverflow && (movedAway || event.gap > config.attachThresholdPx)) {
        next.following = false
      }
      return { state: next, pin: false }
    }

    case 'scroll': {
      const { gap, now } = event
      const previousGap = state.lastGap
      const next = { ...state, lastGap: gap }

      if (isAtBottom(gap, config)) {
        next.dragTowardBottom = true
        if (state.following || now <= state.latchUntil) {
          next.following = true
        }
        return { state: next, pin: false }
      }

      const movedAway = gap > previousGap + config.directionSlopPx
      const movedTowardBottom = gap < previousGap - config.directionSlopPx

      if (state.pointerDragging && movedAway) {
        next.following = false
        next.dragTowardBottom = false
        return { state: next, pin: false }
      }

      if (state.following) {
        // 纠正器：跟随中任何撑开 gap 但未在上面解除的滚动，用再钉一次撤销。
        return { state: next, pin: true }
      }

      if (movedTowardBottom) {
        next.dragTowardBottom = true
        if (now <= state.latchUntil) {
          next.latchUntil = now + config.latchMs
          if (!state.pointerHeld && gap <= config.reattachZonePx) {
            next.following = true
            return { state: next, pin: true }
          }
        }
      } else if (movedAway) {
        next.dragTowardBottom = false
      }
      return { state: next, pin: false }
    }

    case 'pointerDown': {
      return { state: { ...state, pointerHeld: true, dragTowardBottom: null }, pin: false }
    }

    case 'pointerDragStart': {
      if (!state.pointerHeld) {
        return { state, pin: false }
      }
      return { state: { ...state, pointerDragging: true }, pin: false }
    }

    case 'pointerRelease': {
      if (!state.pointerHeld) {
        return { state, pin: false }
      }
      const next = {
        ...state,
        pointerHeld: false,
        pointerDragging: false,
        dragTowardBottom: null,
      }
      const releaseZonePx = Math.max(config.reattachZonePx, config.attachThresholdPx)
      if (state.dragTowardBottom === true && event.gap <= releaseZonePx) {
        next.following = true
        return { state: next, pin: true }
      }
      return { state: next, pin: false }
    }

    case 'historyKey': {
      const next = { ...state, latchUntil: 0 }
      if (event.hasOverflow) {
        next.following = false
      }
      return { state: next, pin: false }
    }

    case 'followKey': {
      return { state: { ...state, latchUntil: event.now + config.latchMs }, pin: false }
    }

    case 'contentGrowth': {
      return { state: { ...state, lastGap: event.gap }, pin: state.following }
    }

    case 'forceFollow': {
      return {
        state: {
          ...state,
          following: true,
          pointerDragging: false,
          dragTowardBottom: null,
          latchUntil: 0,
        },
        pin: true,
      }
    }

    case 'release': {
      // Kivio 新增：主动脱离跟随（消息导航器跳转到上方消息时用），不钉底。
      return { state: { ...state, following: false, latchUntil: 0 }, pin: false }
    }
  }
}
