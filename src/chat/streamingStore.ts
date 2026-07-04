import { useSyncExternalStore } from 'react'
import type { ConversationStreamSnapshot } from './conversationRuns'

// 流式預覽的高頻狀態從 Chat.tsx 的 useState 抽到這裡，用 React 內建 useSyncExternalStore 訂閱。
// 用外部 store（而非 Context）：60fps 下 Context 會重渲所有 consumer，store 只通知真正訂閱的元件。
//
// 切成兩個 slice，按更新頻率分開訂閱：
// - content：每幀都變（流式文字/推理/工具/分段），僅 MessageList 訂閱。
// - coarse：邊沿才變（streaming/frozen/cancelling/error 布林），Chat 的 showEmptyHero/drain 與
//   InputBar 的取消按鈕訂閱——避免它們被每幀的內容更新拖著重渲。

export interface StreamCoarse {
  streaming: boolean
  streamFrozen: boolean
  cancelling: boolean
  streamError: string
}

// 空閒態內容快照（streaming:false、內容全空）。與 createEmptyStreamSnapshot 不同——後者是
// 「起一輪新流」用的（streaming:true + startedAt:now）。這裡是 reset 的目標常量，引用恆定。
const IDLE_SNAPSHOT: ConversationStreamSnapshot = {
  runId: null,
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

// Chat 每幀傳入的是 streamSnapshotsRef 裡被原地 mutate 的同一個物件引用，必須淺複製出新引用，
// 否則 useSyncExternalStore 的 Object.is 比較偵測不到變化、不會重渲。
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

// 逐欄位淺比較：無實際變化則不分配新物件、不通知。否則流式中重複寫 {streaming:true} 仍會讓
// 訂閱者每幀重渲，等於沒隔離。
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

// 清空預覽：內容回空閒 + streaming/frozen/cancelling 歸位。**不動 streamError**——與
// Chat 原 clearStreamingPreview 語義一致（錯誤由 setStreamErrorForConversation/restore 獨立管理）。
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

export function useStreamCoarse(): StreamCoarse {
  return useSyncExternalStore(subscribeCoarse, getCoarse, getCoarse)
}
