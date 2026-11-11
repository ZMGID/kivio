// 拉取请求列表缓存：键为「工作目录 + 状态」。负责内存缓存、本地持久化（重启后先显示上次列表）、
// 同键并发请求合并，以及在没有该状态缓存时由其他状态的缓存拼出占位列表。
import { dockApi } from './api'
import type { PullRequestItem, PullRequestList, PullRequestStateFilter } from '../../api/dockContracts'

export type CachedList = { data: PullRequestList; at: number }

const STORAGE_KEY = 'kivio-dock-pr-cache-v1'
/** 持久化的键数上限（按最近写入保留）。每键约 30 条 PR，体积在几十 KB 内。 */
const MAX_KEYS = 12
export const PR_STATES: PullRequestStateFilter[] = ['open', 'merged', 'closed', 'all']

let memory: Map<string, CachedList> | null = null
const inflight = new Map<string, Promise<PullRequestList>>()

export function cacheKey(workdir: string, state: PullRequestStateFilter) {
  return `${workdir}\n${state}`
}

function store(): Map<string, CachedList> {
  if (memory) return memory
  memory = new Map()
  try {
    const raw = JSON.parse(window.localStorage?.getItem(STORAGE_KEY) ?? '[]') as unknown
    if (Array.isArray(raw)) {
      for (const entry of raw) {
        if (Array.isArray(entry) && typeof entry[0] === 'string' && entry[1]?.data?.items) memory.set(entry[0], entry[1] as CachedList)
      }
    }
  } catch {
    // 损坏或不可用的存储只影响首屏速度，不影响读取。
  }
  return memory
}

function persist() {
  try {
    const entries = [...store().entries()]
      .filter(([, value]) => value.data.status === 'ready')
      .sort((a, b) => b[1].at - a[1].at)
      .slice(0, MAX_KEYS)
    window.localStorage?.setItem(STORAGE_KEY, JSON.stringify(entries))
  } catch {
    // 配额不足或存储不可用：保留内存缓存即可。
  }
}

export function readCache(key: string): CachedList | undefined {
  return store().get(key)
}

export function writeCache(key: string, data: PullRequestList, at = Date.now()) {
  store().set(key, { data, at })
  persist()
}

/** 请求第一页；同一键已有请求在途时复用它，结果写入缓存。 */
export function fetchFirstPage(workdir: string, state: PullRequestStateFilter): Promise<PullRequestList> {
  const key = cacheKey(workdir, state)
  const pending = inflight.get(key)
  if (pending) return pending
  const request = dockApi.pullRequests(workdir, state)
    .then((data) => {
      writeCache(key, data)
      return data
    })
    .finally(() => inflight.delete(key))
  inflight.set(key, request)
  return request
}

export function isFetching(workdir: string, state: PullRequestStateFilter) {
  return inflight.has(cacheKey(workdir, state))
}

function matchesState(item: PullRequestItem, state: PullRequestStateFilter) {
  if (state === 'all') return true
  if (state === 'open') return item.state === 'open' || item.state === 'draft'
  return item.state === state
}

/**
 * 该状态还没有缓存时，用同一目录其他状态的缓存拼出占位列表（只用于等待期间展示）。
 * 没有任何可用缓存时返回 null。
 */
export function placeholderList(workdir: string, state: PullRequestStateFilter): PullRequestList | null {
  const sources = PR_STATES
    .map((other) => readCache(cacheKey(workdir, other))?.data)
    .filter((data): data is PullRequestList => Boolean(data && data.status === 'ready'))
  if (!sources.length) return null
  const seen = new Set<number>()
  const items: PullRequestItem[] = []
  for (const source of sources) {
    for (const item of source.items) {
      if (seen.has(item.number) || !matchesState(item, state)) continue
      seen.add(item.number)
      items.push(item)
    }
  }
  items.sort((a, b) => (Date.parse(b.updatedAt) || 0) - (Date.parse(a.updatedAt) || 0))
  return { ...sources[0], items, totalCount: items.length, endCursor: null, hasMore: false }
}

/** 测试用：清空内存缓存；不保留存储时一并清除持久化内容（保留存储可模拟重启）。 */
export function resetPullRequestCache({ keepStorage = false } = {}) {
  memory = null
  inflight.clear()
  if (keepStorage) return
  try {
    window.localStorage?.removeItem(STORAGE_KEY)
  } catch {
    // ignore
  }
}
