import { useSyncExternalStore } from 'react'
import { createEmptyStreamSnapshot, type ConversationStreamSnapshot } from './conversationRuns'

// 多模型一問多答（任務 06-30）：單會話的「單條流」預覽仍走 streamingStore（零迴歸）；
// 當一條 user 訊息 fan-out 出 N 條併發 assistant 流時，N 條流靠後端事件裡的 messageId 區分，
// 在這裡按 messageId 二級聚合。MessageGroup 元件訂閱本 store 渲染並排多列的即時流。
//
// 設計要點：
// - 鍵是 messageId（後端為每個臂生成獨立 message_id/run_id），而非 conversationId。
// - 每個會話維護一個「活躍組」(activeGroup)：傳送多模型問題時建組、登記期望列數與共享 group_id，
//   收到屬於該會話的、未知 messageId 的流事件時按需建列。組結束（sendMessage 返回）時清掉。
// - 與 streamingStore 完全獨立：單模型路徑一行程式碼都不碰本 store。

export interface GroupColumnSnapshot extends ConversationStreamSnapshot {
  messageId: string
  providerId: string | null
  model: string | null
}

export interface ActiveGroupState {
  conversationId: string
  groupId: string
  // 期望的列數（= reply_models 數量），用於傳送時先佔位 N 個「思考中」骨架列。
  expectedColumns: number
  // 已知的列，按 messageId 收斂；首批為佔位列（無 messageId），隨流事件填充真實 messageId。
  columns: GroupColumnSnapshot[]
}

// conversationId → 活躍組。一個會話同一時刻最多一個活躍多答組（與「傳送時 busy 拒絕」一致）。
const activeGroups = new Map<string, ActiveGroupState>()

const subs = new Set<() => void>()

// 版本號：任何變更都自增，作為 getSnapshot 的穩定標識，避免每幀分配新物件。
let version = 0

// 結構性變更（建組/認領列/結束組）立即 emit；內容 delta（touchGroup）經 rAF 合幀，
// 避免 N 列各自高頻 setState（與單流 showStreamSnapshotIfCurrent 的合幀策略一致）。
function emit() {
  version += 1
  for (const cb of subs) cb()
}

// ---- 流式 delta 合幀（效能：任務 06-30 步驟 8 / R10）----
// 多答組流式時，每條流的每個 delta 都會 mutate 對應列並請求一次重渲。N 列併發下若每個
// delta 立即 emit()，訂閱者（MessageGroup × N + MessageList）會被每秒成百次 setState 打爆。
// 這裡把 touchGroup 的 emit 節流到每幀最多一次：delta 仍即時累積到列物件，只把「通知重渲」
// 合幀。done/結束等終止幀用 flushGroups 立即 flush。
let groupFlushRaf: number | null = null
let groupFlushPending = false

// 延遲解析全域 rAF（而非模組載入時繫結），讓測試能 stub requestAnimationFrame；
// 無 rAF 環境（部分測試/SSR）降級為 0ms 宏任務，仍保留合幀語義（同步累積、非同步通知）。
function rafSchedule(cb: () => void): number {
  if (typeof requestAnimationFrame === 'function') return requestAnimationFrame(cb)
  // ponytail: 無 rAF 時退化為 setTimeout，返回值統一當作不透明 handle。
  return setTimeout(cb, 0) as unknown as number
}

function rafCancel(handle: number): void {
  if (typeof cancelAnimationFrame === 'function') {
    cancelAnimationFrame(handle)
    return
  }
  clearTimeout(handle)
}

export function subscribeGroups(cb: () => void): () => void {
  subs.add(cb)
  return () => {
    subs.delete(cb)
  }
}

export function getGroupsVersion(): number {
  return version
}

function makeColumn(
  messageId: string,
  providerId: string | null,
  model: string | null,
): GroupColumnSnapshot {
  return {
    ...createEmptyStreamSnapshot(),
    messageId,
    providerId,
    model,
  }
}

/** 起一個多答組：登記會話、group_id、期望列數（先放 N 個佔位骨架列）。 */
export function beginGroup(
  conversationId: string,
  groupId: string,
  arms: { providerId: string | null; model: string | null }[],
): void {
  const columns = arms.map((arm, index) =>
    // 佔位列用一個臨時 messageId（pending-<index>），真實 message_id 到達後認領。
    makeColumn(`pending-${groupId}-${index}`, arm.providerId, arm.model),
  )
  activeGroups.set(conversationId, {
    conversationId,
    groupId,
    expectedColumns: arms.length,
    columns,
  })
  emit()
}

/** 該會話當前是否處於多答組流式中。 */
export function hasActiveGroup(conversationId: string): boolean {
  return activeGroups.has(conversationId)
}

export function getActiveGroup(conversationId: string): ActiveGroupState | undefined {
  return activeGroups.get(conversationId)
}

/**
 * 為某會話的活躍組認領/取得一條列快照（按 messageId）。
 * 若該 messageId 未知：優先認領一個尚未繫結真實 id 的佔位列（pending-*），否則新建一列。
 * 返回被原地 mutate 的列物件（呼叫方累積 delta 後呼叫 touchGroup 觸發重渲）。
 */
export function ensureGroupColumn(
  conversationId: string,
  messageId: string,
  providerId?: string | null,
  model?: string | null,
): GroupColumnSnapshot | null {
  const group = activeGroups.get(conversationId)
  if (!group) return null
  const existing = group.columns.find((col) => col.messageId === messageId)
  if (existing) {
    if (providerId != null && existing.providerId == null) existing.providerId = providerId
    if (model != null && existing.model == null) existing.model = model
    return existing
  }
  // 認領一個還沒綁真實 message_id 的佔位列。
  const pending = group.columns.find((col) => col.messageId.startsWith(`pending-${group.groupId}-`))
  if (pending) {
    pending.messageId = messageId
    if (providerId != null) pending.providerId = providerId
    if (model != null) pending.model = model
    return pending
  }
  // 沒有佔位列可認領（實際臂數 > 期望，理論上不會發生）：直接追加一列。
  const col = makeColumn(messageId, providerId ?? null, model ?? null)
  group.columns.push(col)
  return col
}

/** 列內容被原地 mutate 後，請求一次重渲（rAF 合幀：每幀最多通知訂閱者一次）。 */
export function touchGroup(): void {
  groupFlushPending = true
  if (groupFlushRaf != null) return
  groupFlushRaf = rafSchedule(() => {
    groupFlushRaf = null
    if (!groupFlushPending) return
    groupFlushPending = false
    emit()
  })
}

/** 立即 flush 待合幀的內容更新（done / 結束 / 測試需要同步可見時呼叫）。 */
export function flushGroups(): void {
  if (groupFlushRaf != null) {
    rafCancel(groupFlushRaf)
    groupFlushRaf = null
  }
  if (!groupFlushPending) return
  groupFlushPending = false
  emit()
}

/** 結束並清掉某會話的活躍組（sendMessage 返回 / 錯誤 / 取消時呼叫）。 */
export function endGroup(conversationId: string): void {
  flushGroups()
  if (activeGroups.delete(conversationId)) {
    emit()
  }
}

/** 清空所有活躍組（解除安裝 / 全域重置）。 */
export function resetGroups(): void {
  if (groupFlushRaf != null) {
    rafCancel(groupFlushRaf)
    groupFlushRaf = null
  }
  groupFlushPending = false
  if (activeGroups.size > 0) {
    activeGroups.clear()
    emit()
  }
}

// ---- React hook ----

// 訂閱版本號即可：元件讀 getActiveGroup(conversationId) 取最新列（mutate 的同一引用），
// 版本號變化驅動重渲。MessageGroup 內部據列內容渲染，無需快照複製。
export function useGroupsVersion(): number {
  return useSyncExternalStore(subscribeGroups, getGroupsVersion, getGroupsVersion)
}
