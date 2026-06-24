/// <reference lib="webworker" />
// Pyodide 执行 Worker：把整个 Python 沙盒运行时关在这里。主线程用完后 terminate() 本 worker，
// 即可把 Pyodide 的 WASM 线性内存（matplotlib/numpy 跑后可达数百 MB、且只增不减）整个还给 OS——
// 这是在不关闭 chat 窗口的前提下唯一能真正回收 Pyodide 内存的办法。
import { runPythonInSandbox, type PythonInputFile } from './pyodideRunner'

interface RunRequest {
  id: number
  code: string
  timeoutMs: number
  files: PythonInputFile[]
}

self.onmessage = async (event: MessageEvent<RunRequest>) => {
  const { id, code, timeoutMs, files } = event.data
  try {
    const outcome = await runPythonInSandbox(code, timeoutMs, files ?? [])
    self.postMessage({ id, outcome })
  } catch (err) {
    // runPythonInSandbox 内部已兜底返回 isError 结果；这里只防御它自身抛出的意外。
    const message = err instanceof Error ? err.message : String(err)
    self.postMessage({ id, error: message })
  }
}
