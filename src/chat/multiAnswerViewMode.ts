import { useCallback, useSyncExternalStore } from 'react'

// 多模型一問多答（任務 06-30）：多答組的展示模式偏好。
//  - 'tabs'（切換，預設）：一次只整寬顯示一條答案，組末尾 footer 切換顯示哪條。
//  - 'columns'（並排）：N 列橫向並排（原有實現）。
// 這是一個**全域 UI 偏好**：跨會話共用、重啟保留，寫在 localStorage，不進後端 settings。
// 同一視窗內多個訂閱者（多個多答組的 footer）通過模組級 store 即時同步；其它視窗/標籤頁
// 通過 storage 事件同步。

export type MultiAnswerViewMode = 'tabs' | 'columns'

export const MULTI_ANSWER_VIEW_STORAGE_KEY = 'kivio.chat.multiAnswerView'

const DEFAULT_MODE: MultiAnswerViewMode = 'tabs'

function isValidMode(value: string | null): value is MultiAnswerViewMode {
  return value === 'tabs' || value === 'columns'
}

function readFromStorage(): MultiAnswerViewMode {
  if (typeof window === 'undefined') return DEFAULT_MODE
  try {
    const raw = window.localStorage.getItem(MULTI_ANSWER_VIEW_STORAGE_KEY)
    return isValidMode(raw) ? raw : DEFAULT_MODE
  } catch {
    // 隱私模式 / 儲存被停用 → 退回預設。
    return DEFAULT_MODE
  }
}

// 模組級快取 + 訂閱者集合：getSnapshot 必須返回穩定引用，故快取當前值，僅在變化時更新。
let current: MultiAnswerViewMode = readFromStorage()
const subscribers = new Set<() => void>()

function emit() {
  for (const cb of subscribers) cb()
}

function setMode(next: MultiAnswerViewMode) {
  if (next === current) return
  current = next
  if (typeof window !== 'undefined') {
    try {
      window.localStorage.setItem(MULTI_ANSWER_VIEW_STORAGE_KEY, next)
    } catch {
      // 寫失敗（隱私模式）忽略：記憶體態仍生效到本會話視窗關閉。
    }
  }
  emit()
}

function subscribe(cb: () => void): () => void {
  subscribers.add(cb)
  // 跨視窗/標籤頁同步：另一視窗改了偏好 → storage 事件 → 重讀並通知本視窗訂閱者。
  const onStorage = (e: StorageEvent) => {
    if (e.key !== MULTI_ANSWER_VIEW_STORAGE_KEY) return
    const next = readFromStorage()
    if (next !== current) {
      current = next
      emit()
    }
  }
  if (typeof window !== 'undefined') {
    window.addEventListener('storage', onStorage)
  }
  return () => {
    subscribers.delete(cb)
    if (typeof window !== 'undefined') {
      window.removeEventListener('storage', onStorage)
    }
  }
}

function getSnapshot(): MultiAnswerViewMode {
  return current
}

/**
 * 多答組展示模式偏好（全域，跨會話）。返回 `[mode, setMode]`。
 * 預設 'tabs'（切換）。改動寫 localStorage 並即時同步本視窗所有訂閱者。
 */
export function useMultiAnswerViewMode(): [MultiAnswerViewMode, (mode: MultiAnswerViewMode) => void] {
  const mode = useSyncExternalStore(subscribe, getSnapshot, getSnapshot)
  const set = useCallback((next: MultiAnswerViewMode) => setMode(next), [])
  return [mode, set]
}

// 測試輔助：直接重置記憶體態 + storage，避免用例間偏好串味。
export function _resetMultiAnswerViewModeForTest(): void {
  current = DEFAULT_MODE
  if (typeof window !== 'undefined') {
    try {
      window.localStorage.removeItem(MULTI_ANSWER_VIEW_STORAGE_KEY)
    } catch {
      // ignore
    }
  }
  emit()
}

// 測試輔助：直接置某模式（同步記憶體態 + storage + 通知訂閱者）。
export function _setMultiAnswerViewModeForTest(mode: MultiAnswerViewMode): void {
  current = mode
  if (typeof window !== 'undefined') {
    try {
      window.localStorage.setItem(MULTI_ANSWER_VIEW_STORAGE_KEY, mode)
    } catch {
      // ignore
    }
  }
  emit()
}
