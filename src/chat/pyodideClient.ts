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
// Worker 内部的 timeoutMs 只覆盖 Python 执行；主线程额外给首次加载一个短宽限。
// Rust 侧等待预算为 timeoutMs + 12s，始终晚于这里 2s，避免后端先返回而 Worker 继续占内存。
const COLD_LOAD_GRACE_MS = 10_000

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

function resetWorker(err: Error) {
  rejectAllPending(err)
  terminateWorker()
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
    resetWorker(new Error(`Python worker 异常：${event.message || '未知错误'}`))
  }
  next.onmessageerror = () => {
    resetWorker(new Error('Python worker 返回了无法解析的消息，已重置沙盒。'))
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
      // Pyodide 无法安全中断单次执行。卡死时必须重置整个 Worker，并立即
      // 拒绝所有并发 pending，不能让其他 Promise 等到各自的 guard 才结束。
      resetWorker(new Error('Python 执行超时：worker 无响应，已重置沙盒，请重试。'))
    }, timeoutMs + COLD_LOAD_GRACE_MS)
    pending.set(id, { resolve, reject, guard })
    try {
      target.postMessage({ id, code, timeoutMs, files })
    } catch (err) {
      clearTimeout(guard)
      pending.delete(id)
      scheduleIdleTerminate()
      reject(err instanceof Error ? err : new Error(String(err)))
    }
  })
}

/** Chat/WebView teardown 时主动释放 Pyodide WASM 内存并结束所有挂起调用。 */
export function disposePythonSandbox() {
  resetWorker(new Error('Python 沙盒已关闭。'))
}
