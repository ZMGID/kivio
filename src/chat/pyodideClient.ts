// Pyodide 沙盒的主线程客户端：按需起一个 Worker 跑 Python，空闲一段时间后 terminate() 卸载，
// 把 Pyodide 的 WASM 内存（数百 MB、WASM 线性内存只增不减）整个还给 OS。对调用方保持与
// 旧 runPythonInSandbox 完全相同的签名，Chat 侧无需改动逻辑。
//
// 为什么不在主线程「用完丢引用」：Pyodide 没有 destroy，丢引用 + GC 也无法让已增长的 WASM
// 线性内存还给 OS（与 WebKit 不归还已释放堆同理）。唯一可靠的回收 = 终结承载它的 worker。
import type { PythonInputFile, PythonRunOutcome } from './pyodideRunner'

// 空闲多久后卸载 worker。同一任务里连续多步 Python 复用同一运行时、不重载；
// 用户/agent 停止使用后释放。设短了会频繁重载（matplotlib/numpy 冷启动数秒），设长了占用久。
const IDLE_TERMINATE_MS = 60_000
// 主线程兜底超时 = 执行预算 + 冷加载预算。worker 内部已按 timeoutMs 限制执行，这里再防 worker
// 整体卡死（如 pyodide 加载 hang）导致 Promise 永不 resolve。
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
    else run.reject(new Error('Python worker 返回了空结果'))
    scheduleIdleTerminate()
  }
  next.onerror = (event) => {
    // worker 整体崩溃：拒绝所有挂起任务并销毁，下次调用重建。
    rejectAllPending(new Error(`Python worker 异常：${event.message || '未知错误'}`))
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
      // 卡死则杀掉整个 worker（连同可能 hang 的 pyodide），下次调用冷重建。
      terminateWorker()
      reject(new Error('Python 执行超时：worker 无响应，已重置沙盒，请重试。'))
    }, timeoutMs + COLD_LOAD_BUDGET_MS)
    pending.set(id, { resolve, reject, guard })
    target.postMessage({ id, code, timeoutMs, files })
  })
}
