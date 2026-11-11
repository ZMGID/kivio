// 拉取请求面板的纯规则：搜索语法、筛选、排序、分组与行数色块。
// 列表数据来自后端一页页加载，这里只对已加载的 PR 做即时的本地处理。
import type { PullRequestChecks, PullRequestItem, PullRequestReviewDecision } from '../../api/dockContracts'

export type PullRequestSort = 'updated' | 'created' | 'number' | 'size'
export type ChecksFilter = 'any' | Exclude<PullRequestChecks, 'none'>
export type ReviewFilter = 'any' | Exclude<PullRequestReviewDecision, ''>
export type DraftFilter = 'any' | 'only' | 'exclude'

export type PullRequestFilters = {
  checks: ChecksFilter
  review: ReviewFilter
  draft: DraftFilter
}

export const DEFAULT_FILTERS: PullRequestFilters = { checks: 'any', review: 'any', draft: 'any' }

export function activeFilterCount(filters: PullRequestFilters): number {
  return (Object.keys(filters) as (keyof PullRequestFilters)[]).filter((key) => filters[key] !== DEFAULT_FILTERS[key]).length
}

export type PullRequestQuery = {
  /** 自由文本，匹配标题、编号、作者与分支。 */
  terms: string[]
  labels: string[]
  authors: string[]
  heads: string[]
  bases: string[]
  numbers: number[]
  flags: Set<'draft' | 'conflict' | 'review' | 'failing' | 'mine'>
}

const FLAG_ALIASES: Record<string, 'draft' | 'conflict' | 'review' | 'failing' | 'mine'> = {
  draft: 'draft',
  conflict: 'conflict',
  conflicts: 'conflict',
  review: 'review',
  failing: 'failing',
  failed: 'failing',
  mine: 'mine',
}

/** 解析搜索框：`label:bug author:octo head:feat base:main is:draft #12 关键词`。引号包裹的值可含空格。 */
export function parsePullRequestQuery(input: string): PullRequestQuery {
  const query: PullRequestQuery = { terms: [], labels: [], authors: [], heads: [], bases: [], numbers: [], flags: new Set() }
  const tokens = input.match(/(?:[^\s"]+:"[^"]*"|"[^"]*"|\S+)/g) ?? []
  for (const token of tokens) {
    const unquote = (value: string) => value.replace(/^"|"$/g, '').trim().toLowerCase()
    const colon = token.indexOf(':')
    const key = colon > 0 ? token.slice(0, colon).toLowerCase() : ''
    const value = colon > 0 ? unquote(token.slice(colon + 1)) : ''
    if (key && value) {
      if (key === 'label') { query.labels.push(value); continue }
      if (key === 'author') { query.authors.push(value.replace(/^@/, '')); continue }
      if (key === 'head' || key === 'branch') { query.heads.push(value); continue }
      if (key === 'base') { query.bases.push(value); continue }
      if (key === 'is' && FLAG_ALIASES[value]) { query.flags.add(FLAG_ALIASES[value]); continue }
    }
    const number = /^#(\d+)$/.exec(token)
    if (number) { query.numbers.push(Number(number[1])); continue }
    const term = unquote(token)
    if (term) query.terms.push(term)
  }
  return query
}

export function matchesPullRequest(item: PullRequestItem, query: PullRequestQuery, filters: PullRequestFilters, viewer = ''): boolean {
  if (filters.checks !== 'any' && item.checks !== filters.checks) return false
  if (filters.review !== 'any' && item.reviewDecision !== filters.review) return false
  if (filters.draft === 'only' && item.state !== 'draft') return false
  if (filters.draft === 'exclude' && item.state === 'draft') return false

  const author = item.author.toLowerCase()
  if (query.numbers.length && !query.numbers.includes(item.number)) return false
  if (query.authors.length && !query.authors.includes(author)) return false
  const labels = item.labels.map((label) => label.name.toLowerCase())
  if (!query.labels.every((label) => labels.some((name) => name.includes(label)))) return false
  if (!query.heads.every((head) => item.headRef.toLowerCase().includes(head))) return false
  if (!query.bases.every((base) => item.baseRef.toLowerCase() === base)) return false
  if (query.flags.has('draft') && item.state !== 'draft') return false
  if (query.flags.has('conflict') && !item.conflicting) return false
  if (query.flags.has('review') && !item.reviewRequested) return false
  if (query.flags.has('failing') && item.checks !== 'failure') return false
  if (query.flags.has('mine') && (!viewer || author !== viewer.toLowerCase())) return false

  const haystack = `${item.title} #${item.number} ${item.number} ${item.author} ${item.headRef}`.toLowerCase()
  return query.terms.every((term) => haystack.includes(term))
}

export function sortPullRequests(items: PullRequestItem[], sort: PullRequestSort): PullRequestItem[] {
  const time = (value: string) => Date.parse(value) || 0
  const next = [...items]
  switch (sort) {
    case 'created':
      return next.sort((a, b) => time(b.createdAt) - time(a.createdAt) || b.number - a.number)
    case 'number':
      return next.sort((a, b) => b.number - a.number)
    case 'size':
      return next.sort((a, b) => b.additions + b.deletions - (a.additions + a.deletions) || b.number - a.number)
    default:
      return next.sort((a, b) => time(b.updatedAt) - time(a.updatedAt) || b.number - a.number)
  }
}

export type PullRequestGroupKey = 'review' | 'authored' | 'others'
export type PullRequestGroup = { key: PullRequestGroupKey; items: PullRequestItem[] }

/** 待我审查 → 我创建的 → 其他；保留输入顺序，空组不返回。 */
export function groupPullRequests(items: PullRequestItem[], viewer: string): PullRequestGroup[] {
  const me = viewer.toLowerCase()
  const groups: Record<PullRequestGroupKey, PullRequestItem[]> = { review: [], authored: [], others: [] }
  for (const item of items) {
    if (me && item.author.toLowerCase() === me) groups.authored.push(item)
    else if (item.reviewRequested && item.state !== 'merged' && item.state !== 'closed') groups.review.push(item)
    else groups.others.push(item)
  }
  return (['review', 'authored', 'others'] as const)
    .map((key) => ({ key, items: groups[key] }))
    .filter((group) => group.items.length > 0)
}

export type DiffBlock = 'add' | 'del' | 'neutral'

/** GitHub 式五格行数色块：按增删比例分配，至少各占一格（若有）。 */
export function diffBlocks(additions: number, deletions: number, total = 5): DiffBlock[] {
  const changed = additions + deletions
  if (changed <= 0) return Array<DiffBlock>(total).fill('neutral')
  // 改动少时不铺满：1–9 行占 1 格，以此类推，最多 total 格。
  const filled = Math.min(total, Math.max(1, Math.ceil(Math.log10(changed + 1) * 1.6)))
  let add = Math.round((additions / changed) * filled)
  if (additions > 0 && add === 0) add = 1
  if (deletions > 0 && add === filled) add = filled - 1
  const del = filled - add
  return [
    ...Array<DiffBlock>(add).fill('add'),
    ...Array<DiffBlock>(del).fill('del'),
    ...Array<DiffBlock>(total - filled).fill('neutral'),
  ]
}

/** 标题中命中自由文本的区间，用于高亮；区间按起点排序且互不重叠。 */
export function highlightRanges(text: string, terms: string[]): [number, number][] {
  const lower = text.toLowerCase()
  const ranges: [number, number][] = []
  for (const term of terms) {
    if (!term) continue
    let from = 0
    for (;;) {
      const at = lower.indexOf(term, from)
      if (at < 0) break
      ranges.push([at, at + term.length])
      from = at + term.length
    }
  }
  ranges.sort((a, b) => a[0] - b[0])
  const merged: [number, number][] = []
  for (const range of ranges) {
    const last = merged[merged.length - 1]
    if (last && range[0] <= last[1]) last[1] = Math.max(last[1], range[1])
    else merged.push([...range])
  }
  return merged
}

export function formatCount(value: number): string {
  return value.toLocaleString('en-US')
}
