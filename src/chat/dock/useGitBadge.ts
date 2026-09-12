// 按工作目录共享 Git 快照、watcher 和兜底轮询；无消费者时释放缓存。
import { useCallback, useSyncExternalStore } from 'react'
import { dockApi } from './api'
import { gitStatusSignature } from './gitReviewModel'
import type { GitDiffStat, GitRepoState } from './types'
import { workspaceActivity } from './workspaceActivity'

export type GitBadge = {
  state: GitRepoState | null
  diffStat: GitDiffStat | null
  loading: boolean
  /** 变更操作返回的权威 state 立即共享，旧查询不能覆盖它。 */
  applyMutationState: (next: GitRepoState) => void
  refresh: (options?: { silent?: boolean }) => Promise<void>
}

type Snapshot = Pick<GitBadge, 'state' | 'diffStat' | 'loading'>
const EMPTY: Snapshot = { state: null, diffStat: null, loading: false }
const stores = new Map<string, GitBadgeStore>()

function sameStat(left: GitDiffStat | null, right: GitDiffStat | null): boolean {
  return left === right || Boolean(left && right &&
    left.filesChanged === right.filesChanged &&
    left.additions === right.additions && left.deletions === right.deletions)
}

class GitBadgeStore {
  private snapshot = EMPTY
  private signature = gitStatusSignature(null)
  private listeners = new Map<() => void, boolean>()
  private unsubscribe: (() => void) | null = null
  private timer: ReturnType<typeof setInterval> | null = null
  private inFlight: Promise<void> | null = null
  private pending = false
  private epoch = 0

  constructor(private workdir: string) {}

  getSnapshot = (): Snapshot => this.snapshot

  private publish(next: Snapshot) {
    const previous = this.snapshot
    const signature = next.state === previous.state ? this.signature : gitStatusSignature(next.state)
    const state = signature === this.signature ? previous.state : next.state
    const diffStat = sameStat(previous.diffStat, next.diffStat) ? previous.diffStat : next.diffStat
    if (state === previous.state && diffStat === previous.diffStat && next.loading === previous.loading) return
    this.signature = signature
    this.snapshot = { state, diffStat, loading: next.loading }
    for (const listener of this.listeners.keys()) listener()
  }

  private wantsDiff() {
    return [...this.listeners.values()].some(Boolean)
  }

  subscribe(listener: () => void, includeDiffStat: boolean): () => void {
    const hadDiff = this.wantsDiff()
    this.listeners.set(listener, includeDiffStat)
    if (this.workdir && !this.unsubscribe) {
      this.unsubscribe = workspaceActivity.subscribe(this.workdir, (event) => {
        if (event.fs || event.git || event.truncated) void this.refresh({ silent: true })
      })
      if (!workspaceActivity.isAvailable()) {
        this.timer = setInterval(() => void this.refresh({ silent: true }), 10_000)
      }
      void this.refresh()
    } else if (!hadDiff && includeDiffStat && this.snapshot.state && !this.snapshot.diffStat) {
      // 晚挂载的 diff 徽标需要补齐统计；同轮挂载则由下方微任务合并需求。
      void this.refresh({ silent: true })
    }
    return () => {
      this.listeners.delete(listener)
      // StrictMode 同轮退订/重订复用请求；真正卸载后不保留仓库列表或轮询。
      queueMicrotask(() => {
        if (this.listeners.size) return
        this.unsubscribe?.()
        this.unsubscribe = null
        if (this.timer !== null) clearInterval(this.timer)
        this.timer = null
        this.pending = false
        this.epoch += 1
        this.snapshot = EMPTY
        this.signature = gitStatusSignature(null)
        if (stores.get(this.workdir) === this) stores.delete(this.workdir)
      })
    }
  }

  refresh = (options?: { silent?: boolean }): Promise<void> => {
    if (!this.workdir || !this.listeners.size) return Promise.resolve()
    if (!options?.silent) this.publish({ ...this.snapshot, loading: true })
    this.pending = true
    if (this.inFlight) return this.inFlight
    // 延迟到微任务：同轮挂载两个消费者/批量文件事件只查询一次。
    this.inFlight = Promise.resolve().then(async () => {
      try {
        while (this.pending && this.listeners.size) {
          this.pending = false
          const epoch = this.epoch
          const includeDiffStat = this.wantsDiff()
          try {
            const next = await dockApi.gitSnapshot(this.workdir, includeDiffStat)
            if (epoch !== this.epoch || !this.listeners.size) continue
            this.publish({
              state: next.state,
              diffStat: this.wantsDiff() ? next.diffStat : null,
              loading: this.snapshot.loading,
            })
            // 查询途中新增了 diff 消费者，补一次完整快照。
            if (!includeDiffStat && this.wantsDiff()) this.pending = true
          } catch {
            // 状态类信息不打断用户；后续事件或手动刷新会重试。
          }
          // 查询途中再有事件则跑一轮尾随刷新，不并发，也不丢掉最后一次改动。
        }
      } finally {
        this.inFlight = null
        this.publish({ ...this.snapshot, loading: false })
      }
    })
    return this.inFlight
  }

  applyMutationState = (state: GitRepoState) => {
    if (!this.listeners.size) return
    this.epoch += 1
    this.publish({ state, diffStat: null, loading: this.snapshot.loading })
    void this.refresh({ silent: true })
  }
}

export function useGitBadge(workdir: string, includeDiffStat = false): GitBadge {
  // 仅在 React 提交订阅时创建缓存；放弃的并发 render 不留下仓库条目。
  const subscribe = useCallback(
    (listener: () => void) => {
      let store = stores.get(workdir)
      if (!store) {
        store = new GitBadgeStore(workdir)
        stores.set(workdir, store)
      }
      return store.subscribe(listener, includeDiffStat)
    },
    [workdir, includeDiffStat],
  )
  const getSnapshot = useCallback(() => stores.get(workdir)?.getSnapshot() ?? EMPTY, [workdir])
  const refresh = useCallback(
    (options?: { silent?: boolean }) => stores.get(workdir)?.refresh(options) ?? Promise.resolve(),
    [workdir],
  )
  const applyMutationState = useCallback((next: GitRepoState) => {
    stores.get(workdir)?.applyMutationState(next)
  }, [workdir])
  const snapshot = useSyncExternalStore(subscribe, getSnapshot, getSnapshot)
  return { ...snapshot, refresh, applyMutationState }
}
