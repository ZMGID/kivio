import { act, cleanup, renderHook, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { useGitReview } from './useGitReview'
import type { GitRepoState, WorkspaceActivityEvent } from '../../api/dockContracts'

const mocks = vi.hoisted(() => ({
  gitStatus: vi.fn(), gitDiffStat: vi.fn(),
  listeners: new Map<string, (event: WorkspaceActivityEvent) => void>(),
}))
vi.mock('./api', () => ({ dockApi: mocks }))
vi.mock('./workspaceActivity', () => ({ workspaceActivity: {
  isAvailable: () => true,
  subscribe: (dir: string, listener: (event: WorkspaceActivityEvent) => void) => {
    mocks.listeners.set(dir, listener)
    return () => { mocks.listeners.delete(dir) }
  },
} }))
const state: GitRepoState = {
  status: 'ready', repoRoot: '/a', head: 'main', upstream: null,
  ahead: 0, behind: 0, stashCount: 0, entries: [], error: null,
}
function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: Error) => void
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}
function emit(dir = '/a') {
  mocks.listeners.get(dir)?.({ workdir: dir, fs: true, git: false, truncated: false, revision: 1, changedPaths: [] })
}
beforeEach(() => {
  vi.resetAllMocks()
  mocks.gitStatus.mockResolvedValue(state)
  mocks.gitDiffStat.mockResolvedValue({ files: [] })
})
afterEach(cleanup)

it('coalesces edits into one followup query and settles to the newest result', async () => {
  const first = deferred<GitRepoState>()
  const last = deferred<GitRepoState>()
  mocks.gitStatus.mockReturnValueOnce(first.promise).mockReturnValueOnce(last.promise)
  const { result } = renderHook(() => useGitReview({ workdir: '/a', active: true }))
  expect(result.current.statusLoading).toBe(true)
  act(() => { for (let i = 0; i < 8; i++) emit() })
  expect(mocks.gitStatus).toHaveBeenCalledTimes(1)
  await act(async () => first.resolve(state))
  await waitFor(() => expect(mocks.gitStatus).toHaveBeenCalledTimes(2))
  await act(async () => last.resolve({ ...state, head: 'updated' }))
  expect(result.current.status?.head).toBe('updated')
  expect(result.current.statusLoading).toBe(false)
  expect(mocks.gitStatus).toHaveBeenCalledTimes(2)
})

it('does not lose a refresh arriving as the previous query completes', async () => {
  const first = deferred<GitRepoState>()
  mocks.gitStatus.mockReturnValueOnce(first.promise)
  const { result } = renderHook(() => useGitReview({ workdir: '/a', active: true }))
  const nextRefresh = first.promise.then(() => result.current.refresh())
  await act(async () => {
    first.resolve({ ...state, status: 'not_repo' })
    await nextRefresh
  })
  expect(result.current.status?.status).toBe('ready')
  expect(result.current.statusLoading).toBe(false)
})

it('isolates late A results across A to B to A navigation, and retries after errors', async () => {
  const old = deferred<GitRepoState>()
  mocks.gitStatus.mockReturnValueOnce(old.promise)
  const { result, rerender } = renderHook(({ workdir }) => useGitReview({ workdir, active: true }), { initialProps: { workdir: '/a' } })
  rerender({ workdir: '/b' })
  await act(async () => {})
  rerender({ workdir: '/a' })
  await act(async () => {})
  await act(async () => old.resolve({ ...state, head: 'stale' }))
  await waitFor(() => expect(result.current.status?.head).toBe('main'))
  mocks.gitStatus.mockRejectedValueOnce(new Error('offline'))
  await act(async () => { await result.current.refresh() })
  expect(result.current.viewError).toBe('offline')
  await act(async () => { await result.current.refresh() })
  expect(result.current.viewError).toBe('')
})

it('a mutation retires pre-mutation status and refreshes line counts even with the same file list', async () => {
  const pending = deferred<GitRepoState>()
  mocks.gitStatus.mockReturnValueOnce(pending.promise)
  const { result } = renderHook(() => useGitReview({ workdir: '/a', active: true }))
  await act(async () => { await result.current.runMutation(async () => ({ ok: true, state: { ...state, head: 'committed' }, stdout: '', stderr: '', message: '' })) })
  expect(result.current.status?.head).toBe('committed')
  await act(async () => pending.resolve({ ...state, head: 'stale' }))
  await waitFor(() => expect(result.current.status?.head).toBe('main'))
  mocks.gitDiffStat.mockResolvedValue({ files: [{ path: 'a.txt', additions: 4, deletions: 1 }] })
  await act(async () => { await result.current.refresh() })
  expect(result.current.fileStats['a.txt'].additions).toBe(4)
})
