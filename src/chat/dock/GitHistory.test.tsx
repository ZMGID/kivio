import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { ReactNode } from 'react'
import { GitHistory } from './GitHistory'
import { dockApi } from './api'
import type { GitLogResult } from '../../api/dockContracts'

vi.mock('virtua', () => ({ VList: ({ children }: { children: ReactNode }) => <div>{children}</div> }))
vi.mock('./workspaceActivity', () => ({ workspaceActivity: { subscribe: () => () => {}, isAvailable: () => true } }))
vi.mock('./api', () => ({ dockApi: { gitLog: vi.fn(), gitCommitDiff: vi.fn() } }))

const commit = { sha: 'merge', shortSha: 'merge', subject: 'Merge feature', authorName: 'Dev', authorDate: '2026-09-12', refs: ['HEAD -> main'], parents: ['a', 'b'] }
afterEach(() => { cleanup(); vi.resetAllMocks() })

describe('Git history interaction', () => {
  it('shows per-file counts and requests a per-file patch when a file is selected', async () => {
    vi.mocked(dockApi.gitLog).mockResolvedValue({ commits: [commit], hasMore: false })
    vi.mocked(dockApi.gitCommitDiff).mockResolvedValue({ baseRef: 'a', headRef: 'merge', mode: 'commit', files: ['app.ts'], patch: '', stat: '', truncated: true, binaryFiles: [], fileStats: [{ path: 'app.ts', additions: 42, deletions: 18 }] })
    render(<GitHistory workdir="repo" lang="zh" active refreshKey={0} />)
    fireEvent.click(await screen.findByRole('button', { name: /Merge feature/ }))
    const file = await screen.findByRole('button', { name: /app.ts/ })
    expect(file.textContent).toContain('+42')
    expect(file.textContent).toContain('−18')
    expect(screen.getByText('合并提交 · 相对第一父提交')).toBeTruthy()
    expect(dockApi.gitCommitDiff).toHaveBeenCalledTimes(1)
    fireEvent.click(file)
    await waitFor(() => expect(dockApi.gitCommitDiff).toHaveBeenLastCalledWith('repo', 'merge', 'app.ts'))
    fireEvent.click(screen.getByRole('button', { name: '关闭提交详情' }))
    expect(screen.queryByRole('button', { name: /app.ts/ })).toBeNull()
  })

  it('ignores a stale all-branch response after changing the scope', async () => {
    let resolveOld!: (result: GitLogResult) => void
    vi.mocked(dockApi.gitLog).mockImplementationOnce(() => new Promise((resolve) => { resolveOld = resolve }))
      .mockResolvedValue({ commits: [{ ...commit, subject: 'Current history' }], hasMore: false })
    render(<GitHistory workdir="repo" lang="zh" active refreshKey={0} />)
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'current' } })
    await screen.findByRole('button', { name: /Current history/ })
    await act(async () => resolveOld({ commits: [commit], hasMore: true }))
    expect(screen.queryByRole('button', { name: /Merge feature/ })).toBeNull()
    expect(dockApi.gitLog).toHaveBeenLastCalledWith('repo', 50, 0, false)
  })
})
