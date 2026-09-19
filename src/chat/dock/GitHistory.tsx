import { useEffect, useMemo, useRef, useState } from 'react'
import { VList } from 'virtua'
import { GitBranch, Loader2, X } from 'lucide-react'
import { i18n, type Lang } from '../../components/i18n'
import { dockApi } from './api'
import { DiffView } from './DiffView'
import { GRAPH_COLORS, layoutGitGraph, type GraphRow } from './gitGraph'
import { relativeTime } from './gitReviewModel'
import { workspaceActivity } from './workspaceActivity'
import type { GitCommitItem, GitDiffResult } from '../../api/dockContracts'

function GraphCell({ row, columns }: { row: GraphRow; columns: number }) {
  const x = (lane: number) => 10 + lane * 12
  const width = columns * 12 + 8
  return <svg aria-hidden="true" width={Math.min(width, 132)} height={28} viewBox={`0 0 ${width} 28`} preserveAspectRatio="none" className="shrink-0 overflow-visible">
    {row.edges.map((edge, index) => {
      const y1 = edge.half === 'top' ? 0 : 14
      const y2 = edge.half === 'top' ? 14 : 28
      return <path key={index} d={`M ${x(edge.from)} ${y1} C ${x(edge.from)} ${(y1 + y2) / 2}, ${x(edge.to)} ${(y1 + y2) / 2}, ${x(edge.to)} ${y2}`} fill="none" stroke={GRAPH_COLORS[edge.color % GRAPH_COLORS.length]} strokeWidth={1.5} vectorEffect="non-scaling-stroke" />
    })}
    <circle cx={x(row.lane)} cy={14} r={3.5} fill={GRAPH_COLORS[row.color % GRAPH_COLORS.length]} className="stroke-white dark:stroke-neutral-900" strokeWidth={1.2} />
  </svg>
}

export function GitHistory({ workdir, lang, active, refreshKey }: { workdir: string; lang: Lang; active: boolean; refreshKey: number }) {
  const t = i18n[lang]
  const zh = lang === 'zh'
  const [allBranches, setAllBranches] = useState(true)
  const [commits, setCommits] = useState<GitCommitItem[]>([])
  const [limit, setLimit] = useState(50)
  const [hasMore, setHasMore] = useState(false)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')
  const [revision, setRevision] = useState(0)
  const [selected, setSelected] = useState<GitCommitItem | null>(null)
  const [detail, setDetail] = useState<GitDiffResult | null>(null)
  const [detailError, setDetailError] = useState('')
  const [file, setFile] = useState<string | null>(null)
  const [fileDiff, setFileDiff] = useState<GitDiffResult | null>(null)
  const [fileError, setFileError] = useState('')
  const request = useRef(0)

  useEffect(() => {
    setCommits([]); setSelected(null); setLimit(50); setHasMore(false)
  }, [workdir, allBranches])

  useEffect(() => {
    if (!active) return
    let timer: ReturnType<typeof setTimeout> | undefined
    const unsubscribe = workspaceActivity.subscribe(workdir, (event) => {
      if (!event.git) return
      clearTimeout(timer)
      timer = setTimeout(() => setRevision((value) => value + 1), 250)
    })
    const poll = !workspaceActivity.isAvailable() ? setInterval(() => setRevision((value) => value + 1), 10_000) : undefined
    return () => { unsubscribe(); clearTimeout(timer); clearInterval(poll) }
  }, [workdir, active])

  useEffect(() => {
    if (!active) return
    const epoch = ++request.current
    let cancelled = false
    setLoading(true); setError('')
    // Reload the visible prefix so a new commit cannot shift skip offsets between pages.
    void (async () => {
      const result: GitCommitItem[] = []
      let more = false
      while (result.length < limit) {
        const page = await dockApi.gitLog(workdir, Math.min(1000, limit - result.length), result.length, allBranches)
        if (cancelled || request.current !== epoch) return
        result.push(...page.commits)
        more = page.hasMore
        if (!more || !page.commits.length) break
      }
      setCommits(result); setHasMore(more)
    })().catch((err: unknown) => {
      if (!cancelled) setError(String(err))
    }).finally(() => { if (!cancelled) setLoading(false) })
    return () => { cancelled = true }
  }, [workdir, allBranches, limit, active, refreshKey, revision])

  useEffect(() => {
    setDetail(null); setDetailError(''); setFile(null)
    if (!selected) return
    let cancelled = false
    void dockApi.gitCommitDiff(workdir, selected.sha).then((value) => {
      if (!cancelled) setDetail(value)
    }).catch((err: unknown) => { if (!cancelled) setDetailError(String(err)) })
    return () => { cancelled = true }
  }, [workdir, selected, refreshKey])

  useEffect(() => {
    setFileDiff(null); setFileError('')
    if (!file || !selected) return
    let cancelled = false
    void dockApi.gitCommitDiff(workdir, selected.sha, file).then((value) => {
      if (!cancelled) setFileDiff(value)
    }).catch((err: unknown) => { if (!cancelled) setFileError(String(err)) })
    return () => { cancelled = true }
  }, [workdir, selected, file, refreshKey])

  const rows = useMemo(() => layoutGitGraph(commits), [commits])
  const columns = Math.max(2, ...rows.map((row) => row.width))
  return <div className="flex min-h-0 flex-1 flex-col bg-[var(--theme-surface-soft)] text-neutral-700 dark:bg-neutral-900 dark:text-neutral-200">
    <div className="flex items-center justify-between gap-2 border-b border-neutral-200/60 px-2 py-1 dark:border-neutral-700/50">
      <span className="flex items-center gap-1.5 text-[11px] text-neutral-500"><GitBranch size={12} /> Graph {loading && <Loader2 size={11} className="animate-spin" />}</span>
      <select aria-label={zh ? '历史范围' : 'History scope'} value={allBranches ? 'all' : 'current'} onChange={(event) => setAllBranches(event.target.value === 'all')} className="max-w-40 rounded border-0 bg-transparent text-[11px] text-neutral-500 dark:bg-neutral-900">
        <option value="all">{zh ? '所有分支' : 'All branches'}</option>
        <option value="current">{zh ? '当前分支' : 'Current branch'}</option>
      </select>
    </div>
    {error && <button className="px-3 py-2 text-left text-[11px] text-red-500" onClick={() => setRevision((value) => value + 1)}>{error} · {t.dockRetry}</button>}
    <div className="min-h-0 flex-1">
      {!commits.length && !loading && !error ? <div className="p-6 text-center text-[12px] text-neutral-400">{t.dockGitHistoryEmpty}</div> :
        <VList className="custom-scrollbar h-full">
          {commits.map((commit, index) => <button key={commit.sha} type="button" aria-pressed={selected?.sha === commit.sha}
            title={`${commit.subject}\n${commit.authorName} · ${relativeTime(commit.authorDate, lang)}\n${commit.sha}\n${commit.refs.join(', ')}`}
            onClick={() => setSelected(selected?.sha === commit.sha ? null : commit)}
            className={`flex h-7 w-full items-center gap-1 pr-2 text-left text-[12px] ${selected?.sha === commit.sha ? 'bg-violet-500/10' : 'hover:bg-neutral-500/5'}`}>
            <GraphCell row={rows[index]} columns={columns} />
            {commit.refs.length > 0 && <span title={commit.refs.join(', ')} className="max-w-[35%] shrink-0 truncate rounded bg-violet-500/10 px-1 text-[10px] text-violet-700 dark:text-violet-300">{commit.refs.map((ref) => ref.replace('HEAD -> ', '● ')).join(', ')}</span>}
            <span className="min-w-0 flex-1 truncate text-neutral-700 dark:text-neutral-200">{commit.subject}</span>
          </button>)}
          {hasMore && <button disabled={loading} onClick={() => setLimit((value) => value + 50)} className="w-full py-2 text-[11px] text-neutral-500 disabled:opacity-50">{t.dockGitLoadMore}</button>}
        </VList>}
    </div>
    {selected && <div className="custom-scrollbar max-h-[45%] shrink-0 overflow-auto border-t border-neutral-200 dark:border-neutral-700">
      <div className="sticky top-0 z-10 flex items-start gap-2 bg-[var(--theme-surface-soft)] px-2 py-2 dark:bg-neutral-900">
        <div className="min-w-0 flex-1"><div className="truncate text-[12px] font-medium" title={selected.subject}>{selected.subject}</div><div className="mt-0.5 truncate text-[10px] text-neutral-500">{selected.shortSha} · {selected.authorName} · {relativeTime(selected.authorDate, lang)}</div>
          {(selected.parents?.length ?? 0) > 1 && <div className="mt-1 text-[10px] text-neutral-500">{zh ? '合并提交 · 相对第一父提交' : 'Merge commit · compared to first parent'}</div>}
        </div>
        <button aria-label={zh ? '关闭提交详情' : 'Close commit details'} onClick={() => setSelected(null)} className="rounded p-0.5 text-neutral-500 hover:bg-neutral-500/10"><X size={13} /></button>
      </div>
      {detailError ? <div className="p-2 text-[11px] text-red-500">{detailError}</div> : !detail ? <Loader2 size={14} className="m-3 animate-spin text-neutral-400" /> : <>
        {!detail.fileStats?.length && <div className="p-3 text-[11px] text-neutral-400">{t.dockDiffEmpty}</div>}
        {detail.fileStats?.map((stat) => <button key={stat.path} title={stat.path} onClick={() => setFile(file === stat.path ? null : stat.path)} className={`flex w-full items-center gap-2 px-2 py-1 text-left text-[11px] hover:bg-neutral-500/5 ${file === stat.path ? 'bg-neutral-500/10' : ''}`}>
          <span className="min-w-0 flex-1 truncate">{stat.path}</span>
          {detail.binaryFiles.includes(stat.path) ? <span className="text-neutral-400">{zh ? '二进制' : 'Binary'}</span> : <span className="shrink-0 tabular-nums"><span className="text-emerald-600 dark:text-emerald-400">+{stat.additions}</span><span className="ml-2 text-red-500">−{stat.deletions}</span></span>}
        </button>)}
        {file && <div className="px-2 pb-2">{fileError ? <div className="text-[11px] text-red-500">{fileError}</div> : fileDiff ? <DiffView patch={fileDiff.patch} truncated={fileDiff.truncated} lang={lang} emptyText={t.dockDiffEmpty} /> : <Loader2 size={13} className="m-2 animate-spin" />}</div>}
      </>}
    </div>}
  </div>
}
