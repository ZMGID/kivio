import { describe, expect, it } from 'vitest'
import type { GitRepoState } from '../../api/dockContracts'
import { resolveGitMenu, resolveGitQuickAction } from './workspaceGitAction'

const clean: GitRepoState = {
  status: 'ready', repoRoot: '/repo', head: 'main', upstream: 'origin/main',
  ahead: 0, behind: 0, stashCount: 0, entries: [], error: null,
}
const entry = {
  path: 'a.txt', oldPath: null, indexStatus: ' ', worktreeStatus: 'M', kind: 'modified',
  staged: false, conflicted: false, untracked: false,
}
const dirty: GitRepoState = { ...clean, entries: [entry] }

describe('resolveGitQuickAction', () => {
  it('commits and pushes dirty work when a remote exists', () => {
    expect(resolveGitQuickAction(dirty, true, false)).toEqual({ kind: 'run', action: 'commit_push' })
  })

  it('only commits without a remote or while behind upstream', () => {
    expect(resolveGitQuickAction(dirty, false, false)).toEqual({ kind: 'run', action: 'commit' })
    expect(resolveGitQuickAction({ ...dirty, behind: 2 }, true, false)).toEqual({ kind: 'run', action: 'commit' })
  })

  it('pushes unpublished branches only when a remote exists', () => {
    const unpublished = { ...clean, upstream: null }
    expect(resolveGitQuickAction(unpublished, true, false)).toEqual({ kind: 'run', action: 'push' })
    expect(resolveGitQuickAction(unpublished, false, false)).toEqual({ kind: 'hint', action: 'push', hint: 'noRemote' })
  })

  it('explains why nothing can run', () => {
    expect(resolveGitQuickAction(clean, true, false)).toMatchObject({ kind: 'hint', hint: 'upToDate' })
    expect(resolveGitQuickAction({ ...clean, behind: 1 }, true, false)).toMatchObject({ kind: 'hint', hint: 'behind' })
    expect(resolveGitQuickAction({ ...dirty, head: '(detached)' }, true, false)).toMatchObject({ kind: 'hint', hint: 'detached' })
    expect(resolveGitQuickAction(dirty, true, true)).toMatchObject({ kind: 'hint', hint: 'busy' })
    expect(resolveGitQuickAction(null, true, false)).toMatchObject({ kind: 'hint', hint: 'unavailable' })
  })

  it('pushes local commits ahead of upstream', () => {
    expect(resolveGitQuickAction({ ...clean, ahead: 3 }, true, false)).toEqual({ kind: 'run', action: 'push' })
  })
})

describe('resolveGitMenu', () => {
  it('disables every remote action without a remote', () => {
    expect(resolveGitMenu({ ...dirty, upstream: null }, false, false)).toEqual({ commit: true, commit_push: false, push: false })
  })

  it('blocks pushing while behind upstream', () => {
    expect(resolveGitMenu({ ...dirty, ahead: 1, behind: 1 }, true, false)).toEqual({ commit: true, commit_push: false, push: false })
  })

  it('disables everything while busy or detached', () => {
    const none = { commit: false, commit_push: false, push: false }
    expect(resolveGitMenu(dirty, true, true)).toEqual(none)
    expect(resolveGitMenu({ ...dirty, head: 'HEAD' }, true, false)).toEqual(none)
  })
})
