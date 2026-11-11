// Dock「拉取请求」面板：当前仓库的 PR 列表（搜索 / 状态 / 排序 / 筛选），当前分支卡片，
// 行内详情。数据经 GitHub CLI 取得（src-tauri/src/dock/pull_requests.rs）。
// 列表缓存见 pullRequestCache.ts：重启后先显示上次的列表，超过 STALE_MS 在后台刷新；
// 首页到达后预取其他状态，切换状态时顶部控件始终保留。
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { ArrowDownUp, ExternalLink, FolderGit2, GitBranch, GitPullRequest, KeyRound, ListFilter, Loader2, RefreshCw, Search, SquareTerminal, Unplug, X } from 'lucide-react'
import { i18n, type Lang } from '../../components/i18n'
import { Button, IconButton } from '../../components/Button'
import { ErrorNotice } from '../../components/AppNotice'
import { notifyError } from '../../components/appNoticeStore'
import { Input, Select } from '../../settings/public/controls'
import { api } from '../../api/tauri'
import { dockApi } from './api'
import { usePresence } from './workspacePanelMotion'
import {
  DEFAULT_FILTERS,
  activeFilterCount,
  groupPullRequests,
  matchesPullRequest,
  parsePullRequestQuery,
  sortPullRequests,
  type PullRequestFilters,
  type PullRequestGroupKey,
  type PullRequestSort,
} from './pullRequestModel'
import { EmptyState, FiltersMenu, SkeletonRows, StateSegment } from './PullRequestParts'
import { PullRequestRow } from './PullRequestRow'
import { PullRequestBranchCard } from './PullRequestBranchCard'
import { PR_STATES, cacheKey, fetchFirstPage, isFetching, placeholderList, readCache, writeCache } from './pullRequestCache'
import type { PullRequestStateFilter } from '../../api/dockContracts'

const STALE_MS = 60_000
const GH_INSTALL_URL = 'https://cli.github.com/'

function openLink(url: string) {
  void api.openExternal(url).catch((err) => notifyError(String(err)))
}

function isStale(workdir: string, state: PullRequestStateFilter) {
  const cached = readCache(cacheKey(workdir, state))
  return !cached || Date.now() - cached.at >= STALE_MS
}

/**
 * 列表读取的唯一负责人。缓存是数据权威（迟到结果只写自己的键），这里决定何时刷新、
 * 何时预取其他状态，以及没有缓存时用什么占位。
 */
function usePullRequestList(workdir: string, state: PullRequestStateFilter, active: boolean) {
  const key = cacheKey(workdir, state)
  const [, setVersion] = useState(0)
  const rerender = useCallback(() => setVersion((value) => value + 1), [])
  const [moreKey, setMoreKey] = useState<string | null>(null)
  const [error, setError] = useState<{ key: string; message: string } | null>(null)
  const data = workdir ? readCache(key)?.data ?? null : null
  const placeholder = !data && workdir ? placeholderList(workdir, state) : null

  /** 首页到达后在后台取其他状态：之后切换不再等 gh 的一次往返。失败不提示，切过去时会重试。 */
  const prefetchOthers = useCallback((dir: string, current: PullRequestStateFilter) => {
    for (const other of PR_STATES) {
      if (other === current || isFetching(dir, other) || !isStale(dir, other)) continue
      fetchFirstPage(dir, other).then(rerender, () => {})
    }
  }, [rerender])

  const refresh = useCallback(async () => {
    if (!workdir) return
    const requestKey = key
    setError((prev) => (prev?.key === requestKey ? null : prev))
    const request = fetchFirstPage(workdir, state)
    rerender()
    try {
      const next = await request
      if (next.status === 'ready') prefetchOthers(workdir, state)
    } catch (err) {
      setError({ key: requestKey, message: String(err) })
    } finally {
      rerender()
    }
  }, [workdir, state, key, rerender, prefetchOthers])

  const loadMore = useCallback(async () => {
    const requestKey = key
    const cached = readCache(requestKey)
    if (!cached?.data.hasMore || !cached.data.endCursor) return
    setMoreKey(requestKey)
    try {
      const page = await dockApi.pullRequests(workdir, state, cached.data.endCursor)
      // 期间已刷新（第一页被替换）则丢弃：游标属于旧列表。
      if (readCache(requestKey)?.data !== cached.data) return
      const seen = new Set(cached.data.items.map((item) => item.number))
      writeCache(requestKey, {
        ...cached.data,
        items: [...cached.data.items, ...page.items.filter((item) => !seen.has(item.number))],
        endCursor: page.endCursor,
        hasMore: page.hasMore,
        totalCount: page.totalCount || cached.data.totalCount,
      }, cached.at)
      rerender()
    } catch (err) {
      notifyError(String(err))
    } finally {
      setMoreKey((prev) => (prev === requestKey ? null : prev))
    }
  }, [workdir, state, key, rerender])

  // 面板可见时才请求；有未过期的缓存则不打网络。
  useEffect(() => {
    if (!active || !workdir || !isStale(workdir, state)) return
    void refresh()
  }, [active, workdir, state, refresh])

  return {
    data,
    placeholder,
    loading: Boolean(workdir) && isFetching(workdir, state),
    loadingMore: moreKey === key,
    error: error?.key === key ? error.message : '',
    refresh,
    loadMore,
  }
}

type PullRequestsPanelProps = {
  workdir: string
  active: boolean
  lang: Lang
}

export function PullRequestsPanel({ workdir, active, lang }: PullRequestsPanelProps) {
  const t = i18n[lang]
  const [stateFilter, setStateFilter] = useState<PullRequestStateFilter>('open')
  const [search, setSearch] = useState('')
  const [sort, setSort] = useState<PullRequestSort>('updated')
  const [filters, setFilters] = useState<PullRequestFilters>(DEFAULT_FILTERS)
  const [filtersOpen, setFiltersOpen] = useState(false)
  const [expanded, setExpanded] = useState<{ workdir: string; key: string } | null>(null)
  const filterButtonRef = useRef<HTMLButtonElement>(null)
  const searchRef = useRef<HTMLInputElement>(null)
  const filtersPresence = usePresence(filtersOpen)
  const { data, placeholder, loading, loadingMore, error, refresh, loadMore } = usePullRequestList(workdir, stateFilter, active)
  // 没有该状态的数据时，用其他状态的缓存占位（变淡显示），控件不动。
  const view = data ?? placeholder
  const pending = !data

  const query = useMemo(() => parsePullRequestQuery(search), [search])
  const viewer = view?.viewer ?? ''
  const groups = useMemo(() => {
    if (!view) return []
    const visible = view.items.filter((item) => matchesPullRequest(item, query, filters, viewer))
    return groupPullRequests(sortPullRequests(visible, sort), viewer)
  }, [view, query, filters, sort, viewer])
  const expandedKey = expanded?.workdir === workdir ? expanded.key : null
  const toggle = (key: string) => setExpanded(expandedKey === key ? null : { workdir, key })
  const closeFilters = useCallback(() => setFiltersOpen(false), [])
  const reload = useCallback(() => { void refresh() }, [refresh])

  // 「/」聚焦搜索框（已在输入框内时不拦截）。
  useEffect(() => {
    if (!active) return
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== '/' || event.metaKey || event.ctrlKey || event.altKey) return
      if ((event.target as HTMLElement | null)?.closest('input,textarea,[contenteditable]')) return
      const input = searchRef.current
      if (!input?.offsetParent) return
      event.preventDefault()
      input.focus()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [active])

  if (!workdir) return <EmptyState icon={<GitBranch size={18} strokeWidth={1.6} />} title={t.dockNoWorkdir} />

  if (!view && error) {
    return (
      <div className="chat-pr-panel">
        <ErrorNotice message={error} actionLabel={t.prRetry} onAction={reload} />
        <EmptyState icon={<GitPullRequest size={18} strokeWidth={1.6} />} title={t.prEmptyState}>
          <Button size="sm" onClick={reload}><RefreshCw size={12} />{t.prRetry}</Button>
        </EmptyState>
      </div>
    )
  }

  if (data && data.status !== 'ready') {
    const retry = (
      <Button size="sm" onClick={reload} disabled={loading}>
        <RefreshCw size={12} className={loading ? 'animate-spin' : ''} />
        {t.prRetry}
      </Button>
    )
    if (data.status === 'gh_missing') {
      return (
        <EmptyState icon={<SquareTerminal size={18} strokeWidth={1.6} />} title={t.prGhMissing} hint={t.prGhMissingHint}>
          <Button size="sm" variant="primary" onClick={() => openLink(GH_INSTALL_URL)}>
            <ExternalLink size={12} />
            {t.prGhInstall}
          </Button>
          {retry}
        </EmptyState>
      )
    }
    if (data.status === 'gh_unauthenticated') {
      return <EmptyState icon={<KeyRound size={18} strokeWidth={1.6} />} title={t.prGhUnauth} hint={t.prGhUnauthHint}>{retry}</EmptyState>
    }
    if (data.status === 'no_github_remote') {
      return <EmptyState icon={<Unplug size={18} strokeWidth={1.6} />} title={t.prNoGithub} hint={t.prNoGithubHint}>{retry}</EmptyState>
    }
    return <EmptyState icon={<GitBranch size={18} strokeWidth={1.6} />} title={t.dockGitNotRepo} />
  }

  const filterCount = activeFilterCount(filters)
  const narrowed = Boolean(search.trim()) || filterCount > 0
  // 还没有任何可展示的列表（或占位里恰好没有该状态的 PR）：只在列表区域显示骨架。
  const listSkeleton = !view || (pending && groups.length === 0 && !narrowed)
  const groupTitle: Record<PullRequestGroupKey, string> = {
    review: t.prGroupReview,
    authored: t.prGroupAuthored,
    others: t.prGroupOthers,
  }
  const sortOptions = [
    { value: 'updated', label: t.prSortUpdated },
    { value: 'created', label: t.prSortCreated },
    { value: 'number', label: t.prSortNumber },
    { value: 'size', label: t.prSortSize },
  ]
  let rowIndex = 0

  return (
    <div className="chat-pr-panel">
      <ErrorNotice message={error} actionLabel={t.prRetry} onAction={reload} />
      <div className="chat-pr-header relative">
        <div className="chat-pr-repo">
          <FolderGit2 size={13} strokeWidth={1.75} className="shrink-0" />
          {view ? (
            <span className="chat-pr-repo-name" title={view.repository}>{view.repository}</span>
          ) : (
            <span className="kv-skeleton h-3 w-28 rounded" aria-hidden="true" />
          )}
          <span className="flex-1" />
          <IconButton label={t.prOpenRepo} size="xs" variant="ghost" disabled={!view} onClick={() => view && openLink(`${view.repositoryUrl}/pulls`)}>
            <ExternalLink size={12} />
          </IconButton>
          <IconButton label={t.dockRefresh} size="xs" variant="ghost" onClick={reload} disabled={loading}>
            <RefreshCw size={12} className={loading ? 'animate-spin' : ''} />
          </IconButton>
        </div>
        <div className="chat-pr-search">
          <Search size={13} aria-hidden="true" />
          <Input
            ref={searchRef}
            value={search}
            onChange={setSearch}
            placeholder={t.prSearchPlaceholder}
            aria-label={t.prSearchPlaceholder}
            title={t.prSearchHelp}
            spellCheck={false}
            onKeyDown={(event) => {
              if (event.key === 'Escape' && search) {
                event.stopPropagation()
                setSearch('')
              }
            }}
          />
          {search && (
            <IconButton label={t.prCancel} size="xs" variant="ghost" className="chat-pr-search-clear" onClick={() => { setSearch(''); searchRef.current?.focus() }}>
              <X size={12} />
            </IconButton>
          )}
        </div>
        <div className="chat-pr-controls">
          <StateSegment value={stateFilter} count={data && !loading ? data.totalCount : null} onChange={setStateFilter} t={t} />
          <Select
            value={sort}
            onChange={(value) => setSort(value as PullRequestSort)}
            options={sortOptions}
            size="sm"
            triggerIcon={<ArrowDownUp size={14} />}
            ariaLabel={`${t.prSort} · ${sortOptions.find((option) => option.value === sort)?.label}`}
          />
          <span className="chat-pr-filter-anchor">
            <IconButton
              ref={filterButtonRef}
              label={t.prFilters}
              aria-haspopup="menu"
              aria-expanded={filtersOpen}
              onClick={() => setFiltersOpen((value) => !value)}
            >
              <ListFilter size={14} />
            </IconButton>
            {filterCount > 0 && <span className="chat-pr-filter-dot" aria-hidden="true" />}
          </span>
        </div>
        {loading && <div className="chat-pr-progress" aria-hidden="true" />}
      </div>

      <div
        className={`custom-scrollbar min-h-0 flex-1 overflow-y-auto pb-3 chat-pr-list${pending ? ' is-pending' : ''}`}
        aria-busy={pending || undefined}
      >
        {view && (
          <PullRequestBranchCard
            key={`${workdir}\n${view.currentBranch}`}
            list={view}
            workdir={workdir}
            lang={lang}
            t={t}
            expanded={expandedKey === 'current'}
            onToggle={() => toggle('current')}
            onRefresh={reload}
          />
        )}

        {listSkeleton ? (
          <SkeletonRows />
        ) : groups.length === 0 ? (
          <EmptyState
            icon={narrowed ? <Search size={18} strokeWidth={1.6} /> : <GitPullRequest size={18} strokeWidth={1.6} />}
            title={narrowed ? t.prEmpty : t.prEmptyState}
          >
            {narrowed && (
              <Button size="sm" onClick={() => { setSearch(''); setFilters(DEFAULT_FILTERS) }}>
                <X size={12} />
                {t.prFiltersReset}
              </Button>
            )}
          </EmptyState>
        ) : (
          groups.map((group) => (
            <section key={group.key} aria-label={groupTitle[group.key]}>
              <div className="chat-pr-group-head">
                {groupTitle[group.key]}
                <span className="chat-pr-group-count">{group.items.length}</span>
              </div>
              {group.items.map((item) => {
                const key = `list:${item.number}`
                return (
                  <PullRequestRow
                    key={item.number}
                    item={item}
                    open={expandedKey === key}
                    index={rowIndex++}
                    terms={query.terms}
                    workdir={workdir}
                    currentBranch={view?.currentBranch ?? ''}
                    lang={lang}
                    t={t}
                    onToggle={() => toggle(key)}
                    onCheckedOut={reload}
                  />
                )
              })}
            </section>
          ))
        )}

        {data?.hasMore && (
          <div className="flex items-center justify-between gap-2 px-4 pt-3 text-[11px] text-[var(--text-faint)]">
            <span className="tabular-nums">
              {t.prLoadedCount.replace('{n}', String(data.items.length)).replace('{total}', String(data.totalCount))}
            </span>
            <Button size="sm" variant="ghost" onClick={() => void loadMore()} disabled={loadingMore || loading}>
              {loadingMore && <Loader2 size={12} className="animate-spin" />}
              {t.prLoadMore}
            </Button>
          </div>
        )}
      </div>

      {filtersPresence.present && filterButtonRef.current && (
        <FiltersMenu
          anchor={filterButtonRef.current}
          filters={filters}
          closing={filtersPresence.closing}
          onChange={setFilters}
          onClose={closeFilters}
          t={t}
        />
      )}
    </div>
  )
}
