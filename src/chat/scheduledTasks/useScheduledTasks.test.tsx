import { act, cleanup, renderHook } from '@testing-library/react'
import { StrictMode, type ReactNode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { ScheduledTask, ScheduledTaskRun, ScheduledTasksChangedEvent, ScheduledTasksSnapshot } from '../../api/scheduledTaskContracts'
import type { api as ApiContract } from '../../api/tauri'
import type { useScheduledTasks as UseScheduledTasks } from './useScheduledTasks'

let api: typeof ApiContract
let useScheduledTasks: typeof UseScheduledTasks

vi.mock('../../api/tauri', () => ({
  api: {
    scheduledTasksSnapshot: vi.fn(),
    onScheduledTasksChanged: vi.fn(),
  },
}))

type Deferred<T> = {
  promise: Promise<T>
  resolve: (value: T) => void
  reject: (reason?: unknown) => void
}

function deferred<T>(): Deferred<T> {
  let resolve!: Deferred<T>['resolve']
  let reject!: Deferred<T>['reject']
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

function task(over: Partial<ScheduledTask> = {}): ScheduledTask {
  return {
    id: 'task-a',
    name: 'Digest',
    prompt: 'Summarize',
    schedule: { kind: 'daily', hour: 9, minute: 0 },
    conversationId: 'conv',
    enabled: true,
    status: 'active',
    nextRunAt: 200,
    lastRunAt: null,
    runCount: 1,
    lastError: null,
    source: 'user',
    createdAt: 1,
    updatedAt: 1,
    ...over,
  }
}

function run(over: Partial<ScheduledTaskRun> = {}): ScheduledTaskRun {
  return {
    id: 'run-1',
    taskId: 'task-a',
    trigger: 'schedule',
    scheduledAt: 100,
    status: 'running',
    conversationId: 'conv',
    error: null,
    createdAt: 10,
    startedAt: 11,
    finishedAt: null,
    ...over,
  }
}

const pendingListens: Array<(dispose: () => void) => void> = []
const disposes: Array<ReturnType<typeof vi.fn>> = []
const handlers: Array<(event: ScheduledTasksChangedEvent) => void> = []
const snapshots: Array<Deferred<ScheduledTasksSnapshot>> = []

function wire() {
  pendingListens.length = 0
  disposes.length = 0
  handlers.length = 0
  snapshots.length = 0
  vi.mocked(api.onScheduledTasksChanged).mockImplementation(handler => {
    handlers.push(handler)
    const dispose = vi.fn()
    disposes.push(dispose)
    return new Promise<() => void>(resolve => { pendingListens.push(resolve) })
  })
  vi.mocked(api.scheduledTasksSnapshot).mockImplementation(() => {
    const next = deferred<ScheduledTasksSnapshot>()
    snapshots.push(next)
    return next.promise
  })
}

beforeEach(async () => {
  // Each case exercises fresh module-owned subscription state; static imports retain prior cases.
  vi.resetModules()
  api = (await import('../../api/tauri')).api
  useScheduledTasks = (await import('./useScheduledTasks')).useScheduledTasks
  vi.mocked(api.scheduledTasksSnapshot).mockClear()
  vi.mocked(api.onScheduledTasksChanged).mockClear()
  wire()
})

afterEach(async () => {
  cleanup()
  await act(async () => { await Promise.resolve() })
  await act(async () => {
    for (const release of pendingListens.splice(0)) release(() => {})
    await Promise.resolve()
    for (const pending of snapshots.splice(0)) pending.resolve({ tasks: [], liveRuns: [] })
    await Promise.resolve()
  })
})

async function connect(index = 0) {
  await act(async () => { await Promise.resolve() })
  const release = pendingListens[index]
  const dispose = disposes[index]
  if (!release || !dispose) throw new Error(`listener ${index} was not requested`)
  await act(async () => { release(dispose) })
}

async function deliver(index: number, payload: ScheduledTasksSnapshot) {
  const pending = snapshots[index]
  if (!pending) throw new Error(`snapshot ${index} was not requested`)
  await act(async () => {
    pending.resolve(payload)
    await pending.promise
  })
}

function Strict({ children }: { children: ReactNode }) {
  return <StrictMode>{children}</StrictMode>
}

describe('useScheduledTasks', () => {
  it('restores queued and running work only after the listener is registered', async () => {
    const { result } = renderHook(() => useScheduledTasks())
    expect(api.scheduledTasksSnapshot).not.toHaveBeenCalled()

    await connect()
    expect(api.scheduledTasksSnapshot).toHaveBeenCalledTimes(1)
    await deliver(0, {
      tasks: [task({ revision: 2 }), task({ id: 'task-b', name: 'Other', conversationId: 'conv-b' })],
      liveRuns: [
        run({ id: 'digest-running', status: 'running', createdAt: 50 }),
        run({ id: 'digest-queued', status: 'queued', createdAt: 5 }),
        run({ id: 'done', status: 'succeeded', createdAt: 100, finishedAt: 101 }),
        run({ id: 'other-running', taskId: 'task-b', status: 'running', createdAt: 8 }),
        run({ id: 'other-queued', taskId: 'task-b', status: 'queued', createdAt: 2 }),
        run({ id: 'orphan', taskId: 'missing', status: 'running' }),
      ],
    })

    expect(result.current.error).toBe('')
    expect(result.current.tasks?.map(item => item.id)).toEqual(['task-a', 'task-b'])
    expect(result.current.tasks?.[0].revision).toBe(2)
    expect(result.current.tasks?.[1].revision).toBeUndefined()
    expect(result.current.liveRuns.get('task-a')).toMatchObject({ id: 'digest-queued', status: 'queued' })
    expect(result.current.liveRuns.get('task-b')).toMatchObject({ id: 'other-queued', status: 'queued' })
    expect(result.current.liveRuns.has('missing')).toBe(false)
    expect([...result.current.liveRuns.values()].map(item => item.id)).not.toContain('done')
  })

  it('keeps a run event that arrives before the startup snapshot', async () => {
    const { result } = renderHook(() => useScheduledTasks())
    await connect()
    const queued = run({ id: 'fresh', status: 'queued', createdAt: 3 })
    act(() => { handlers[0]({ taskId: 'task-a', run: queued }) })

    await deliver(0, {
      tasks: [task({ name: 'STALE' })],
      liveRuns: [run({ id: 'old', status: 'running', createdAt: 1 })],
    })
    expect(result.current.tasks).toBeNull()
    expect(result.current.liveRuns.get('task-a')?.id).toBe('fresh')
    expect(snapshots).toHaveLength(2)

    await deliver(1, { tasks: [task()], liveRuns: [queued] })
    expect(result.current.tasks).toEqual([task()])
    expect(result.current.liveRuns.get('task-a')?.id).toBe('fresh')
    expect(result.current.error).toBe('')
  })

  it('keeps the other live run when one ends and a snapshot from before that event arrives', async () => {
    const running = run({ id: 'running', status: 'running', createdAt: 10 })
    const queued = run({ id: 'queued', status: 'queued', createdAt: 20 })
    const { result } = renderHook(() => useScheduledTasks())
    await connect()
    await deliver(0, { tasks: [task()], liveRuns: [running, queued] })
    expect(result.current.liveRuns.get('task-a')?.id).toBe('queued')

    act(() => { handlers[0]({ taskId: 'task-a', run: { ...queued, status: 'skipped', finishedAt: 25 } }) })
    expect(result.current.liveRuns.get('task-a')?.id).toBe('running')
    await deliver(1, { tasks: [task()], liveRuns: [running] })
    expect(result.current.liveRuns.get('task-a')?.id).toBe('running')

    const nextQueued = run({ id: 'queued-2', status: 'queued', createdAt: 30 })
    act(() => { handlers[0]({ taskId: 'task-a', run: nextQueued }) })
    await deliver(2, { tasks: [task()], liveRuns: [running, nextQueued] })
    expect(result.current.liveRuns.get('task-a')?.id).toBe('queued-2')

    act(() => { result.current.refresh() })
    expect(snapshots).toHaveLength(4)
    act(() => { handlers[0]({ taskId: 'task-a', run: { ...running, status: 'succeeded', finishedAt: 40 } }) })
    expect(result.current.liveRuns.get('task-a')?.id).toBe('queued-2')

    await deliver(3, { tasks: [task({ name: 'STALE' })], liveRuns: [running] })
    expect(result.current.tasks?.[0].name).toBe('Digest')
    expect(result.current.liveRuns.get('task-a')?.id).toBe('queued-2')

    await deliver(4, { tasks: [task()], liveRuns: [nextQueued] })
    expect(result.current.tasks?.[0].name).toBe('Digest')
    expect(result.current.liveRuns.get('task-a')).toMatchObject({ id: 'queued-2', status: 'queued' })
  })

  it('does not let a snapshot issued before a delete event restore that task', async () => {
    const { result } = renderHook(() => useScheduledTasks())
    await connect()
    await deliver(0, { tasks: [task()], liveRuns: [run({ id: 'live', status: 'running' })] })

    act(() => { result.current.refresh() })
    act(() => { handlers[0]({ taskId: 'task-a', run: null }) })
    await deliver(1, {
      tasks: [task({ name: 'STALE' })],
      liveRuns: [run({ id: 'ghost', status: 'queued', createdAt: 99 })],
    })
    expect(result.current.tasks?.map(item => item.name)).toEqual(['Digest'])
    expect(result.current.liveRuns.get('task-a')?.id).toBe('live')

    await deliver(2, { tasks: [], liveRuns: [] })
    expect(result.current.tasks).toEqual([])
    expect(result.current.liveRuns.has('task-a')).toBe(false)
  })

  it('drops the unmounted generation and restores activity on the next subscription', async () => {
    const first = renderHook(() => useScheduledTasks())
    await connect(0)
    await deliver(0, { tasks: [task({ name: 'Live' })], liveRuns: [run({ id: 'live', status: 'running' })] })
    expect(first.result.current.liveRuns.get('task-a')?.id).toBe('live')

    act(() => { first.result.current.refresh() })
    const stale = snapshots.length - 1
    first.unmount()
    await act(async () => { await Promise.resolve() })
    expect(disposes[0]).toHaveBeenCalledTimes(1)

    const second = renderHook(() => useScheduledTasks())
    await connect(1)
    const fresh = snapshots.length - 1
    const queued = run({ id: 'next', status: 'queued', createdAt: 4 })
    await deliver(fresh, { tasks: [task({ name: 'Back' })], liveRuns: [queued] })
    expect(second.result.current.tasks?.map(item => item.name)).toEqual(['Back'])
    expect(second.result.current.liveRuns.get('task-a')?.id).toBe('next')

    await deliver(stale, { tasks: [task({ name: 'STALE' })], liveRuns: [run({ id: 'stale', status: 'running' })] })
    expect(second.result.current.tasks?.map(item => item.name)).toEqual(['Back'])
    expect(second.result.current.liveRuns.get('task-a')?.id).toBe('next')

    act(() => { handlers[0]({ taskId: 'task-a', run: run({ id: 'from-old', status: 'queued', createdAt: 100 }) }) })
    expect(second.result.current.tasks?.map(item => item.name)).toEqual(['Back'])
    expect(second.result.current.liveRuns.get('task-a')?.id).toBe('next')
  })

  it('shares one listener across a strict-mode resubscribe', async () => {
    const { result } = renderHook(() => useScheduledTasks(), { wrapper: Strict })
    await connect()
    await deliver(0, { tasks: [task()], liveRuns: [run({ id: 'live', status: 'queued' })] })
    expect(api.onScheduledTasksChanged).toHaveBeenCalledTimes(1)
    expect(result.current.liveRuns.get('task-a')?.status).toBe('queued')
    expect(disposes[0]).not.toHaveBeenCalled()
  })

  it.each(['rejects', 'throws'] as const)('reports a listener that %s, then recovers on refresh', async mode => {
    if (mode === 'rejects') {
      vi.mocked(api.onScheduledTasksChanged).mockRejectedValueOnce(new Error('no bus'))
    } else {
      vi.mocked(api.onScheduledTasksChanged).mockImplementationOnce(() => { throw new Error('no bus') })
    }
    const { result } = renderHook(() => useScheduledTasks())
    await act(async () => { await Promise.resolve() })
    expect(result.current.error).toContain('no bus')
    expect(result.current.tasks).toBeNull()
    expect(api.scheduledTasksSnapshot).not.toHaveBeenCalled()

    act(() => { result.current.refresh() })
    expect(api.scheduledTasksSnapshot).not.toHaveBeenCalled()
    await connect()
    await deliver(0, { tasks: [task()], liveRuns: [] })
    expect(result.current.error).toBe('')
    expect(result.current.tasks).toEqual([task()])
  })

  it('ignores a stale snapshot failure and lets a later refresh replace the error', async () => {
    const { result } = renderHook(() => useScheduledTasks())
    await connect()
    const queued = run({ id: 'queued', status: 'queued' })
    act(() => { handlers[0]({ taskId: 'task-a', run: queued }) })
    await act(async () => {
      snapshots[0].reject(new Error('disk'))
      await snapshots[0].promise.catch(() => undefined)
    })
    expect(result.current.error).toBe('')
    expect(result.current.tasks).toBeNull()
    expect(result.current.liveRuns.get('task-a')?.id).toBe('queued')

    await deliver(1, { tasks: [task()], liveRuns: [queued] })
    expect(result.current.tasks).toEqual([task()])
    expect(result.current.error).toBe('')

    act(() => { result.current.refresh() })
    await act(async () => {
      snapshots[2].reject(new Error('offline'))
      await snapshots[2].promise.catch(() => undefined)
    })
    expect(result.current.error).toContain('offline')
    expect(result.current.tasks).toEqual([task()])
    expect(result.current.liveRuns.get('task-a')?.id).toBe('queued')

    act(() => { result.current.refresh() })
    await deliver(3, { tasks: [task({ name: 'Fresh' })], liveRuns: [] })
    expect(result.current.error).toBe('')
    expect(result.current.tasks?.[0].name).toBe('Fresh')
    expect(result.current.liveRuns.has('task-a')).toBe(false)
  })
})
