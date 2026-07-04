// Pyodide 沙盒的主執行緒客戶端：按需起一個 Worker 跑 Python，空閒一段時間後 terminate() 解除安裝，
// 把 Pyodide 的 WASM 記憶體（數百 MB、WASM 線性記憶體只增不減）整個還給 OS。對呼叫方保持與
// 舊 runPythonInSandbox 完全相同的簽名，Chat 側無需改動邏輯。
//
// 為什麼不在主執行緒「用完丟引用」：Pyodide 沒有 destroy，丟引用 + GC 也無法讓已增長的 WASM
// 線性記憶體還給 OS（與 WebKit 不歸還已釋放堆同理）。唯一可靠的回收 = 終結承載它的 worker。
import type { PythonInputFile, PythonRunOutcome } from './pyodideRunner'

// 空閒多久後解除安裝 worker。同一任務裡連續多步 Python 複用同一執行時、不過載；
// 使用者/agent 停止使用後釋放。設短了會頻繁過載（matplotlib/numpy 冷啟動數秒），設長了佔用久。
const IDLE_TERMINATE_MS = 60_000
// 主執行緒兜底超時 = 執行預算 + 冷載入預算。worker 內部已按 timeoutMs 限制執行，這裡再防 worker
// 整體卡死（如 pyodide 載入 hang）導致 Promise 永不 resolve。
const COLD_LOAD_BUDGET_MS = 120_000

interface PendingRun {
  resolve: (outcome: PythonRunOutcome) => void
  reject: (err: Error) => void
  guard: ReturnType<typeof setTimeout>
}

let worker: Worker | null = null
let idleTimer: ReturnType<typeof setTimeout> | null = null
let seq = 0
const pending = new Map<number, PendingRun>()

function terminateWorker() {
  if (idleTimer) {
    clearTimeout(idleTimer)
    idleTimer = null
  }
  worker?.terminate()
  worker = null
}

function scheduleIdleTerminate() {
  if (idleTimer) clearTimeout(idleTimer)
  idleTimer = null
  if (pending.size > 0) return
  idleTimer = setTimeout(() => {
    if (pending.size === 0) terminateWorker()
  }, IDLE_TERMINATE_MS)
}

function rejectAllPending(err: Error) {
  for (const [, run] of pending) {
    clearTimeout(run.guard)
    run.reject(err)
  }
  pending.clear()
}

function ensureWorker(): Worker {
  if (worker) return worker
  const next = new Worker(new URL('./pyodideWorker.ts', import.meta.url), { type: 'module' })
  next.onmessage = (event: MessageEvent<{ id: number; outcome?: PythonRunOutcome; error?: string }>) => {
    const { id, outcome, error } = event.data
    const run = pending.get(id)
    if (!run) return
    clearTimeout(run.guard)
    pending.delete(id)
    if (error) run.reject(new Error(error))
    else if (outcome) run.resolve(outcome)
    else run.reject(new Error('Python worker 返回了空結果'))
    scheduleIdleTerminate()
  }
  next.onerror = (event) => {
    // worker 整體崩潰：拒絕所有掛起任務並銷燬，下次呼叫重建。
    rejectAllPending(new Error(`Python worker 異常：${event.message || '未知錯誤'}`))
    terminateWorker()
  }
  worker = next
  return next
}

export function runPythonInSandbox(
  code: string,
  timeoutMs: number,
  files: PythonInputFile[] = [],
): Promise<PythonRunOutcome> {
  const target = ensureWorker()
  if (idleTimer) {
    clearTimeout(idleTimer)
    idleTimer = null
  }
  const id = ++seq
  return new Promise<PythonRunOutcome>((resolve, reject) => {
    const guard = setTimeout(() => {
      pending.delete(id)
      // 卡死則殺掉整個 worker（連同可能 hang 的 pyodide），下次呼叫冷重建。
      terminateWorker()
      reject(new Error('Python 執行超時：worker 無回應，已重置沙盒，請重試。'))
    }, timeoutMs + COLD_LOAD_BUDGET_MS)
    pending.set(id, { resolve, reject, guard })
    target.postMessage({ id, code, timeoutMs, files })
  })
}
