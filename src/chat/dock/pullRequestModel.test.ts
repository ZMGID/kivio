import { describe, expect, it } from 'vitest'
import type { PullRequestItem } from '../../api/dockContracts'
import {
  DEFAULT_FILTERS,
  activeFilterCount,
  diffBlocks,
  groupPullRequests,
  highlightRanges,
  matchesPullRequest,
  parsePullRequestQuery,
  sortPullRequests,
} from './pullRequestModel'

function pr(overrides: Partial<PullRequestItem> = {}): PullRequestItem {
  return {
    number: 1,
    title: 'feat: add pull requests',
    url: 'https://github.com/o/r/pull/1',
    state: 'open',
    createdAt: '2026-11-01T00:00:00Z',
    updatedAt: '2026-11-02T00:00:00Z',
    additions: 10,
    deletions: 2,
    changedFiles: 3,
    headRef: 'feat/pr-panel',
    baseRef: 'main',
    crossRepository: false,
    author: 'octo',
    authorAvatar: '',
    labels: [],
    comments: 0,
    checks: 'none',
    reviewDecision: '',
    conflicting: false,
    reviewRequested: false,
    ...overrides,
  }
}

const match = (item: PullRequestItem, text: string, viewer = '') =>
  matchesPullRequest(item, parsePullRequestQuery(text), DEFAULT_FILTERS, viewer)

describe('parsePullRequestQuery', () => {
  it('splits qualifiers, numbers and free text', () => {
    const query = parsePullRequestQuery('label:"good first" author:@Octo head:feat base:main is:draft #12 Panel')
    expect(query.labels).toEqual(['good first'])
    expect(query.authors).toEqual(['octo'])
    expect(query.heads).toEqual(['feat'])
    expect(query.bases).toEqual(['main'])
    expect([...query.flags]).toEqual(['draft'])
    expect(query.numbers).toEqual([12])
    expect(query.terms).toEqual(['panel'])
  })

  it('keeps unknown qualifiers and empty values as text', () => {
    const query = parsePullRequestQuery('is:whatever label: fix')
    expect(query.flags.size).toBe(0)
    expect(query.terms).toEqual(['is:whatever', 'label:', 'fix'])
  })
})

describe('matchesPullRequest', () => {
  it('matches free text across title, number, author and branch', () => {
    expect(match(pr(), 'pull')).toBe(true)
    expect(match(pr(), 'pr-panel')).toBe(true)
    expect(match(pr(), 'OCTO add')).toBe(true)
    expect(match(pr({ number: 70 }), '#70')).toBe(true)
    expect(match(pr(), 'missing')).toBe(false)
  })

  it('applies qualifiers and flags', () => {
    const labelled = pr({ labels: [{ name: 'bug', color: 'd73a4a' }] })
    expect(match(labelled, 'label:bug')).toBe(true)
    expect(match(pr(), 'label:bug')).toBe(false)
    expect(match(pr(), 'base:develop')).toBe(false)
    expect(match(pr({ state: 'draft' }), 'is:draft')).toBe(true)
    expect(match(pr(), 'is:draft')).toBe(false)
    expect(match(pr({ conflicting: true }), 'is:conflict')).toBe(true)
    expect(match(pr({ reviewRequested: true }), 'is:review')).toBe(true)
    expect(match(pr({ checks: 'failure' }), 'is:failing')).toBe(true)
    expect(match(pr(), 'is:mine', 'Octo')).toBe(true)
    expect(match(pr(), 'is:mine')).toBe(false)
  })

  it('applies menu filters', () => {
    const query = parsePullRequestQuery('')
    expect(matchesPullRequest(pr({ checks: 'success' }), query, { ...DEFAULT_FILTERS, checks: 'success' })).toBe(true)
    expect(matchesPullRequest(pr({ checks: 'pending' }), query, { ...DEFAULT_FILTERS, checks: 'success' })).toBe(false)
    expect(matchesPullRequest(pr({ reviewDecision: 'approved' }), query, { ...DEFAULT_FILTERS, review: 'approved' })).toBe(true)
    expect(matchesPullRequest(pr({ state: 'draft' }), query, { ...DEFAULT_FILTERS, draft: 'exclude' })).toBe(false)
    expect(matchesPullRequest(pr(), query, { ...DEFAULT_FILTERS, draft: 'only' })).toBe(false)
    expect(activeFilterCount({ ...DEFAULT_FILTERS, draft: 'only', checks: 'failure' })).toBe(2)
  })
})

describe('sortPullRequests', () => {
  const items = [
    pr({ number: 1, updatedAt: '2026-11-03T00:00:00Z', createdAt: '2026-10-01T00:00:00Z', additions: 5, deletions: 0 }),
    pr({ number: 2, updatedAt: '2026-11-01T00:00:00Z', createdAt: '2026-11-01T00:00:00Z', additions: 500, deletions: 20 }),
    pr({ number: 3, updatedAt: '2026-11-02T00:00:00Z', createdAt: '2026-10-15T00:00:00Z', additions: 40, deletions: 40 }),
  ]
  const numbers = (sorted: PullRequestItem[]) => sorted.map((item) => item.number)

  it('orders by each sort key without mutating input', () => {
    expect(numbers(sortPullRequests(items, 'updated'))).toEqual([1, 3, 2])
    expect(numbers(sortPullRequests(items, 'created'))).toEqual([2, 3, 1])
    expect(numbers(sortPullRequests(items, 'number'))).toEqual([3, 2, 1])
    expect(numbers(sortPullRequests(items, 'size'))).toEqual([2, 3, 1])
    expect(numbers(items)).toEqual([1, 2, 3])
  })
})

describe('groupPullRequests', () => {
  it('groups review requests, authored and others, skipping empty groups', () => {
    const groups = groupPullRequests([
      pr({ number: 1, author: 'me' }),
      pr({ number: 2, author: 'other', reviewRequested: true }),
      pr({ number: 3, author: 'other', reviewRequested: true, state: 'merged' }),
      pr({ number: 4, author: 'other' }),
    ], 'ME')
    expect(groups.map((group) => [group.key, group.items.map((item) => item.number)])).toEqual([
      ['review', [2]],
      ['authored', [1]],
      ['others', [3, 4]],
    ])
    expect(groupPullRequests([pr()], '').map((group) => group.key)).toEqual(['others'])
  })
})

describe('diffBlocks', () => {
  it('scales filled blocks with size and keeps both kinds visible', () => {
    expect(diffBlocks(0, 0)).toEqual(['neutral', 'neutral', 'neutral', 'neutral', 'neutral'])
    expect(diffBlocks(3, 0)).toEqual(['add', 'neutral', 'neutral', 'neutral', 'neutral'])
    const large = diffBlocks(1442, 9)
    expect(large).toHaveLength(5)
    expect(large.filter((block) => block === 'neutral')).toHaveLength(0)
    expect(large).toContain('del')
    expect(diffBlocks(1, 1000).filter((block) => block === 'add')).toHaveLength(1)
  })
})

describe('highlightRanges', () => {
  it('finds case-insensitive, merged ranges', () => {
    expect(highlightRanges('Add pull requests panel', ['pull', 'll req', 'panel'])).toEqual([[4, 12], [18, 23]])
    expect(highlightRanges('abc', [''])).toEqual([])
  })
})
