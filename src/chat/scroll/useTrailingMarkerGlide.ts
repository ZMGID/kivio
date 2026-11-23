import { useLayoutEffect, useRef } from 'react'
import { prefersReducedMotion } from '../utils'

// 平滑时间常数：一行新字（约 24px）大约 250ms 走完。
const MARKER_GLIDE_TAU_MS = 80
// 位移超过这个值（发送、切会话、展开大块）直接落位，不做长距离滑行。
const MARKER_SNAP_PX = 160
// 这一轮结束（落库交接完成）后这段时间内的提交仍做平滑；之后闲置不再读布局。
const SETTLE_WINDOW_MS = 1500
// 状态行前面紧挨着的流内行（流式回答、压缩进度、错误）；再往前是绝对定位的历史行。
const OBSERVED_PREVIOUS_SIBLINGS = 3

/**
 * 下一帧显示位置：指数逼近目标；距离过大或已足够近时直接落到目标。
 * 往下追时最多落后 `maxLagPx`——落后更多，刚长出来的正文就会压到墨墨。
 */
export function nextMarkerPosition(
  shown: number | null,
  target: number,
  dtMs: number,
  { reducedMotion = false, maxLagPx = Infinity }: { reducedMotion?: boolean; maxLagPx?: number } = {},
): number {
  if (shown == null || reducedMotion || Math.abs(target - shown) > MARKER_SNAP_PX) return target
  let next = shown + (target - shown) * (1 - Math.exp(-Math.max(0, dtMs) / MARKER_GLIDE_TAU_MS))
  if (target > next) next = Math.max(next, target - maxLagPx)
  return Math.abs(target - next) < 0.5 ? target : next
}

type MarkerState = { shown: number | null; applied: number; settleUntil: number }

/** 标记到滚动内容底部之间的流内距离：后面的兄弟 + 各层祖先的下内边距/边框。贴底时就是 sticky 的 bottom。 */
function distanceToContentEnd(marker: HTMLElement, viewport: HTMLElement): number {
  let below = 0
  for (let el: HTMLElement = marker; el !== viewport;) {
    for (let next = el.nextElementSibling; next; next = next.nextElementSibling) {
      if (getComputedStyle(next).position !== 'absolute') below += (next as HTMLElement).offsetHeight
    }
    const parent = el.parentElement
    if (!parent) break
    const style = getComputedStyle(parent)
    below += parseFloat(style.paddingBottom) + parseFloat(style.borderBottomWidth)
    if (parent === viewport) break
    el = parent
  }
  return below
}

/**
 * 消息流末尾状态行（墨墨）的视觉位置：
 * - 生成中、下方还有空白：跟着正文平滑下移，不随每行新字一跳一跳（只写 transform）。
 * - 生成中、贴底跟随：sticky 固定在钉底后的槽位。正文由跟随缓动滚上来，
 *   墨墨不随缓动滞后先被挤出底边再拉回；由浏览器在绘制时夹住，和 rAF 先后无关。
 * - 用户离开底部、按住指针（跟随暂停）或生成结束：去掉 sticky，贴着内容走，不悬浮在正文上。
 *
 * 只在位置还没落定时跑 rAF；新字、视口变化、排队消息和列表提交负责把它唤醒。
 */
export function useTrailingMarkerGlide({ marker, viewport, active, live, isFollowing, isPinningBottom, subscribePinning, maxLagPx, resetKey }: {
  marker: HTMLElement | null
  viewport: HTMLElement | null
  /** 正在出字（可贴底固定）。 */
  active: boolean
  /** 这一轮还没落库（含停止后的冻结）；结束时开始收尾平滑窗口。 */
  live: boolean
  isFollowing: () => boolean
  isPinningBottom: () => boolean
  /** 贴底状态变化（按下/松开指针）时唤醒；不经 React 渲染。 */
  subscribePinning: (listener: () => void) => () => void
  /** 生成中往下平滑时最多落后多少 px（状态行上方留出的空白）。 */
  maxLagPx: number
  /** 变化即直接落位（切会话复用同一列表时不从上个会话的位置滑过来）。 */
  resetKey?: unknown
}): void {
  const isFollowingRef = useRef(isFollowing)
  isFollowingRef.current = isFollowing
  const isPinningBottomRef = useRef(isPinningBottom)
  isPinningBottomRef.current = isPinningBottom
  // 整个挂载期只有这一个对象；重置只改字段，正在运行的 effect 与之始终一致。
  const stateRef = useRef<MarkerState>({ shown: null, applied: 0, settleUntil: 0 })
  const syncRef = useRef<(() => void) | null>(null)
  const wasLiveRef = useRef(live)

  // 换节点或换会话：位移、sticky 和平滑状态一起清掉，下一次同步直接落位。
  useLayoutEffect(() => {
    const state = stateRef.current
    state.shown = null
    state.applied = 0
    if (!marker) return
    marker.style.transform = ''
    marker.style.position = ''
    marker.style.bottom = ''
  }, [marker, resetKey])

  // 收尾窗口从这一轮真正结束（落库交接）时起算；进行中不会过期，不论停了多久没出字。
  useLayoutEffect(() => {
    const state = stateRef.current
    if (live) state.settleUntil = Infinity
    else if (wasLiveRef.current) state.settleUntil = performance.now() + SETTLE_WINDOW_MS
    wasLiveRef.current = live
  }, [live])

  // 布局 effect：生成开始/结束（留空、sticky 变化）的同一帧就补上位移，收尾不先跳一下。
  useLayoutEffect(() => {
    if (!marker || !viewport) return
    const state = stateRef.current
    const reducedMotion = prefersReducedMotion()
    let frame = 0
    let last = performance.now()
    let stopped = false

    const writeOffset = (offset: number) => {
      const rounded = Math.round(offset)
      const changed = rounded !== Math.round(state.applied)
      state.applied = offset
      if (changed) marker.style.transform = rounded === 0 ? '' : `translateY(${rounded}px)`
    }

    /** 返回 sticky 的 bottom（未固定时为 0）。 */
    const writeSticky = (pin: boolean): number => {
      if (!pin) {
        if (marker.style.position) {
          marker.style.position = ''
          marker.style.bottom = ''
        }
        return 0
      }
      const below = distanceToContentEnd(marker, viewport)
      const bottom = `${below}px`
      if (marker.style.position !== 'sticky') marker.style.position = 'sticky'
      if (marker.style.bottom !== bottom) marker.style.bottom = bottom
      return below
    }

    /** 返回位置是否还没落定（需要下一帧继续）。 */
    const update = (now: number): boolean => {
      const dt = Math.min(64, Math.max(0, now - last))
      last = now
      if (!isFollowingRef.current()) {
        // 读者在别处：贴着内容走，不平滑、不轮询；回到底部时列表提交会重新唤醒。
        writeSticky(false)
        state.shown = null
        writeOffset(0)
        return false
      }
      const pinned = active && isPinningBottomRef.current()
      const stickyBottom = writeSticky(pinned)
      const viewportRect = viewport.getBoundingClientRect()
      const rect = marker.getBoundingClientRect()
      const offset = Math.round(state.applied)
      const natural = rect.top - viewportRect.top - offset
      // 被 sticky 夹在槽位里且槽位上移（视口变矮、插入排队消息）：直接落位，不从槽外滑回来。
      const stuck = pinned && rect.bottom - offset >= viewportRect.bottom - stickyBottom - 0.5
      if (stuck && state.shown != null && state.shown > natural) state.shown = natural
      // 贴底时正文还会因跟随缓动落后一段；两段落后加起来不超过上限，正文才碰不到墨墨。
      const scrollLag = stuck ? Math.max(0, viewport.scrollHeight - viewport.clientHeight - viewport.scrollTop) : 0
      state.shown = nextMarkerPosition(state.shown, natural, dt, {
        reducedMotion,
        maxLagPx: active ? Math.max(0, maxLagPx - scrollLag) : Infinity,
      })
      writeOffset(state.shown - natural)
      return Math.abs(state.shown - natural) >= 0.5
    }

    const step = (now: number) => {
      frame = 0
      if (stopped) return
      if (update(now)) frame = requestAnimationFrame(step)
    }

    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(() => {
      // 布局后、绘制前：新一行字撑高的同一帧就补上位移，不会先闪一下跳下去的位置。
      sync()
    })
    const observed = new Set<Element>()
    const observeNeighbours = () => {
      if (!observer) return
      // 状态行自身（状态说明换行变高）、视口、前后的流内邻居。
      const wanted = new Set<Element>([viewport, marker])
      let el = marker.previousElementSibling
      for (let count = 0; el && count < OBSERVED_PREVIOUS_SIBLINGS; count += 1, el = el.previousElementSibling) {
        if (getComputedStyle(el).position === 'absolute') break
        wanted.add(el)
      }
      for (let next = marker.nextElementSibling; next; next = next.nextElementSibling) wanted.add(next)
      for (const target of observed) {
        if (wanted.has(target)) continue
        observer.unobserve(target)
        observed.delete(target)
      }
      for (const target of wanted) {
        if (observed.has(target)) continue
        observer.observe(target)
        observed.add(target)
      }
    }
    const releaseNeighbours = () => {
      observer?.disconnect()
      observed.clear()
    }

    // 提交/尺寸变化时的同步更新。循环已停时从 dt=0 起步，位移不会在发现的这一帧就被吃掉一截。
    function sync() {
      if (stopped) return
      const now = performance.now()
      if (!frame && now > state.settleUntil) {
        // 闲置且已落位：不读布局、不留观察目标，也不带着旧位置等下一轮生成。
        state.shown = null
        writeSticky(false)
        if (state.applied) writeOffset(0)
        releaseNeighbours()
        return
      }
      if (!frame) last = now
      observeNeighbours()
      if (update(now) && !frame) frame = requestAnimationFrame(step)
    }
    syncRef.current = sync
    sync()
    // 按下指针：跟随暂停、正文会落后，立即松开 sticky；松开时重新贴底。
    const unsubscribePinning = subscribePinning(sync)

    return () => {
      stopped = true
      unsubscribePinning()
      if (syncRef.current === sync) syncRef.current = null
      if (frame) cancelAnimationFrame(frame)
      releaseNeighbours()
      // 不留 sticky；未走完的位移交给下一次 effect 继续（state 保留）。
      writeSticky(false)
    }
  }, [marker, viewport, active, maxLagPx, subscribePinning])

  // 列表每次提交（收尾换行、预留交接、钉底、跟随状态变化）后、绘制前同步一次。
  useLayoutEffect(() => {
    syncRef.current?.()
  })
}
