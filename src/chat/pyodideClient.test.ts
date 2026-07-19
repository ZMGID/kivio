import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

type WorkerMessage = { id: number; outcome?: { content: string; isError: boolean; artifacts: [] }; error?: string }

class MockWorker {
  static instances: MockWorker[] = []

  onmessage: ((event: MessageEvent<WorkerMessage>) => void) | null = null
  onerror: ((event: ErrorEvent) => void) | null = null
  onmessageerror: ((event: MessageEvent) => void) | null = null
  postMessage = vi.fn()
  terminate = vi.fn()

  constructor() {
    MockWorker.instances.push(this)
  }

  emitMessage(data: WorkerMessage) {
    this.onmessage?.({ data } as MessageEvent<WorkerMessage>)
  }

  emitError(message: string) {
    this.onerror?.({ message } as ErrorEvent)
  }
}

beforeEach(() => {
  vi.useFakeTimers()
  vi.resetModules()
  MockWorker.instances = []
  vi.stubGlobal('Worker', MockWorker)
})

afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('pyodideClient worker lifecycle', () => {
  it('terminates an idle worker after a completed run', async () => {
    const { runPythonInSandbox } = await import('./pyodideClient')
    const run = runPythonInSandbox('print(1)', 1_000)
    const instance = MockWorker.instances[0]
    const id = (instance.postMessage.mock.calls[0][0] as { id: number }).id

    instance.emitMessage({ id, outcome: { content: '1', isError: false, artifacts: [] } })
    await expect(run).resolves.toMatchObject({ content: '1', isError: false })

    await vi.advanceTimersByTimeAsync(60_000)
    expect(instance.terminate).toHaveBeenCalledOnce()
  })

  it('rejects every pending run on timeout and rebuilds the worker', async () => {
    const { runPythonInSandbox } = await import('./pyodideClient')
    const first = runPythonInSandbox('while True: pass', 1_000)
    const second = runPythonInSandbox('print(2)', 1_000)
    const original = MockWorker.instances[0]
    const firstRejected = expect(first).rejects.toThrow('已重置沙盒')
    const secondRejected = expect(second).rejects.toThrow('已重置沙盒')

    await vi.advanceTimersByTimeAsync(11_000)
    await Promise.all([firstRejected, secondRejected])
    expect(original.terminate).toHaveBeenCalledOnce()

    const third = runPythonInSandbox('print(3)', 1_000)
    expect(MockWorker.instances).toHaveLength(2)
    const replacement = MockWorker.instances[1]
    const id = (replacement.postMessage.mock.calls[0][0] as { id: number }).id
    replacement.emitMessage({ id, outcome: { content: '3', isError: false, artifacts: [] } })
    await expect(third).resolves.toMatchObject({ content: '3' })
  })

  it('uses the same reset path for worker errors and explicit disposal', async () => {
    const client = await import('./pyodideClient')
    const crashed = client.runPythonInSandbox('print(1)', 1_000)
    MockWorker.instances[0].emitError('boom')
    await expect(crashed).rejects.toThrow('boom')
    expect(MockWorker.instances[0].terminate).toHaveBeenCalledOnce()

    const disposed = client.runPythonInSandbox('print(2)', 1_000)
    const replacement = MockWorker.instances[1]
    client.disposePythonSandbox()
    await expect(disposed).rejects.toThrow('沙盒已关闭')
    expect(replacement.terminate).toHaveBeenCalledOnce()

    client.disposePythonSandbox()
    expect(replacement.terminate).toHaveBeenCalledOnce()
  })
})
