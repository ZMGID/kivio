// Git 面板数据 hook：status 刷新（签名守卫防恒等重渲）、选中文件 diff、
// 变更操作单飞行锁、workspace-activity 失效。
import { useCallback, useEffect, useRef, useState } from 'react'
import { dockApi } from './api'
import { gitStatusSignature } from './gitReviewModel'
import type { GitDiffResult, GitDiffStatFile, GitMutationResult, GitRepoState } from '../../api/dockContracts'
import { workspaceActivity } from './workspaceActivity'

const FALLBACK_POLL_MS = 10_000

export type UseGitReviewOptions = {
  workdir: string
  active: boolean
}

export function useGitReview({ workdir, active }: UseGitReviewOptions) {
  const [status, setStatus] = useState<GitRepoState | null>(null)
  const [statusLoading, setStatusLoading] = useState(false)
  const [viewError, setViewError] = useState('')
  const [mutationError, setMutationError] = useState('')
  const [busy, setBusy] = useState(false)
  const [fileDiff, setFileDiff] = useState<{ key: string; result: GitDiffResult } | null>(null)
  const [fileDiffLoading, setFileDiffLoading] = useState(false)
  /** 逐文件行数统计（改动列表行内徽标）：path → {additions, deletions}。 */
  const [fileStats, setFileStats] = useState<Record<string, GitDiffStatFile>>({})

  const workdirRef = useRef(workdir)
  workdirRef.current = workdir
  const activeRef = useRef(active)
  activeRef.current = active
  const statusRef = useRef(status)
  statusRef.current = status
  const signatureRef = useRef('')
  const busyRef = useRef(false)
  const dirtyRef = useRef(false)
  const lastRevisionRef = useRef(-1)
  const fileDiffEpochRef = useRef(0)
  const fileStatsKeyRef = useRef('')
  const statusEpochRef = useRef(0)
  const mountedRef = useRef(false)
  const refreshQueue = useRef<{ pending: boolean; force: boolean; promise: Promise<void> | null }>({ pending: false, force: false, promise: null })

  /** 与 status 串行拉取 numstat；同一修改文件的内容变了也会更新行数。 */
  const loadFileStats = useCallback(async (currentWorkdir: string, epoch: number) => {
    try {
      const stat = await dockApi.gitDiffStat(currentWorkdir)
      if (!mountedRef.current || statusEpochRef.current !== epoch) return
      const key = stat.files.map((f) => `${f.path}:${f.additions}:${f.deletions}`).join(';')
      if (key === fileStatsKeyRef.current) return
      fileStatsKeyRef.current = key
      const map: Record<string, GitDiffStatFile> = {}
      for (const file of stat.files) map[file.path] = file
      setFileStats(map)
    } catch {
      // 行数徽标是增强信息，失败静默，下轮 refresh 自愈。
    }
  }, [])

  const refresh = useCallback((options: { silent?: boolean; force?: boolean } = {}): Promise<void> => {
    const queue = refreshQueue.current
    if (!workdirRef.current || !mountedRef.current) return Promise.resolve()
    queue.pending = true
    queue.force ||= options.force ?? false
    if (!options.silent) setStatusLoading(true)
    if (queue.promise) return queue.promise
    queue.promise = (async () => {
      try {
        // Serialize status + numstat, retaining only one followup for an edit burst.
        while (queue.pending && mountedRef.current) {
          queue.pending = false
          const force = queue.force
          queue.force = false
          const currentWorkdir = workdirRef.current
          const epoch = statusEpochRef.current
          if (!currentWorkdir) break
          try {
            const next = await dockApi.gitStatus(currentWorkdir)
            if (!mountedRef.current || statusEpochRef.current !== epoch) continue
            const signature = gitStatusSignature(next)
            if (force || signature !== signatureRef.current) {
              signatureRef.current = signature
              setStatus(next)
            }
            if (next.status === 'ready') {
              await loadFileStats(currentWorkdir, epoch)
            } else {
              fileStatsKeyRef.current = ''
              setFileStats({})
            }
            if (mountedRef.current && statusEpochRef.current === epoch) setViewError('')
          } catch (err) {
            if (mountedRef.current && statusEpochRef.current === epoch) {
              setViewError(err instanceof Error ? err.message : String(err))
            }
          }
        }
      } finally {
        // Release in the worker itself, before another microtask can enqueue a
        // refresh against a settled promise whose pending flag nobody consumes.
        queue.promise = null
        if (mountedRef.current) setStatusLoading(false)
      }
    })()
    return queue.promise
  }, [loadFileStats])

  // workdir 变化：清空全部 git 状态并立即重拉。
  useEffect(() => {
    const queue = refreshQueue.current
    mountedRef.current = true
    statusEpochRef.current += 1
    fileDiffEpochRef.current += 1
    statusRef.current = null
    signatureRef.current = ''
    lastRevisionRef.current = -1
    dirtyRef.current = false
    setStatus(null)
    setViewError('')
    setMutationError('')
    setFileDiff(null)
    fileStatsKeyRef.current = ''
    setFileStats({})
    setStatusLoading(false)
    setFileDiffLoading(false)
    return () => {
      mountedRef.current = false
      statusEpochRef.current += 1
      fileDiffEpochRef.current += 1
      queue.pending = false
    }
  }, [workdir])

  // active 由 false → true：冲刷脏标记。
  useEffect(() => {
    if (!active) {
      dirtyRef.current ||= refreshQueue.current.pending
      refreshQueue.current.pending = false
      return
    }
    if (!workdir) return
    if (!statusRef.current || dirtyRef.current) {
      dirtyRef.current = false
      void refresh({ silent: Boolean(statusRef.current) })
    }
  }, [active, workdir, refresh])

  // workspace-activity：fs 或 git 任一置位即静默刷新；revision 回退（watcher 重建/重置）强制刷新。
  useEffect(() => {
    if (!workdir) return
    return workspaceActivity.subscribe(workdir, (activity) => {
      const regressed = activity.revision < lastRevisionRef.current
      lastRevisionRef.current = Math.max(lastRevisionRef.current, activity.revision)
      if (!activity.fs && !activity.git && !activity.truncated) return
      if (!activeRef.current) {
        dirtyRef.current = true
        return
      }
      void refresh({ silent: true, force: regressed })
    })
  }, [workdir, refresh])

  // watcher 不可用（浏览器预览）时的 10s 兜底轮询。
  useEffect(() => {
    if (!active || !workdir || workspaceActivity.isAvailable()) return
    const timer = window.setInterval(() => void refresh({ silent: true }), FALLBACK_POLL_MS)
    return () => window.clearInterval(timer)
  }, [active, workdir, refresh])

  /** 选中文件 diff：working_tree 为主；有 upstream 时并行拉 branch diff，working_tree 为空则回退。 */
  const selectFileDiff = useCallback(async (path: string | null) => {
    const epoch = ++fileDiffEpochRef.current
    if (!path) {
      setFileDiff(null)
      setFileDiffLoading(false)
      return
    }
    const currentWorkdir = workdirRef.current
    if (!currentWorkdir) return
    setFileDiffLoading(true)
    const hasUpstream = Boolean(statusRef.current?.upstream)
    const [workingTree, branch] = await Promise.allSettled([
      dockApi.gitDiff(currentWorkdir, 'working_tree', path),
      hasUpstream ? dockApi.gitDiff(currentWorkdir, 'branch', path) : Promise.resolve(null),
    ])
    if (fileDiffEpochRef.current !== epoch || workdirRef.current !== currentWorkdir) return
    const workingResult = workingTree.status === 'fulfilled' ? workingTree.value : null
    const branchResult = branch.status === 'fulfilled' ? branch.value : null
    const picked =
      workingResult && (workingResult.patch || !branchResult)
        ? workingResult
        : (branchResult ?? workingResult)
    if (picked) {
      setFileDiff({ key: `${currentWorkdir}:${path}`, result: picked })
    } else {
      setFileDiff(null)
      setViewError(
        workingTree.status === 'rejected'
          ? String(workingTree.reason instanceof Error ? workingTree.reason.message : workingTree.reason)
          : 'diff failed',
      )
    }
    setFileDiffLoading(false)
  }, [])

  /**
   * 变更操作单飞行：同一时刻只允许一个在飞；ok:false 时抛出（message/stderr 优先），
   * 成功则先应用随响应返回的新 status，再静默刷新兜底。
   */
  const runMutation = useCallback(
    async (action: () => Promise<GitMutationResult>): Promise<void> => {
      if (busyRef.current) return
      const epoch = statusEpochRef.current
      busyRef.current = true
      setBusy(true)
      setMutationError('')
      try {
        const result = await action()
        if (!result.ok) {
          throw new Error(result.message || result.stderr || 'git operation failed')
        }
        if (!mountedRef.current || statusEpochRef.current !== epoch) return
        // Retire any pre-mutation query; its response cannot overwrite this state.
        statusEpochRef.current += 1
        const signature = gitStatusSignature(result.state)
        if (signature !== signatureRef.current) {
          signatureRef.current = signature
          setStatus(result.state)
        }
        void refresh({ silent: true })
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err)
        if (mountedRef.current && statusEpochRef.current === epoch) setMutationError(message)
        throw err
      } finally {
        busyRef.current = false
        setBusy(false)
      }
    },
    [refresh],
  )

  const clearMutationError = useCallback(() => setMutationError(''), [])

  return {
    status,
    statusLoading,
    viewError,
    mutationError,
    clearMutationError,
    busy,
    fileDiff,
    fileDiffLoading,
    selectFileDiff,
    fileStats,
    refresh,
    runMutation,
  }
}
