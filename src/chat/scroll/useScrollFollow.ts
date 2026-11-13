import { createContext, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'


import {
  createFollowState,
  DEFAULT_FOLLOW_CONFIG,
  type FollowConfig,
  type FollowEvent,
  type FollowState,
  reduceFollowEvent,
} from './scrollFollowCore'

// Share the viewport authority with nested timelines, including multi-answer columns.
export const ScrollFollowingContext = createContext({
  following: true,
  isFollowing: (): boolean => true,
  // 流式行每次提交后调用：ResizeObserver 未投递时补一次钉底（见 notifyContentGrowth）。
  notifyContentGrowth: (): void => {},
})

// 低于此高度元素无法有效滚动；其上的 wheel/touch 不应改变跟随状态。
const SCROLLABLE_OVERFLOW_MIN_PX = 4

function isEditableEventTarget(target: EventTarget | null) {
  if (!(target instanceof HTMLElement)) return false
  return (
    target.isContentEditable ||
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement ||
    target instanceof HTMLSelectElement
  )
}

function isHistoryScrollKey(event: KeyboardEvent) {
  if (isEditableEventTarget(event.target)) return false
  return (
    event.key === 'ArrowUp' ||
    event.key === 'PageUp' ||
    event.key === 'Home' ||
    (event.key === ' ' && event.shiftKey)
  )
}

function isFollowScrollKey(event: KeyboardEvent) {
  if (isEditableEventTarget(event.target)) return false
  return (
    event.key === 'ArrowDown' ||
    event.key === 'PageDown' ||
    event.key === 'End' ||
    (event.key === ' ' && !event.shiftKey)
  )
}

export type ScrollFollowHandle = {
  // 强制跟随并立即钉底（元素未绑定时于视口到位后钉）。
  stickToBottom: () => void
  // 瞬时跳到底部并强制跟随。给用户可见的操作（回到底部按钮）用；与 stickToBottom 同路径，
  // 不走平滑动画——长会话 virtualizer 重测补偿会中途打断 rAF 动画，导致要点多次。
  jumpToBottom: () => void
  // 主动脱离跟随（导航跳转到上方消息时用）。
  releaseFollow: () => void
  // Restore a previous reading location before the viewport effect binds.
  restoreReadingPosition: (offset: number) => void
  // 程序化定位（消息导航）唯一的 scrollTop 写入口，不改变 follow 意图。
  scrollToOffset: (offset: number, options?: { adjustments?: number; behavior?: ScrollBehavior }) => void
  isFollowing: () => boolean
  markLayoutCompensation: () => void
  // 仅在仍跟随时钉底（不 force re-attach）。live 行测高 / 代码块 hydrate 用。
  pinIfFollowing: () => void
  // 内容刚提交过：跟随中且 scrollHeight 真变了才补一次 contentGrowth。
  // ResizeObserver 是主路径，这是它未投递时的补钉；高度没变则 no-op。
  notifyContentGrowth: () => void
}

// 跟随解除和按钮显示故意分开：底部附近的微小上滚不应立刻弹出按钮。
// 用视口高度算「明显离开底部」的距离，再设上下限，避免固定 px 在不同窗口里过早/过晚出现。
const JUMP_BUTTON_SHOW_RATIO = 0.35
const JUMP_BUTTON_SHOW_MIN_PX = 240
const JUMP_BUTTON_SHOW_MAX_PX = 480

// 流式跟随的缓动：每帧向底部逼近剩余距离的 1 - e^(-dt/τ)。τ=70ms 时一行（约 26px）
// 在 ~200ms 内平滑到位，跟得上逐字出字的换行节奏。
const GLIDE_TAU_MS = 70
// 超过这个缺口不缓动、直接到底：打开会话、行重测、大块内容落地都可能一次长出很多，
// 缓动过去会读成一段「自动滚动动画」，也会和虚拟列表的补偿互抢。
const GLIDE_MAX_PX = 240

export type UseScrollFollowArgs = {
  viewport: HTMLElement | null
  content?: Element | null
  listenerRoot?: HTMLElement | null
  enabled?: boolean
  trackKeys?: boolean
  config?: Partial<FollowConfig>
  /**
   * 流式内容指纹。ResizeObserver 是主路径；外置 live row / jsdom stub 等 RO
   * 不投递时，跟随时若 scrollHeight 真变了，靠这个信号补 contentGrowth。
   * 高度没变则 no-op，避免和 RO 双通道每 token 互写。
   */
  growthSignal?: string | number | null
  /**
   * 跟随中内容小幅增长时缓动到底，而不是瞬间钉底（流式出字时开启）。
   * 只影响 contentGrowth 引起的钉底；强制跟随、回到底部、松手重接等仍是瞬时。
   */
  glideGrowth?: boolean
}

export function useScrollFollow(args: UseScrollFollowArgs): {
  handle: ScrollFollowHandle
  following: boolean
  showJumpButton: boolean
} {
  const {
    viewport,
    content = null,
    listenerRoot = null,
    enabled = true,
    trackKeys = false,
    growthSignal = null,
    glideGrowth = false,
  } = args
  const glideGrowthRef = useRef(glideGrowth)
  glideGrowthRef.current = glideGrowth


  const stateRef = useRef<FollowState>(createFollowState())
  const boundViewportRef = useRef<HTMLElement | null>(null)
  const configRef = useRef<FollowConfig>(DEFAULT_FOLLOW_CONFIG)
  configRef.current = { ...DEFAULT_FOLLOW_CONFIG, ...args.config }
  const [following, setFollowing] = useState(true)
  const [showJumpButton, setShowJumpButton] = useState(false)
  const layoutCompensationTicketRef = useRef(false)
  const layoutCompensationClearRafRef = useRef<number | null>(null)

  // 机制一（对齐 use-stick-to-bottom 的 ignoreScrollToTop）：记下**我们自己写完之后读回来的**
  // scrollTop。下一个 scroll 事件里 el.scrollTop 与之相等 → 这一下是自己弄出来的，不是用户滚的。
  // 必须读回：pin 写的是 scrollHeight，浏览器会 clamp 到 scrollHeight - clientHeight，
  // 拿写入值去比永远比不中。
  const ignoreScrollTopRef = useRef<number | null>(null)
  // Keep only geometry evidence for virtualizer/browser compensation. There is no timer-based
  // "resize window": user intent is recorded by wheel/touch/key events, while a scroll that
  // arrives with a larger scrollHeight is treated as layout compensation.
  const lastScrollHeightRef = useRef<number | null>(null)
  const lastScrollTopRef = useRef<number | null>(null)
  const geometrySampledRef = useRef(false)
  const pendingRestoreRef = useRef<number | null>(null)

  // 唯一的 scrollTop 写入口：写完立刻读回并登记，别处一律不许直接赋值。
  const applyScrollTop = useCallback((el: HTMLElement, value: number) => {
    el.scrollTop = value
    ignoreScrollTopRef.current = el.scrollTop
  }, [])

  const jumpButtonThreshold = useCallback(() => {
    const viewport = boundViewportRef.current
    const viewportHeight = viewport?.clientHeight ?? 0
    return Math.min(
      JUMP_BUTTON_SHOW_MAX_PX,
      Math.max(JUMP_BUTTON_SHOW_MIN_PX, viewportHeight * JUMP_BUTTON_SHOW_RATIO),
    )
  }, [])

  const markLayoutCompensation = useCallback(() => {
    layoutCompensationTicketRef.current = true
    if (layoutCompensationClearRafRef.current !== null) {
      cancelAnimationFrame(layoutCompensationClearRafRef.current)
    }
    layoutCompensationClearRafRef.current = requestAnimationFrame(() => {
      // Scroll events caused by a measurement/anchor correction are delivered
      // in the scroll steps before the following paint. Keep the ticket through
      // one extra frame so a ResizeObserver → virtualizer → scroll sequence is
      // one explicit transaction even when the first rAF also performs pinning.
      layoutCompensationClearRafRef.current = requestAnimationFrame(() => {
        layoutCompensationClearRafRef.current = null
        layoutCompensationTicketRef.current = false
      })
    })
  }, [])

  // 缓动循环：每帧读取最新的底部位置（内容在缓动中继续增长也会被追上）。
  const glideRef = useRef<{ frame: number; last: number } | null>(null)
  const stopGlide = useCallback(() => {
    const glide = glideRef.current
    if (!glide) return
    cancelAnimationFrame(glide.frame)
    glideRef.current = null
  }, [])

  const pinToBottom = useCallback(() => {
    // 对齐 LiveAgent：瞬时单次写入。双帧 rAF 会在 paint 后再钉一次，
    // 流式中表现就是生成内容整段「往下闪」。virtualizer 估算→实测的第二下
    // 高度变化由 ResizeObserver contentGrowth 再钉，节奏已 ≤1/frame。
    stopGlide()
    const el = boundViewportRef.current
    if (el) applyScrollTop(el, el.scrollHeight)
  }, [applyScrollTop, stopGlide])

  const glideToBottom = useCallback(() => {
    const el = boundViewportRef.current
    if (!el) return
    const gap = el.scrollHeight - el.clientHeight - el.scrollTop
    const reducedMotion = typeof window !== 'undefined'
      && window.matchMedia?.('(prefers-reduced-motion: reduce)').matches === true
    if (gap > GLIDE_MAX_PX || reducedMotion) {
      pinToBottom()
      return
    }
    if (glideRef.current) return
    const step = (now: number) => {
      const glide = glideRef.current
      const viewport = boundViewportRef.current
      if (!glide || !viewport) {
        glideRef.current = null
        return
      }
      const state = stateRef.current
      // 用户接管（滚轮、触摸、按住拖选、主动脱离）立刻停，不和人抢视口。
      if (!state.following || state.pointerHeld) {
        glideRef.current = null
        return
      }
      const max = viewport.scrollHeight - viewport.clientHeight
      const remaining = max - viewport.scrollTop
      if (remaining <= 1) {
        // 最后不到 1px 直接落到底，不留小数缺口。
        if (remaining > 0.01) applyScrollTop(viewport, max)
        glideRef.current = null
        return
      }
      const dt = Math.min(64, Math.max(0, now - glide.last))
      glide.last = now
      const next = remaining > GLIDE_MAX_PX
        ? max
        : viewport.scrollTop + Math.min(remaining, Math.max(1, remaining * (1 - Math.exp(-dt / GLIDE_TAU_MS))))
      applyScrollTop(viewport, next)
      glide.frame = requestAnimationFrame(step)
    }
    const startedAt = performance.now()
    glideRef.current = { frame: requestAnimationFrame(step), last: startedAt }
  }, [applyScrollTop, pinToBottom])

  const dispatch = useCallback(
    (event: FollowEvent) => {
      const wasFollowing = stateRef.current.following
      const step = reduceFollowEvent(stateRef.current, event, configRef.current)
      stateRef.current = step.state
      if (step.pin) {
        // 流式增长：缓动到底。缓动自身产生的 scroll 事件仍处于离底状态，reducer 会要求
        // 「纠正钉底」，此时交给正在进行的缓动，而不是瞬间打断它。
        const glide = glideGrowthRef.current
          && (event.type === 'contentGrowth' || (event.type === 'scroll' && glideRef.current !== null))
        if (glide) glideToBottom()
        else pinToBottom()
      } else if (!step.state.following) {
        stopGlide()
      }
      if (step.state.following !== wasFollowing) {
        setFollowing(step.state.following)
      }
      // `gap` 是当前事件采样值；没有 gap 的控制事件（forceFollow/release）沿用
      // 状态里的最近一次采样。只在跨过显示阈值时更新，避免每个 scroll 帧触发重渲。
      const gap = 'gap' in event ? event.gap : step.state.lastGap
      const shouldShowJumpButton = !step.state.following && gap > jumpButtonThreshold()
      setShowJumpButton((visible) => visible === shouldShowJumpButton ? visible : shouldShowJumpButton)
    },
    [glideToBottom, jumpButtonThreshold, pinToBottom, stopGlide],
  )

  const stickToBottom = useCallback(() => {
    dispatch({ type: 'forceFollow' })
  }, [dispatch])

  const pinIfFollowing = useCallback(() => {
    if (!stateRef.current.following) return
    pinToBottom()
  }, [pinToBottom])

  const releaseFollow = useCallback(() => {
    dispatch({ type: 'release' })
  }, [dispatch])

  const restoreReadingPosition = useCallback((offset: number) => {
    const target = Math.max(0, offset)
    // A restore is consumed once. Keeping an already-applied offset would
    // replay it when content/listener bindings change after the reader moves.
    pendingRestoreRef.current = boundViewportRef.current ? null : target
    dispatch({ type: 'release' })
    const el = boundViewportRef.current ?? viewport
    if (el) applyScrollTop(el, target)
  }, [applyScrollTop, dispatch, viewport])

  const scrollToOffset = useCallback((offset: number, options?: { adjustments?: number; behavior?: ScrollBehavior }) => {
    const viewport = boundViewportRef.current
    if (!viewport) return
    const nextOffset = Math.max(0, offset + (options?.adjustments ?? 0))
    // TanStack's scrollToFn and message navigation stay synchronous so every
    // resulting scroll event matches this authority's one-shot programmatic token.
    applyScrollTop(viewport, nextOffset)
  }, [applyScrollTop])

  // Instant jump: force-follow + pin. No rAF animation — long chats remeasure
  // rows mid-flight and used to cancel the smooth path before stickToBottom ran.
  const jumpToBottom = useCallback(() => {
    stickToBottom()
  }, [stickToBottom])

  const initializedViewportRef = useRef<HTMLElement | null>(null)
  useEffect(() => {
    if (!enabled || !viewport) {
      return
    }
    const root = listenerRoot ?? viewport
    const growthTarget = content ?? viewport.firstElementChild

    // Observer/listener rebinding is not a new reading session. Only a new
    // viewport initializes follow; the same viewport retains the user's intent.
    const newViewport = initializedViewportRef.current !== viewport
    initializedViewportRef.current = viewport
    boundViewportRef.current = viewport
    const restoredOffset = pendingRestoreRef.current
    pendingRestoreRef.current = null
    if (restoredOffset !== null) {
      stateRef.current = { ...createFollowState(), following: false, userDetached: true }
      applyScrollTop(viewport, restoredOffset)
    } else if (newViewport) {
      stateRef.current = createFollowState()
      pinToBottom()
    }
    setFollowing(stateRef.current.following)
    setShowJumpButton(!stateRef.current.following
      && viewport.scrollTop < viewport.scrollHeight - viewport.clientHeight - jumpButtonThreshold())
    lastScrollHeightRef.current = viewport.scrollHeight
    lastScrollTopRef.current = viewport.scrollTop
    geometrySampledRef.current = false

    const getGap = () =>
      Math.max(0, viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight)
    const hasOverflow = () =>
      viewport.scrollHeight - viewport.clientHeight > SCROLLABLE_OVERFLOW_MIN_PX
    const nestedCanConsumeWheelUp = (target: EventTarget | null) => {
      let node = target instanceof Element ? target : null
      while (node && node !== viewport && node !== root) {
        if (
          node instanceof HTMLElement &&
          node.scrollTop > 0 &&
          node.scrollHeight - node.clientHeight > SCROLLABLE_OVERFLOW_MIN_PX
        ) {
          return true
        }
        node = node.parentElement
      }
      return false
    }

    const handleScroll = () => {
      // token **读一次就作废**：一次写入只授权一个 scroll 事件。不作废的话会卡死 ——
      // 「底部」这个数值是稳定的（我们 pin 写 scrollHeight，浏览器 clamp 成 max；
      // 用户把滚动条拖到最底，浏览器写进去的也是同一个 max，逐位相等），于是用户
      // 拖回底部那一下会被永远判成 self，三条重跟随的路全部落空。
      const token = ignoreScrollTopRef.current
      ignoreScrollTopRef.current = null
      const scrollTop = viewport.scrollTop
      const gap = getGap()
      const previousScrollHeight = lastScrollHeightRef.current
      const previousScrollTop = lastScrollTopRef.current
      const contentGrewBeforeScroll =
        geometrySampledRef.current &&
        previousScrollHeight !== null &&
        viewport.scrollHeight > previousScrollHeight &&
        (previousScrollTop === null || scrollTop >= previousScrollTop - 1) &&
        stateRef.current.following
      lastScrollHeightRef.current = viewport.scrollHeight
      lastScrollTopRef.current = scrollTop
      geometrySampledRef.current = true
      const layoutCompensation = layoutCompensationTicketRef.current
      layoutCompensationTicketRef.current = false
      const userReturnedDuringLayout = layoutCompensation
        && stateRef.current.userDetached
        && gap <= configRef.current.attachThresholdPx
        && previousScrollTop !== null
        && scrollTop > previousScrollTop + 1
      const selfInduced = scrollTop === token || contentGrewBeforeScroll || (layoutCompensation && !userReturnedDuringLayout)
      // Native scrollbar movement may have no wheel/pointer event. A backwards
      // move with unchanged geometry and no compensation token takes ownership.
      if (!selfInduced && previousScrollTop !== null &&
        scrollTop < previousScrollTop - 1 &&
        previousScrollHeight === viewport.scrollHeight &&
        gap > configRef.current.attachThresholdPx) {
        dispatch({ type: 'release' })
      }
      dispatch({
        type: 'scroll',
        gap,
        now: Date.now(),
        source: selfInduced ? 'self' : 'user',
      })
    }

    const handleWheel = (event: WheelEvent) => {
      dispatch({
        type: 'wheel',
        deltaX: event.deltaX,
        deltaY: event.deltaY,
        gap: getGap(),
        hasOverflow: hasOverflow(),
        nestedCanConsume: event.deltaY < 0 && nestedCanConsumeWheelUp(event.target),
        now: Date.now(),
      })
    }

    let touchY: number | null = null
    const handleTouchStart = (event: TouchEvent) => {
      touchY = event.touches[0]?.clientY ?? null
    }
    const handleTouchMove = (event: TouchEvent) => {
      const nextY = event.touches[0]?.clientY ?? null
      const previousY = touchY
      touchY = nextY
      dispatch({
        type: 'touchMove',
        fingerMovedDown: previousY === null || nextY === null ? null : nextY > previousY + 1,
        gap: getGap(),
        hasOverflow: hasOverflow(),
        now: Date.now(),
      })
    }

    // pointerHeld 只用于「按住不放时别自动重新跟随」（按住拖选文本会带出滚动）。
    // 原来这里还有一条按 `[data-scroll-area-scrollbar]` 识别拖滚动条的分支 —— 那个属性
    // 整个 src/ 里没有任何组件渲染（是从自定义滚动条组件那边搬过来时留下的），聊天列表用的是
    // .custom-scrollbar 原生滚动条，永远命中不了。原生滚动条既不派发 DOM 指针事件也没有 wheel，
    // 现在由 scroll 事件的 source 判定统一接管，这条分支连同 pointerDragging 一起删掉了。
    const handlePointerDown = (event: PointerEvent) => {
      if (event.pointerType === 'mouse' && event.button === 2) {
        return
      }
      dispatch({ type: 'pointerDown' })
    }
    const handlePointerRelease = () => {
      dispatch({ type: 'pointerRelease', gap: getGap() })
    }
    const handlePointerMove = (event: PointerEvent) => {
      if (!stateRef.current.pointerHeld) {
        return
      }
      if (event.buttons === 0) {
        handlePointerRelease()
      }
    }

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing || event.altKey || event.ctrlKey || event.metaKey) return
      const target = event.target
      if (!(target instanceof Node) || (target !== document.body && target !== document.documentElement && !root.contains(target))) return
      if (isHistoryScrollKey(event)) {
        dispatch({ type: 'historyKey', hasOverflow: hasOverflow(), now: Date.now() })
      } else if (isFollowScrollKey(event)) {
        dispatch({ type: 'followKey', now: Date.now() })
      }
    }

    const handleVisibilityChange = () => {
      if (document.visibilityState === 'visible' && stateRef.current.following) {
        pinToBottom()
      }
    }

    viewport.addEventListener('scroll', handleScroll, { passive: true })
    root.addEventListener('wheel', handleWheel, { passive: true })
    root.addEventListener('touchstart', handleTouchStart, { passive: true })
    root.addEventListener('touchmove', handleTouchMove, { passive: true })
    root.addEventListener('pointerdown', handlePointerDown, { passive: true })
    window.addEventListener('pointerup', handlePointerRelease, { passive: true })
    window.addEventListener('pointercancel', handlePointerRelease, { passive: true })
    window.addEventListener('pointermove', handlePointerMove, { passive: true })
    window.addEventListener('blur', handlePointerRelease)
    if (trackKeys) {
      window.addEventListener('keydown', handleKeyDown)
    }
    document.addEventListener('visibilitychange', handleVisibilityChange)

    // 布局后、绘制前，在每次内容/视口尺寸变化时触发 —— 这就是流式钉底驱动。
    const resizeObserver =
      typeof ResizeObserver === 'undefined'
        ? null
        : new ResizeObserver(() => {
            markLayoutCompensation()
            dispatch({ type: 'contentGrowth', gap: getGap() })
            lastScrollHeightRef.current = viewport.scrollHeight
            lastScrollTopRef.current = viewport.scrollTop
            geometrySampledRef.current = true
          })
    resizeObserver?.observe(viewport)
    if (growthTarget instanceof Element) {
      resizeObserver?.observe(growthTarget)
    }

    return () => {
      viewport.removeEventListener('scroll', handleScroll)
      root.removeEventListener('wheel', handleWheel)
      root.removeEventListener('touchstart', handleTouchStart)
      root.removeEventListener('touchmove', handleTouchMove)
      root.removeEventListener('pointerdown', handlePointerDown)
      window.removeEventListener('pointerup', handlePointerRelease)
      window.removeEventListener('pointercancel', handlePointerRelease)
      window.removeEventListener('pointermove', handlePointerMove)
      window.removeEventListener('blur', handlePointerRelease)
      if (trackKeys) {
        window.removeEventListener('keydown', handleKeyDown)
      }
      document.removeEventListener('visibilitychange', handleVisibilityChange)
      resizeObserver?.disconnect()
      if (layoutCompensationClearRafRef.current !== null) {
        cancelAnimationFrame(layoutCompensationClearRafRef.current)
        layoutCompensationClearRafRef.current = null
      }
      layoutCompensationTicketRef.current = false
      stopGlide()
      ignoreScrollTopRef.current = null
      boundViewportRef.current = null
      lastScrollHeightRef.current = null
      lastScrollTopRef.current = null
      geometrySampledRef.current = false
    }
  }, [applyScrollTop, content, dispatch, enabled, jumpButtonThreshold, listenerRoot, markLayoutCompensation, pinToBottom, stopGlide, trackKeys, viewport])

  // RO 主路径之外的补钉：仅当跟随中且 scrollHeight 真的变了。
  const notifyContentGrowth = useCallback(() => {
    const el = boundViewportRef.current ?? (enabled ? viewport : null)
    if (!el || !stateRef.current.following) return
    const nextHeight = el.scrollHeight
    const prevHeight = lastScrollHeightRef.current
    if (prevHeight != null && nextHeight === prevHeight) return
    markLayoutCompensation()
    const gap = Math.max(0, nextHeight - el.scrollTop - el.clientHeight)
    dispatch({ type: 'contentGrowth', gap })
    lastScrollHeightRef.current = nextHeight
    lastScrollTopRef.current = el.scrollTop
  }, [dispatch, enabled, markLayoutCompensation, viewport])

  useLayoutEffect(() => {
    if (growthSignal == null) return
    notifyContentGrowth()
  }, [growthSignal, notifyContentGrowth])


  const handle = useMemo<ScrollFollowHandle>(
    () => ({
      stickToBottom,
      jumpToBottom,
      releaseFollow,
      restoreReadingPosition,
      scrollToOffset,
      isFollowing: () => stateRef.current.following,
      markLayoutCompensation,
      pinIfFollowing,
      notifyContentGrowth,
    }),
    [jumpToBottom, markLayoutCompensation, notifyContentGrowth, pinIfFollowing, releaseFollow, restoreReadingPosition, scrollToOffset, stickToBottom],
  )

  return { handle, following, showJumpButton }
}
