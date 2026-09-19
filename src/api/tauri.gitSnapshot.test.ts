import { beforeEach, describe, expect, it, vi } from 'vitest'
import { api } from './tauri'

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke }))

beforeEach(() => vi.resetAllMocks())

describe('Git snapshot IPC boundary', () => {
  it('requests optional statistics and normalizes both parts of the response', async () => {
    invoke.mockResolvedValue({
      state: { status: 'ready', repoRoot: '/repo', head: 'main', entries: [] },
      diffStat: { filesChanged: 1, additions: 2, deletions: 0, files: [{ path: 'a', additions: 2, deletions: 0 }] },
    })
    const snapshot = await api.dockGitSnapshot('/repo', true)
    expect(invoke).toHaveBeenCalledWith('dock_git_snapshot', { workdir: '/repo', includeDiffStat: true })
    expect(snapshot.state).toMatchObject({ status: 'ready', head: 'main', ahead: 0, entries: [] })
    expect(snapshot.diffStat).toEqual({ filesChanged: 1, additions: 2, deletions: 0, files: [{ path: 'a', additions: 2, deletions: 0 }] })
  })

  it('keeps absent statistics null for branch-only queries and diff failures', async () => {
    invoke.mockResolvedValue({ state: { status: 'ready', head: 'main' }, diffStat: null })
    const snapshot = await api.dockGitSnapshot('/repo')
    expect(invoke).toHaveBeenCalledWith('dock_git_snapshot', { workdir: '/repo', includeDiffStat: false })
    expect(snapshot.state.head).toBe('main')
    expect(snapshot.diffStat).toBeNull()
  })

  it('normalizes snake_case Git wire fields at the API boundary', async () => {
    invoke.mockResolvedValue({
      state: {
        status: 'ready', repo_root: '/repo', head: 'main', stash_count: 2,
        entries: [{ path: 'a.txt', index_status: 'M', worktree_status: ' ', staged: true }],
      },
      diffStat: {
        files_changed: 1, additions: 3, deletions: 1,
        files: [{ path: 'a.txt', additions: 3, deletions: 1 }],
      },
    })

    const snapshot = await api.dockGitSnapshot('/repo', true)
    expect(snapshot.state).toMatchObject({ repoRoot: '/repo', stashCount: 2 })
    expect(snapshot.state.entries[0]).toMatchObject({ indexStatus: 'M', worktreeStatus: ' ', staged: true })
    expect(snapshot.diffStat).toMatchObject({ filesChanged: 1, additions: 3, deletions: 1 })
  })
})
