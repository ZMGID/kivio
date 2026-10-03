import { describe, expect, it } from 'vitest'
import { gitStatusSignature, partitionStatusEntries, statusLetter } from './gitReviewModel'
import type { GitRepoState, GitStatusEntry } from '../../api/dockContracts'

function statusEntry(partial: Partial<GitStatusEntry> & { path: string }): GitStatusEntry {
  return {
    oldPath: null,
    indexStatus: ' ',
    worktreeStatus: ' ',
    kind: 'file',
    staged: false,
    conflicted: false,
    untracked: false,
    ...partial,
  }
}

describe('partitionStatusEntries', () => {
  it('splits into conflicted / staged / unstaged', () => {
    const entries = [
      statusEntry({ path: 'staged.ts', staged: true, indexStatus: 'M' }),
      statusEntry({ path: 'dirty.ts', worktreeStatus: 'M' }),
      statusEntry({ path: 'new.ts', untracked: true }),
      statusEntry({ path: 'conflict.ts', conflicted: true, staged: true }),
    ]
    const { conflicted, staged, unstaged } = partitionStatusEntries(entries)
    expect(conflicted.map((e) => e.path)).toEqual(['conflict.ts'])
    expect(staged.map((e) => e.path)).toEqual(['staged.ts'])
    expect(unstaged.map((e) => e.path)).toEqual(['dirty.ts', 'new.ts'])
  })
})

describe('statusLetter', () => {
  it('maps entry states to badge letters', () => {
    expect(statusLetter(statusEntry({ path: 'a', conflicted: true }))).toBe('U')
    expect(statusLetter(statusEntry({ path: 'a', untracked: true }))).toBe('A')
    expect(statusLetter(statusEntry({ path: 'a', indexStatus: 'R' }))).toBe('R')
    expect(statusLetter(statusEntry({ path: 'a', indexStatus: 'A' }))).toBe('A')
    expect(statusLetter(statusEntry({ path: 'a', worktreeStatus: 'D' }))).toBe('D')
    expect(statusLetter(statusEntry({ path: 'a', worktreeStatus: 'M' }))).toBe('M')
  })
})

describe('gitStatusSignature', () => {
  const base: GitRepoState = {
    repoRoot: '/repo',
    head: 'main',
    upstream: 'origin/main',
    ahead: 1,
    behind: 0,
    stashCount: 0,
    entries: [statusEntry({ path: 'a.ts', worktreeStatus: 'M' })],
    status: 'ready',
    error: null,
  }

  it('is stable for identical states', () => {
    expect(gitStatusSignature(base)).toBe(gitStatusSignature({ ...base, entries: [...base.entries] }))
  })

  it('changes when entries or head fields change', () => {
    expect(gitStatusSignature({ ...base, ahead: 2 })).not.toBe(gitStatusSignature(base))
    expect(
      gitStatusSignature({ ...base, entries: [...base.entries, statusEntry({ path: 'b.ts', untracked: true })] }),
    ).not.toBe(gitStatusSignature(base))
  })

  it('handles null state', () => {
    expect(gitStatusSignature(null)).toBe('null')
  })
})
