import { useCallback, useRef, useSyncExternalStore } from 'react'
import type { ConversationStreamSnapshot } from './conversationRuns'

// 流式预览的高频状态从 Chat.tsx 的 useState 抽到这里，用 React 内置 useSyncExternalStore 订阅。
// 用外部 store（而非 Context）：60fps 下 Context 会重渲所有 consumer，store 只通知真正订阅的组件。
//
// 切成两个 slice，按更新频率分开订阅：
// - content：每帧都变（流式文本/推理/工具/分段），仅 MessageList 订阅。
// - coarse：边沿才变（streaming/frozen/cancelling/error 布尔），Chat 的 showEmptyHero/drain 与
//   InputBar 的取消按钮订阅——避免它们被每帧的内容更新拖着重渲。

export interface StreamCoarse {
  streaming: boolean
  streamFrozen: boolean
  cancelling: boolean
  streamError: string
}

// 空闲态内容快照（streaming:false、内容全空）。与 createEmptyStreamSnapshot 不同——后者是
// 「起一轮新流」用的（streaming:true + startedAt:now）。这里是 reset 的目标常量，引用恒定。
const IDLE_SNAPSHOT: ConversationStreamSnapshot = {
  runId: null,
  messageId: null,
  streaming: false,
  content: '',
  reasoning: '',
  reasoningStreaming: false,
  toolCalls: [],
  segments: [],
  startedAt: null,
  reasoningStartedAt: null,
  reasoningDurationMs: null,
  reasoningStartedAtBySegmentId: {},
  reasoningDurationMsBySegmentId: {},
  statusNote: null,
  retryStatus: null,
}

let snapshot: ConversationStreamSnapshot = IDLE_SNAPSHOT
let coarse: StreamCoarse = {
  streaming: false,
  streamFrozen: false,
  cancelling: false,
  streamError: '',
}

const snapshotSubs = new Set<() => void>()
const coarseSubs = new Set<() => void>()

function emit(subs: Set<() => void>) {
  for (const cb of subs) cb()
}

// ---- content slice ----

export function subscribeSnapshot(cb: () => void): () => void {
  snapshotSubs.add(cb)
  return () => {
    snapshotSubs.delete(cb)
  }
}

export function getSnapshot(): ConversationStreamSnapshot {
  return snapshot
}

// Chat 每帧传入的是 streamSnapshotsRef 里被原地 mutate 的同一个对象引用，必须浅拷贝出新引用，
// 否则 useSyncExternalStore 的 Object.is 比较检测不到变化、不会重渲。
export function setSnapshot(next: ConversationStreamSnapshot): void {
  snapshot = { ...next }
  emit(snapshotSubs)
}

export function patchSnapshot(patch: Partial<ConversationStreamSnapshot>): void {
  snapshot = { ...snapshot, ...patch }
  emit(snapshotSubs)
}

// ---- coarse slice ----

export function subscribeCoarse(cb: () => void): () => void {
  coarseSubs.add(cb)
  return () => {
    coarseSubs.delete(cb)
  }
}

export function getCoarse(): StreamCoarse {
  return coarse
}

// 逐字段浅比较：无实际变化则不分配新对象、不通知。否则流式中重复写 {streaming:true} 仍会让
// 订阅者每帧重渲，等于没隔离。
export function setCoarse(patch: Partial<StreamCoarse>): void {
  let changed = false
  for (const key of Object.keys(patch) as (keyof StreamCoarse)[]) {
    if (!Object.is(coarse[key], patch[key])) {
      changed = true
      break
    }
  }
  if (!changed) return
  coarse = { ...coarse, ...patch }
  emit(coarseSubs)
}

// 清空预览：内容回空闲 + streaming/frozen/cancelling 归位。**不动 streamError**——与
// Chat 原 clearStreamingPreview 语义一致（错误由 setStreamErrorForConversation/restore 独立管理）。
export function reset(): void {
  if (snapshot !== IDLE_SNAPSHOT) {
    snapshot = IDLE_SNAPSHOT
    emit(snapshotSubs)
  }
  setCoarse({ streaming: false, streamFrozen: false, cancelling: false })
}

// ---- React hooks ----

export function useStreamSnapshot(): ConversationStreamSnapshot {
  return useSyncExternalStore(subscribeSnapshot, getSnapshot, getSnapshot)
}

const subscribeNothing = (): (() => void) => () => {}
const getIdleSnapshot = (): ConversationStreamSnapshot => IDLE_SNAPSHOT

/**
 * 只在 `live` 时订阅每帧内容。历史行与流式行共用同一个组件位置（落库交接不重挂），
 * 历史行传 false：不订阅、恒得空闲快照，流式帧不会重渲它。
 */
export function useLiveStreamSnapshot(live: boolean): ConversationStreamSnapshot {
  return useSyncExternalStore(
    live ? subscribeSnapshot : subscribeNothing,
    live ? getSnapshot : getIdleSnapshot,
    live ? getSnapshot : getIdleSnapshot,
  )
}

/**
 * 订阅内容快照的派生值：只有 `isEqual` 判定变了才重渲。给只关心边沿信息
 * （消息 id、是否出现子代理等）的组件用，避免跟着每个 token 重渲。
 */
export function useStreamSelector<T>(
  selector: (snapshot: ConversationStreamSnapshot) => T,
  isEqual: (a: T, b: T) => boolean = Object.is,
): T {
  const cache = useRef<{ source: ConversationStreamSnapshot; value: T } | null>(null)
  const select = useCallback(() => {
    const source = snapshot
    const previous = cache.current
    if (previous?.source === source) return previous.value
    const next = selector(source)
    const value = previous && isEqual(previous.value, next) ? previous.value : next
    cache.current = { source, value }
    return value
  }, [isEqual, selector])
  return useSyncExternalStore(subscribeSnapshot, select, select)
}

export function useStreamCoarse(): StreamCoarse {
  return useSyncExternalStore(subscribeCoarse, getCoarse, getCoarse)
}
