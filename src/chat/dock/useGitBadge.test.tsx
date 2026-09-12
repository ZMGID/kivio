import { StrictMode } from 'react'
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { GitDiffChip } from './GitDiffChip'
import { GitStatusPill } from './GitStatusPill'
import type { GitRepoState, GitSnapshot, WorkspaceActivityEvent } from './types'

const mocks = vi.hoisted(() => ({
  gitStatus: vi.fn(),
  gitDiffStat: vi.fn(),
  gitInit: vi.fn(),
  gitSnapshot: vi.fn(),
  listeners: new Map<string, Set<(event: WorkspaceActivityEvent) => void>>(),
  subscribe: vi.fn(),
  available: true,
}))

vi.mock('./api', () => ({ dockApi: mocks }))
vi.mock('./workspaceActivity', () => ({
  workspaceActivity: {
    isAvailable: () => mocks.available,
    subscribe: (workdir: string, callback: (event: WorkspaceActivityEvent) => void) => {
      mocks.subscribe(workdir)
      const listeners = mocks.listeners.get(workdir) ?? new Set()
      listeners.add(callback)
      mocks.listeners.set(workdir, listeners)
      return () => {
        listeners.delete(callback)
        if (!listeners.size) mocks.listeners.delete(workdir)
      }
    },
  },
}))

const state: GitRepoState = {
  status: 'ready', repoRoot: '/repo', head: 'main', upstream: null,
  ahead: 0, behind: 0, stashCount: 0, entries: [], error: null,
}
const stat = { filesChanged: 1, additions: 3, deletions: 1, files: [] }

function Badges({ workdir = '/repo' }: { workdir?: string }) {
  return <>
    <GitStatusPill workdir={workdir} lang="en" onOpenGitPanel={() => {}} />
    <GitDiffChip workdir={workdir} lang="en" onOpenGitPanel={() => {}} />
  </>
}

function emit(workdir = '/repo') {
  for (const listener of mocks.listeners.get(workdir) ?? []) {
    listener({ workdir, fs: true, git: false, truncated: false, revision: 1, changedPaths: [] })
  }
}

beforeEach(() => {
  vi.resetAllMocks()
  mocks.available = true
  mocks.gitStatus.mockResolvedValue(state)
  mocks.gitDiffStat.mockResolvedValue(stat)
  mocks.gitSnapshot.mockImplementation(async (_workdir: string, includeDiffStat: boolean) => ({
    state, diffStat: includeDiffStat ? stat : null,
  }))
})

afterEach(async () => {
  cleanup()
  await act(async () => {})
  vi.useRealTimers()
})

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => { resolve = done })
  return { promise, resolve }
}

describe('Git badges resource sharing', () => {
  it('shares one refresh when both real consumers mount', async () => {
    render(<StrictMode><Badges /></StrictMode>)
    await screen.findByText('+3')
    expect(mocks.gitSnapshot).toHaveBeenCalledTimes(1)
    expect(mocks.gitSnapshot).toHaveBeenCalledWith('/repo', true)
    expect(mocks.gitStatus).not.toHaveBeenCalled()
    expect(mocks.gitDiffStat).not.toHaveBeenCalled()
    expect(mocks.subscribe).toHaveBeenCalledTimes(1)
  })

  it('shares one refresh for a workspace event', async () => {
    render(<Badges />)
    await screen.findByText('+3')
    mocks.gitSnapshot.mockClear()
    await act(async () => emit())
    expect(mocks.gitSnapshot).toHaveBeenCalledTimes(1)
    expect(mocks.gitSnapshot).toHaveBeenCalledWith('/repo', true)
  })

  it('does not calculate line statistics for a branch-only consumer', async () => {
    render(<GitStatusPill workdir="/repo" lang="en" onOpenGitPanel={() => {}} />)
    await screen.findByText('main')
    expect(mocks.gitDiffStat).not.toHaveBeenCalled()
    expect(mocks.gitSnapshot).toHaveBeenCalledTimes(1)
    expect(mocks.gitSnapshot).toHaveBeenCalledWith('/repo', false)
    fireEvent.click(screen.getByTitle('Git'))
    expect(screen.getByRole('dialog')).toBeInTheDocument()
  })

  it('serializes bursts during a query and refreshes the final edit', async () => {
    const pending = deferred<GitSnapshot>()
    mocks.gitSnapshot.mockReturnValueOnce(pending.promise)
    render(<Badges />)
    await waitFor(() => expect(mocks.gitSnapshot).toHaveBeenCalledTimes(1))
    await act(async () => {
      emit()
      emit()
      emit()
    })
    expect(mocks.gitSnapshot).toHaveBeenCalledTimes(1)
    mocks.gitSnapshot.mockResolvedValue({ state, diffStat: { ...stat, additions: 9 } })
    await act(async () => pending.resolve({ state, diffStat: stat }))
    expect(mocks.gitSnapshot).toHaveBeenCalledTimes(2)
    expect(screen.getByText('+9')).toBeInTheDocument()
  })

  it('does not let a previous workdir reply overwrite the new one', async () => {
    const pending = deferred<GitSnapshot>()
    mocks.gitSnapshot.mockReturnValueOnce(pending.promise)
    const view = render(<Badges />)
    await waitFor(() => expect(mocks.gitSnapshot).toHaveBeenCalledTimes(1))
    mocks.gitSnapshot.mockResolvedValue({ state: { ...state, head: 'other' }, diffStat: { ...stat, additions: 7 } })
    view.rerender(<Badges workdir="/other" />)
    await screen.findByText('other')
    await act(async () => pending.resolve({ state, diffStat: stat }))
    expect(screen.queryByText('main')).not.toBeInTheDocument()
    expect(screen.getByText('+7')).toBeInTheDocument()
    expect(mocks.listeners.has('/repo')).toBe(false)
  })

  it('shares fallback polling and releases it with the last consumer', async () => {
    vi.useFakeTimers()
    mocks.available = false
    const view = render(<Badges />)
    await act(async () => {})
    expect(mocks.gitSnapshot).toHaveBeenCalledTimes(1)
    await act(async () => vi.advanceTimersByTime(10_000))
    expect(mocks.gitSnapshot).toHaveBeenCalledTimes(2)
    view.unmount()
    await act(async () => {})
    expect(mocks.listeners.size).toBe(0)
    await act(async () => vi.advanceTimersByTime(20_000))
    expect(mocks.gitSnapshot).toHaveBeenCalledTimes(2)
  })

  it('adds diff statistics when the diff consumer arrives during a branch query', async () => {
    const pending = deferred<GitSnapshot>()
    mocks.gitSnapshot.mockReturnValueOnce(pending.promise)
    const view = render(<GitStatusPill workdir="/repo" lang="en" onOpenGitPanel={() => {}} />)
    await waitFor(() => expect(mocks.gitSnapshot).toHaveBeenCalledWith('/repo', false))
    view.rerender(<Badges />)
    await act(async () => pending.resolve({ state, diffStat: null }))
    expect(screen.getByText('+3')).toBeInTheDocument()
    expect(mocks.gitSnapshot).toHaveBeenLastCalledWith('/repo', true)
    expect(mocks.gitSnapshot).toHaveBeenCalledTimes(2)
  })

  it('reuses cached state for another branch consumer and drops unused diff work', async () => {
    const view = render(<Badges />)
    await screen.findByText('+3')
    view.rerender(<>
      <GitStatusPill workdir="/repo" lang="en" onOpenGitPanel={() => {}} />
      <GitStatusPill workdir="/repo" lang="en" onOpenGitPanel={() => {}} />
    </>)
    expect(mocks.gitSnapshot).toHaveBeenCalledTimes(1)
    await act(async () => emit())
    expect(mocks.gitSnapshot).toHaveBeenLastCalledWith('/repo', false)
  })

  it('retries after a failed query and does not query an empty workdir', async () => {
    mocks.gitSnapshot.mockRejectedValueOnce(new Error('unavailable'))
    const view = render(<Badges />)
    await act(async () => {})
    await act(async () => emit())
    expect(screen.getByText('+3')).toBeInTheDocument()
    view.rerender(<Badges workdir="" />)
    await act(async () => {})
    expect(mocks.gitSnapshot).toHaveBeenCalledTimes(2)
    expect(mocks.listeners.size).toBe(0)
    expect(screen.queryByText('+3')).not.toBeInTheDocument()
  })

  it('shares mutation state immediately and rejects a query started before the mutation', async () => {
    const pending = deferred<GitSnapshot>()
    const postMutation = deferred<GitSnapshot>()
    const noRepo = { ...state, status: 'not_repo' as const, head: '' }
    mocks.gitSnapshot.mockResolvedValueOnce({ state: noRepo, diffStat: null })
    mocks.gitSnapshot.mockReturnValueOnce(pending.promise).mockReturnValueOnce(postMutation.promise)
    mocks.gitInit.mockResolvedValue({ ok: true, state, stdout: '', stderr: '', message: '' })
    render(<Badges />)
    await act(async () => {})
    await act(async () => emit())
    fireEvent.click(screen.getByTitle('Git'))
    fireEvent.click(screen.getByText('Initialize repository'))
    await waitFor(() => expect(screen.getAllByText('main')).toHaveLength(2))
    await act(async () => pending.resolve({ state: noRepo, diffStat: null }))
    expect(screen.getAllByText('main')).toHaveLength(2)
    expect(screen.queryByText('Initialize repository')).not.toBeInTheDocument()
    await act(async () => postMutation.resolve({ state, diffStat: stat }))
    expect(screen.getByText('+3')).toBeInTheDocument()
    expect(mocks.gitSnapshot).toHaveBeenCalledTimes(3)
  })
})
