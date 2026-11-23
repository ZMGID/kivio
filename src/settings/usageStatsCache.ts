import { api, type UsageScanReport, type UsageStatsQuery, type UsageStatsResponse } from '../api/tauri'

const AUTO_SYNC_INTERVAL_MS = 30_000
const MAX_CACHED_QUERIES = 8

export function isCurrentUsageStats(stats: UsageStatsResponse, now = new Date()) {
  const timestamp = now.getTime() / 1000
  return stats.sampledAt <= timestamp && timestamp < stats.validUntil
    && stats.utcOffset === -now.getTimezoneOffset() * 60
}

class UsageStatsCache {
  private readonly stats = new Map<string, UsageStatsResponse>()
  private readonly pendingStats = new Map<string, Promise<UsageStatsResponse>>()
  private scan: { report: UsageScanReport; completedAt: number } | null = null
  private pendingScan: { force: boolean; request: Promise<UsageScanReport> } | null = null

  peek(query: UsageStatsQuery) {
    const stats = this.stats.get(JSON.stringify(query))
    return stats && isCurrentUsageStats(stats) ? stats : null
  }

  get lastScan() {
    return this.scan?.report ?? null
  }

  load(query: UsageStatsQuery, fresh = false) {
    const key = JSON.stringify(query)
    const pending = this.pendingStats.get(key)
    if (!fresh && pending) return pending

    const request = api.usageGetStats(query).then(result => {
      if (this.pendingStats.get(key) === request) {
        this.stats.delete(key)
        this.stats.set(key, result)
        if (this.stats.size > MAX_CACHED_QUERIES) {
          const oldest = this.stats.keys().next()
          if (!oldest.done) this.stats.delete(oldest.value)
        }
      }
      return result
    }).finally(() => {
      if (this.pendingStats.get(key) === request) this.pendingStats.delete(key)
    })
    this.pendingStats.set(key, request)
    return request
  }

  sync(force = false): Promise<UsageScanReport> {
    const pending = this.pendingScan
    if (pending) {
      if (!force || pending.force) return pending.request
      return pending.request.then(() => this.sync(true), () => this.sync(true))
    }
    if (!force && this.scan && performance.now() - this.scan.completedAt < AUTO_SYNC_INTERVAL_MS) {
      return Promise.resolve(this.scan.report)
    }
    const request = api.usageSyncCli(force).then(report => {
      if (this.pendingScan?.request === request) this.scan = { report, completedAt: performance.now() }
      return report
    }).finally(() => {
      if (this.pendingScan?.request === request) this.pendingScan = null
    })
    this.pendingScan = { force, request }
    return request
  }

  clear() {
    this.stats.clear()
    this.pendingStats.clear()
    this.scan = null
    this.pendingScan = null
  }
}

export const usageStatsCache = new UsageStatsCache()
