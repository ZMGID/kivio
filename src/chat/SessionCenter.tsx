import { ErrorNotice } from '../components/AppNotice'
import { notifyError } from '../components/appNoticeStore'
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from 'react'
import { createPortal } from 'react-dom'
import {
  Archive,
  ArchiveRestore,
  Download,
  FolderKanban,
  Layers,
  Loader2,
  MessagesSquare,
  MoreHorizontal,
  Pin,
  PinOff,
  RefreshCw,
  Star,
  StarOff,
  Trash2,
} from 'lucide-react'
import { chatApi } from './api'
import type {
  ChatProject,
  ChatSet,
  ConversationLibraryOrder,
  ConversationLibraryQuery,
  ConversationLibraryShelf,
  ConversationLibrarySort,
  ConversationSearchHit,
} from './types'
import { IconButton, Button } from '../components/Button'
import { Checkbox, Input, SearchInput, Select } from '../settings/public/controls'
import { useT, type Lang } from '../components/i18n'
import {
  conversationOwnerLabel,
  dayBucket,
  dayBucketLabel,
  formatRelativeTime,
  libraryTimestamp,
  type DayBucket,
} from './sessionLibrary/format'
import { HighlightText } from './searchHighlight'
import { useClampedMenuPosition } from './useClampedMenuPosition'
import { confirmDialog } from '../components/dialogQueue'
import { useWindowStore } from '../utils/windowStore'
import {
  markSessionBatchNotified,
  markSessionBatchRefreshed,
  sessionBatchStore,
  startSessionBatch,
  type SessionBatchAction,
} from './sessionBatchStore'

const PAGE_SIZE = 80

/** 工具栏上的范围切换。后端还认 uncategorized / recent7d，对话库不再提供这两个入口。 */
type ShelfId = Extract<ConversationLibraryShelf, 'all' | 'starred' | 'archived'>

/** 归属筛选的取值：全部，或某个集 / 项目。 */
const OWNER_ALL = 'all'

function sameSelection(current: Set<string>, ids: readonly string[]): boolean {
  if (current.size !== ids.length) return false
  return ids.every((id) => current.has(id))
}

function failedBatch(
  ids: readonly string[],
  kinds: readonly SessionBatchAction['kind'][],
): SessionBatchAction | null {
  const snap = sessionBatchStore.getSnapshot()
  const action = snap.action
  if (snap.phase !== 'error' || !action || !kinds.includes(action.kind)) return null
  if (!sameSelection(new Set(ids), action.ids)) return null
  return action
}

function removesCurrentConversation(action: SessionBatchAction): boolean {
  if (action.kind === 'delete' || action.kind === 'delete-one') return true
  if (action.kind === 'archive' || action.kind === 'archive-one') return action.archived
  return false
}

/** Row delete keeps the single-conversation failure prefix; export keeps paths already written. */
function batchNotice(
  batch: { error: string; warnings: string[]; action: SessionBatchAction | null },
  failedPrefix: string,
): string {
  const base = batch.action?.kind === 'delete-one' && batch.error ? failedPrefix + batch.error : batch.error
  if (batch.action?.kind === 'export' && batch.warnings.length > 0) {
    return [base, ...batch.warnings].filter(Boolean).join('\n')
  }
  return base
}

interface SessionCenterProps {
  lang: Lang
  currentConversationId?: string
  generatingConversationIds?: ReadonlySet<string>
  onSelectConversation: (
    id: string,
    conversation?: ConversationSearchHit,
    scope?: { project: ChatProject | null; set: ChatSet | null },
  ) => void
  onConversationDeleted?: (id: string) => void
  onForceDropConversation?: (id: string) => void
  onConversationsChanged?: () => void
}

interface LibraryState {
  items: ConversationSearchHit[]
  total: number
  loading: boolean
  loadingMore: boolean
  error: string
}

export type { SessionCenterProps }

export function SessionCenter({
  lang,
  currentConversationId,
  generatingConversationIds = new Set(),
  onSelectConversation,
  onConversationDeleted,
  onForceDropConversation,
  onConversationsChanged,
}: SessionCenterProps) {
  const t = useT()
  const searchInputRef = useRef<HTMLInputElement>(null)
  const listRef = useRef<HTMLDivElement>(null)
  const rootRef = useRef<HTMLDivElement>(null)
  const lastClickedIdRef = useRef<string | null>(null)

  /** 对话库自身的宽度（不是视口）：聊天侧栏和设置导航都会吃掉一部分。 */
  const [pageWidth, setPageWidth] = useState(1200)
  const [shelf, setShelf] = useState<ShelfId>('all')
  const [projectId, setProjectId] = useState<string | null>(null)
  const [setId, setSetId] = useState<string | null>(null)
  const [searchInput, setSearchInput] = useState('')
  const [debouncedQ, setDebouncedQ] = useState('')
  const [sort, setSort] = useState<ConversationLibrarySort>('updated')
  const [order, setOrder] = useState<ConversationLibraryOrder>('desc')

  const compactPad = pageWidth < 640
  const showOwner = pageWidth >= 560

  useLayoutEffect(() => {
    const node = rootRef.current
    if (!node) return
    const measure = () => {
      const width = node.clientWidth
      if (width > 0) setPageWidth((prev) => (Math.abs(prev - width) < 1 ? prev : width))
    }
    measure()
    const ro = new ResizeObserver(measure)
    ro.observe(node)
    return () => ro.disconnect()
  }, [])

  const [projects, setProjects] = useState<ChatProject[]>([])
  const [sets, setSets] = useState<ChatSet[]>([])
  const [state, setState] = useState<LibraryState>({
    items: [],
    total: 0,
    loading: true,
    loadingMore: false,
    error: '',
  })
  const [selected, setSelected] = useState<Set<string>>(() => new Set())
  const [busy, setBusy] = useState(false)
  const [batch] = useWindowStore(sessionBatchStore)
  const controlsBusy = busy || batch.phase === 'running'
  const liveBatchRef = useRef({
    onConversationDeleted,
    onConversationsChanged,
    currentConversationId,
  })
  liveBatchRef.current = {
    onConversationDeleted,
    onConversationsChanged,
    currentConversationId,
  }
  const batchTextRef = useRef(t)
  batchTextRef.current = t
  const selectionRev = useRef(0)
  const selectionTouched = useRef(false)
  const [menu, setMenu] = useState<{ id: string; x: number; y: number } | null>(null)
  const [renameId, setRenameId] = useState<string | null>(null)
  const [renameDraft, setRenameDraft] = useState('')
  const [moveOpen, setMoveOpen] = useState<'set' | 'project' | null>(null)

  // debounce search
  useEffect(() => {
    const timer = window.setTimeout(() => setDebouncedQ(searchInput.trim()), 250)
    return () => window.clearTimeout(timer)
  }, [searchInput])

  // keyboard: / focus search, Esc clear selection / menu
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === '/' && !e.metaKey && !e.ctrlKey && !e.altKey) {
        const tag = (e.target as HTMLElement)?.tagName
        if (tag === 'INPUT' || tag === 'TEXTAREA' || (e.target as HTMLElement)?.isContentEditable) return
        e.preventDefault()
        searchInputRef.current?.focus()
      }
      if (e.key === 'Escape') {
        selectionTouched.current = true
        selectionRev.current += 1
        setSelected(new Set())
        setMenu(null)
        setMoveOpen(null)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  const buildQuery = useCallback(
    (offset: number): ConversationLibraryQuery => ({
      offset,
      limit: PAGE_SIZE,
      sort,
      order,
      q: debouncedQ || undefined,
      fullText: true,
      shelf,
      projectId: projectId,
      setId: setId,
    }),
    [debouncedQ, order, projectId, setId, shelf, sort],
  )

  const itemsLenRef = useRef(0)
  itemsLenRef.current = state.items.length
  const loadingMoreRef = useRef(false)
  /** 书架/筛选切换时丢弃过期响应，避免旧页回写造成闪一下。 */
  const loadSeqRef = useRef(0)
  const applySelectionRef = useRef<(revAtStart: number) => void>(() => {})
  applySelectionRef.current = (revAtStart: number) => {
    if (selectionRev.current !== revAtStart) return
    const pending = sessionBatchStore.getSnapshot()
    const action = pending.action
    if ((pending.phase === 'running' || pending.phase === 'error') && action) {
      setSelected((current) => {
        if (selectionTouched.current && !sameSelection(current, action.ids)) return current
        return new Set(action.ids)
      })
      return
    }
    setSelected((current) => {
      if (
        pending.phase === 'success'
        && action
        && selectionTouched.current
        && !sameSelection(current, action.ids)
      ) return current
      return new Set()
    })
  }

  const loadPage = useCallback(
    async (opts?: { append?: boolean }) => {
      const append = opts?.append ?? false
      const revAtStart = selectionRev.current
      if (append) {
        if (loadingMoreRef.current) return
        loadingMoreRef.current = true
      } else {
        loadSeqRef.current += 1
      }
      const seq = loadSeqRef.current
      setState((s) => ({
        ...s,
        // 已有列表时只标 loading，不先清空，避免书架切换白闪
        loading: !append,
        loadingMore: append,
        error: '',
      }))
      try {
        const offset = append ? itemsLenRef.current : 0
        const [page, projectData, setData] = await Promise.all([
          chatApi.queryConversations(buildQuery(offset)),
          append ? Promise.resolve(null) : chatApi.getProjects(),
          append ? Promise.resolve(null) : chatApi.getSets(),
        ])
        if (seq !== loadSeqRef.current) return
        if (!append) {
          if (projectData) setProjects(projectData)
          if (setData) setSets(setData)
        }
        setState((s) => ({
          items: append ? [...s.items, ...page.items] : page.items,
          total: page.total,
          loading: false,
          loadingMore: false,
          error: '',
        }))
        if (!append) applySelectionRef.current(revAtStart)
      } catch (err) {
        if (seq !== loadSeqRef.current) return
        setState((s) => ({
          ...s,
          loading: false,
          loadingMore: false,
          error: err instanceof Error ? err.message : String(err),
        }))
      } finally {
        if (append) loadingMoreRef.current = false
      }
    },
    [buildQuery],
  )

  // reset & reload when filters change
  useEffect(() => {
    void loadPage({ append: false })
  }, [loadPage])

  useLayoutEffect(() => {
    selectionTouched.current = false
    const pending = sessionBatchStore.getSnapshot()
    if ((pending.phase === 'running' || pending.phase === 'error') && pending.action) {
      setSelected(new Set(pending.action.ids))
    }
  }, [])

  const loadPageRef = useRef(loadPage)
  loadPageRef.current = loadPage
  useEffect(() => {
    const snap = sessionBatchStore.getSnapshot()
    if (snap.phase === 'running' || snap.settledGeneration === 0) return
    const generation = snap.settledGeneration
    if (snap.notifiedGeneration !== generation) {
      markSessionBatchNotified(generation)
      const claimed = sessionBatchStore.getSnapshot()
      if (claimed.notifiedGeneration !== generation || claimed.phase === 'running') return
      const action = claimed.action
      const currentId = liveBatchRef.current.currentConversationId
      if (claimed.phase === 'success' && action && currentId && action.ids.includes(currentId) && removesCurrentConversation(action)) {
        liveBatchRef.current.onConversationDeleted?.(currentId)
      }
      liveBatchRef.current.onConversationsChanged?.()
      const text = batchTextRef.current
      if (claimed.phase === 'error' && claimed.error) {
        void notifyError(batchNotice(claimed, text.chatDeleteConversationFailed))
      }
      if (claimed.phase === 'success' && action && (action.kind === 'delete' || action.kind === 'delete-one') && claimed.warnings.length > 0) {
        const lines = action.kind === 'delete' ? claimed.warnings.slice(0, 8) : claimed.warnings
        void notifyError(text.chatDeleteConversationPartial + lines.join('\n'))
      }
    }
    const pending = sessionBatchStore.getSnapshot()
    if (pending.phase === 'running' || pending.settledGeneration !== generation) return
    if (pending.refreshedGeneration === generation) return
    let active = true
    void (async () => {
      await loadPageRef.current({ append: false })
      if (!active) return
      const latest = sessionBatchStore.getSnapshot()
      if (latest.phase === 'running' || latest.settledGeneration !== generation) return
      if (latest.refreshedGeneration === generation) return
      markSessionBatchRefreshed(generation)
    })()
    return () => { active = false }
  }, [batch.phase, batch.settledGeneration])

  const hasMore = state.items.length < state.total

  const onScrollList = useCallback(() => {
    const el = listRef.current
    if (!el || state.loadingMore || !hasMore || state.loading) return
    if (el.scrollHeight - el.scrollTop - el.clientHeight < 240) {
      void loadPage({ append: true })
    }
  }, [hasMore, loadPage, state.loading, state.loadingMore])

  const notify = useCallback(() => {
    onConversationsChanged?.()
  }, [onConversationsChanged])

  const markSelectionTouched = useCallback(() => {
    selectionTouched.current = true
    selectionRev.current += 1
  }, [])

  const holdSubmittedSelection = (ids: string[]) => {
    selectionRev.current += 1
    setSelected(new Set(ids))
  }

  const toggleSelect = useCallback((id: string, shiftKey: boolean) => {
    markSelectionTouched()
    setSelected((prev) => {
      const next = new Set(prev)
      if (shiftKey && lastClickedIdRef.current) {
        const ids = state.items.map((c) => c.id)
        const a = ids.indexOf(lastClickedIdRef.current)
        const b = ids.indexOf(id)
        if (a >= 0 && b >= 0) {
          const [lo, hi] = a < b ? [a, b] : [b, a]
          for (let i = lo; i <= hi; i++) next.add(ids[i])
          return next
        }
      }
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
    lastClickedIdRef.current = id
  }, [markSelectionTouched, state.items])

  const selectAllVisible = useCallback(() => {
    markSelectionTouched()
    setSelected(new Set(state.items.map((c) => c.id)))
  }, [markSelectionTouched, state.items])

  const clearSelection = useCallback(() => {
    markSelectionTouched()
    setSelected(new Set())
  }, [markSelectionTouched])

  const selectedIds = useMemo(() => [...selected], [selected])

  const batchRunning = () => sessionBatchStore.getSnapshot().phase === 'running'

  const bulkPin = (pinned: boolean) => {
    if (selectedIds.length === 0 || busy || batchRunning()) return
    holdSubmittedSelection(selectedIds)
    void startSessionBatch({ kind: 'pin', pinned, ids: selectedIds })
  }

  const bulkArchive = (archived: boolean) => {
    if (selectedIds.length === 0 || busy || batchRunning()) return
    const retry = failedBatch(selectedIds, ['archive', 'archive-one'])
    holdSubmittedSelection(selectedIds)
    if (retry?.kind === 'archive-one') {
      void startSessionBatch(retry)
      return
    }
    void startSessionBatch({ kind: 'archive', archived, ids: selectedIds })
  }

  const bulkMoveProject = (pid: string | null) => {
    if (selectedIds.length === 0 || busy || batchRunning()) return
    setMoveOpen(null)
    holdSubmittedSelection(selectedIds)
    void startSessionBatch({ kind: 'move-project', projectId: pid, ids: selectedIds })
  }

  const bulkMoveSet = (sid: string | null) => {
    if (selectedIds.length === 0 || busy || batchRunning()) return
    setMoveOpen(null)
    holdSubmittedSelection(selectedIds)
    void startSessionBatch({ kind: 'move-set', setId: sid, ids: selectedIds })
  }

  const bulkDelete = async () => {
    const ids = [...selectedIds]
    if (ids.length === 0 || busy || batchRunning()) return
    if (!(await confirmDialog({ message: t.chatLibBulkDeleteConfirm.replace('{n}', String(ids.length)), confirmLabel: t.dialogDelete, danger: true }))) return
    if (batchRunning()) return
    for (const id of ids) {
      if (generatingConversationIds.has(id)) onForceDropConversation?.(id)
    }
    holdSubmittedSelection(ids)
    const retry = failedBatch(ids, ['delete-one'])
    void startSessionBatch(retry ?? { kind: 'delete', ids })
  }

  const bulkExport = () => {
    if (selectedIds.length === 0 || busy || batchRunning()) return
    const targets = selectedIds.map((id) => {
      const row = state.items.find((item) => item.id === id)
      return { id, title: row?.title || id }
    })
    holdSubmittedSelection(selectedIds)
    void startSessionBatch({ kind: 'export', ids: selectedIds, targets, lang })
  }

  const patchOne = useCallback(
    async (id: string, patch: { pinned?: boolean; archived?: boolean; title?: string; projectId?: string | null; setId?: string | null }) => {
      if (busy || sessionBatchStore.getSnapshot().phase === 'running') return
      const archiveOnly = patch.archived !== undefined
        && patch.pinned === undefined
        && patch.title === undefined
        && patch.projectId === undefined
        && patch.setId === undefined
      if (archiveOnly) {
        setMenu(null)
        selectionRev.current += 1
        setSelected(new Set([id]))
        void startSessionBatch({ kind: 'archive-one', archived: patch.archived === true, ids: [id] })
        return
      }
      setBusy(true)
      try {
        await chatApi.updateConversation(id, patch)
        notify()
        await loadPage({ append: false })
      } catch (err) {
        void notifyError(err instanceof Error ? err.message : String(err))
      } finally {
        setBusy(false)
        setMenu(null)
        setRenameId(null)
      }
    },
    [busy, loadPage, notify],
  )

  const regenerateTitle = useCallback(
    async (id: string) => {
      if (busy || sessionBatchStore.getSnapshot().phase === 'running') return
      setBusy(true)
      setMenu(null)
      try {
        await chatApi.regenerateConversationTitle(id)
        notify()
        await loadPage({ append: false })
      } catch (err) {
        void notifyError(
          t.chatRegenerateTitleFailed + (err instanceof Error ? err.message : String(err)),
        )
      } finally {
        setBusy(false)
      }
    },
    [busy, loadPage, notify, t],
  )

  const deleteOne = useCallback(
    async (id: string) => {
      if (busy || sessionBatchStore.getSnapshot().phase === 'running') return
      if (!(await confirmDialog({ message: t.chatDeleteConversationConfirm, confirmLabel: t.dialogDelete, danger: true }))) return
      if (sessionBatchStore.getSnapshot().phase === 'running') return
      if (generatingConversationIds.has(id)) onForceDropConversation?.(id)
      setMenu(null)
      selectionRev.current += 1
      setSelected(new Set([id]))
      void startSessionBatch({ kind: 'delete-one', ids: [id] })
    },
    [busy, generatingConversationIds, onForceDropConversation, t],
  )

  // 按时间排序时按「今天 / 昨天 / 本周 / 更早」分段；按标题或消息数排序时分段没有意义。
  const grouped = useMemo(() => {
    if (sort !== 'updated' && sort !== 'created') {
      return [{ key: 'all', label: '', items: state.items }]
    }
    const map = new Map<DayBucket, ConversationSearchHit[]>()
    for (const c of state.items) {
      const bucket = dayBucket(libraryTimestamp(c, sort))
      const items = map.get(bucket)
      if (items) items.push(c)
      else map.set(bucket, [c])
    }
    return [...map.entries()].map(([key, items]) => ({ key, label: dayBucketLabel(key, t), items }))
  }, [sort, state.items, t])

  const scopes: Array<{ id: ShelfId; label: string }> = [
    { id: 'all', label: t.chatLibShelfAll },
    { id: 'starred', label: t.chatLibShelfStarred },
    { id: 'archived', label: t.chatLibShelfArchived },
  ]

  const ownerValue = setId ? `set:${setId}` : projectId ? `project:${projectId}` : OWNER_ALL
  const ownerOptions = useMemo(
    () => [
      { value: OWNER_ALL, label: t.chatLibOwnerAll },
      ...sets.map((s) => ({ value: `set:${s.id}`, label: `${t.chatSetPrefix} · ${s.name}` })),
      ...projects.map((p) => ({ value: `project:${p.id}`, label: p.name })),
    ],
    [projects, sets, t],
  )
  const pickOwner = (value: string) => {
    const [kind, ...rest] = value.split(':')
    const id = rest.join(':')
    setSetId(kind === 'set' ? id : null)
    setProjectId(kind === 'project' ? id : null)
  }

  const menuConv = menu ? state.items.find((c) => c.id === menu.id) : undefined

  const selectionScope = useCallback((conversation: ConversationSearchHit) => {
    const conversationProjectId = conversation.project_id ?? conversation.projectId ?? null
    const conversationSetId = conversation.set_id ?? conversation.setId ?? null
    return {
      project: projects.find((project) => (
        conversationProjectId
          ? project.id === conversationProjectId
          : Boolean(conversation.folder) && project.name === conversation.folder
      )) ?? null,
      set: sets.find((set) => set.id === conversationSetId) ?? null,
    }
  }, [projects, sets])
  const sortOptions = useMemo(
    () => [
      { value: 'updated:desc', label: t.chatLibSortUpdatedDesc },
      { value: 'created:desc', label: t.chatLibSortCreatedDesc },
      { value: 'title:asc', label: t.chatLibSortTitleAsc },
      { value: 'messages:desc', label: t.chatLibSortMessagesDesc },
    ],
    [t],
  )

  const allSelectedPinned = selected.size > 0
    && state.items.some((c) => selected.has(c.id))
    && state.items.every((c) => !selected.has(c.id) || c.pinned)
  const filtered = Boolean(debouncedQ) || shelf !== 'all' || ownerValue !== OWNER_ALL

  return (
    <div
      ref={rootRef}
      className="assistant-center-root flex h-full min-h-0 flex-col pt-1"
    >
      <div
        className={`mx-auto flex min-h-0 w-full max-w-[1200px] flex-1 flex-col ${compactPad ? 'px-2 pb-2' : 'px-4 pb-4'}`}
      >
        {/* 一行工具栏：搜索、范围（全部 / 收藏 / 归档）、归属、排序、刷新。 */}
        <div className="flex shrink-0 flex-wrap items-center gap-2 pb-3">
          <SearchInput
            ref={searchInputRef}
            className="min-w-[180px] flex-1"
            type="search"
            value={searchInput}
            onChange={setSearchInput}
            placeholder={t.chatSessionSearchPlaceholder}
            aria-label={t.chatSessionSearchPlaceholder}
            clearLabel={t.dialogClear}
          />
          <div className="kv-seg shrink-0" role="group" aria-label={t.chatLibScope}>
            {scopes.map(({ id, label }) => (
              <button
                key={id}
                type="button"
                className={shelf === id ? 'active' : ''}
                aria-pressed={shelf === id}
                onClick={() => setShelf(id)}
              >
                {label}
              </button>
            ))}
          </div>
          {ownerOptions.length > 1 && (
            <Select
              className="w-[148px] shrink-0"
              value={ownerValue}
              onChange={pickOwner}
              options={ownerOptions}
              ariaLabel={t.chatLibColOwner}
            />
          )}
          <Select
            className="w-[124px] shrink-0"
            value={`${sort}:${order}`}
            onChange={(value) => {
              const [s, o] = value.split(':') as [ConversationLibrarySort, ConversationLibraryOrder]
              setSort(s)
              setOrder(o)
            }}
            options={sortOptions}
            ariaLabel={t.chatLibSort}
          />
          <IconButton label={t.chatSessionRefresh} onClick={() => void loadPage({ append: false })} disabled={state.loading || busy}>
            <RefreshCw size={14} className={state.loading ? 'animate-spin' : ''} />
          </IconButton>
        </div>

        {state.error && (
          <ErrorNotice message={state.error} />
        )}

        <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl border border-[var(--theme-surface-border)] bg-[var(--theme-surface)]">
          {/* 列表顶条：平时是全选框 + 总数；选中后原地变成批量操作。 */}
          <div
            className="flex min-h-[38px] shrink-0 flex-wrap items-center gap-1.5 border-b border-[var(--theme-surface-border)] px-3 py-1"
            aria-busy={batch.phase === 'running'}
          >
            <span className="flex w-7 shrink-0 items-center justify-center">
              <Checkbox
                ariaLabel={t.editSelectAll}
                checked={state.items.length > 0 && selected.size === state.items.length}
                indeterminate={selected.size > 0 && selected.size < state.items.length}
                onChange={(checked) => (checked ? selectAllVisible() : clearSelection())}
              />
            </span>
            {selected.size === 0 ? (
              <span className="text-[12px] tabular-nums text-[var(--text-muted)]">
                {state.loading ? t.chatLoading : t.chatLibCount.replace('{n}', String(state.total))}
              </span>
            ) : (
              <>
                <span className="mr-1 text-[12px] font-medium tabular-nums text-[var(--text)]">
                  {t.chatLibSelected.replace('{n}', String(selected.size))}
                </span>
                {allSelectedPinned ? (
                  <Button size="sm" disabled={controlsBusy} aria-label={t.chatLibUnstar} onClick={() => bulkPin(false)}>
                    <StarOff size={12} />
                    {!compactPad && <span>{t.chatLibUnstar}</span>}
                  </Button>
                ) : (
                  <Button size="sm" disabled={controlsBusy} aria-label={t.chatLibStar} onClick={() => bulkPin(true)}>
                    <Star size={12} />
                    {!compactPad && <span>{t.chatLibStar}</span>}
                  </Button>
                )}
                {sets.length > 0 && (
                  <div className="relative">
                    <Button size="sm" disabled={controlsBusy} aria-label={t.chatLibMoveToSet} aria-haspopup="menu" aria-expanded={moveOpen === 'set'} onClick={() => setMoveOpen(moveOpen === 'set' ? null : 'set')}>
                      <Layers size={12} />
                      {!compactPad && <span>{t.chatLibMoveToSet}</span>}
                    </Button>
                    {moveOpen === 'set' && (
                      <MoveMenu
                        clearLabel={t.chatLibClearOwner}
                        targets={sets}
                        onPick={bulkMoveSet}
                      />
                    )}
                  </div>
                )}
                {projects.length > 0 && (
                  <div className="relative">
                    <Button size="sm" disabled={controlsBusy} aria-label={t.chatLibMoveToProject} aria-haspopup="menu" aria-expanded={moveOpen === 'project'} onClick={() => setMoveOpen(moveOpen === 'project' ? null : 'project')}>
                      <FolderKanban size={12} />
                      {!compactPad && <span>{t.chatLibMoveToProject}</span>}
                    </Button>
                    {moveOpen === 'project' && (
                      <MoveMenu
                        clearLabel={t.chatLibClearOwner}
                        targets={projects}
                        onPick={bulkMoveProject}
                      />
                    )}
                  </div>
                )}
                {shelf === 'archived' ? (
                  <Button size="sm" disabled={controlsBusy} aria-label={t.chatLibUnarchive} onClick={() => bulkArchive(false)}>
                    <ArchiveRestore size={12} />
                    {!compactPad && <span>{t.chatLibUnarchive}</span>}
                  </Button>
                ) : (
                  <Button size="sm" disabled={controlsBusy} aria-label={t.chatLibArchive} onClick={() => bulkArchive(true)}>
                    <Archive size={12} />
                    {!compactPad && <span>{t.chatLibArchive}</span>}
                  </Button>
                )}
                <Button size="sm" disabled={controlsBusy} aria-label={t.chatLibExport} onClick={bulkExport}>
                  <Download size={12} />
                  {!compactPad && <span>{t.chatLibExport}</span>}
                </Button>
                <Button size="sm" variant="danger" disabled={controlsBusy} aria-label={t.chatLibDelete} onClick={() => void bulkDelete()}>
                  <Trash2 size={12} />
                  {!compactPad && <span>{t.chatLibDelete}</span>}
                </Button>
                <Button size="sm" variant="ghost" className="ml-auto" onClick={clearSelection}>
                  {t.chatLibClearSelection}
                </Button>
              </>
            )}
          </div>

          <div
            ref={listRef}
            onScroll={onScrollList}
            className="custom-scrollbar min-h-0 flex-1 overflow-x-hidden overflow-y-auto"
          >
            {state.loading && state.items.length === 0 ? (
              <div className="flex flex-col gap-2 p-4">
                {Array.from({ length: 8 }, (_, i) => (
                  <div key={i} className="kv-skeleton h-11 rounded-md" />
                ))}
              </div>
            ) : state.items.length === 0 ? (
              <div className="flex flex-col items-center justify-center px-6 py-16 text-center">
                <MessagesSquare size={22} strokeWidth={1.5} className="text-[var(--text-faint)]" />
                <p className="mt-3 text-[13px] text-[var(--text-muted)]">
                  {debouncedQ ? t.chatSessionEmptySearch : t.chatSessionEmpty}
                </p>
                {filtered && (
                  <p className="mt-1 max-w-sm text-[12px] text-[var(--text-faint)]">{t.chatLibEmptyHint}</p>
                )}
              </div>
            ) : (
              grouped.map((group) => (
                <div key={group.key}>
                  {group.label && (
                    <div className="sticky top-0 z-[5] bg-[var(--theme-surface)] px-3 pb-1 pt-2.5 text-[12px] font-medium text-[var(--text-muted)]">
                      {group.label}
                    </div>
                  )}
                  {group.items.map((c) => {
                    const isSel = selected.has(c.id)
                    const isCurrent = c.id === currentConversationId
                    const owner = conversationOwnerLabel(c, projects, sets, t)
                    // 有搜索词时优先展示命中片段（正文/思考），否则回退 index 预览。
                    const snippet = (c.match_snippet ?? c.matchSnippet ?? '').trim()
                    const line = debouncedQ
                      ? (snippet && snippet !== (c.title || '') ? snippet : (c.preview || snippet))
                      : c.preview
                    return (
                      <div
                        key={c.id}
                        role="button"
                        tabIndex={0}
                        onClick={(e) => {
                          if ((e.target as HTMLElement).closest('[data-row-chrome]')) return
                          onSelectConversation(c.id, c, selectionScope(c))
                        }}
                        onKeyDown={(e: ReactKeyboardEvent) => {
                          if (e.target !== e.currentTarget) return
                          if (e.key === 'Enter') onSelectConversation(c.id, c, selectionScope(c))
                        }}
                        className={`group flex cursor-default items-center gap-2 border-b border-[var(--theme-surface-border)] px-3 py-2 transition-colors hover:bg-[var(--theme-surface-hover)] ${
                          isSel ? 'bg-[var(--accent-soft)]' : ''
                        } ${isCurrent ? 'ring-1 ring-inset ring-[var(--accent)]/35' : ''}`}
                      >
                        <span data-row-chrome className="flex w-7 shrink-0 items-center justify-center" onClick={(e) => e.stopPropagation()}>
                          <Checkbox
                            ariaLabel={c.title}
                            checked={isSel}
                            onChange={(_, e) => toggleSelect(c.id, (e.nativeEvent as MouseEvent).shiftKey)}
                          />
                        </span>
                        <div className="min-w-0 flex-1 overflow-hidden">
                          <div className="flex min-w-0 items-center gap-1.5">
                            {c.pinned && <Star size={12} className="shrink-0 fill-[var(--warn)] text-[var(--warn)]" />}
                            {renameId === c.id ? (
                              <Input
                                data-row-chrome
                                autoFocus
                                className="min-w-0 flex-1"
                                value={renameDraft}
                                onChange={setRenameDraft}
                                aria-label={t.chatRename}
                                onClick={(e) => e.stopPropagation()}
                                onKeyDown={(e) => {
                                  if (e.key === 'Enter') void patchOne(c.id, { title: renameDraft.trim() || c.title })
                                  if (e.key === 'Escape') setRenameId(null)
                                }}
                                onBlur={() => {
                                  if (renameDraft.trim() && renameDraft.trim() !== c.title) {
                                    void patchOne(c.id, { title: renameDraft.trim() })
                                  } else setRenameId(null)
                                }}
                              />
                            ) : (
                              <span className="min-w-0 truncate text-[13px] font-medium text-[var(--text)]">
                                {debouncedQ
                                  ? <HighlightText text={c.title || t.chatLibUntitled} query={debouncedQ} />
                                  : (c.title || t.chatLibUntitled)}
                              </span>
                            )}
                          </div>
                          {line && (
                            <p className="mt-0.5 truncate text-[12px] leading-snug text-[var(--text-muted)]">
                              {debouncedQ
                                ? <HighlightText text={line} query={debouncedQ} />
                                : line}
                            </p>
                          )}
                        </div>
                        {showOwner && owner && (
                          <span className="max-w-[120px] shrink-0 truncate rounded-full bg-[var(--theme-surface-hover)] px-2 py-0.5 text-[11px] text-[var(--text-muted)]">
                            {owner}
                          </span>
                        )}
                        <span className="w-16 shrink-0 text-right text-[12px] tabular-nums text-[var(--text-faint)]">
                          {formatRelativeTime(libraryTimestamp(c, sort), t)}
                        </span>
                        <span data-row-chrome className="shrink-0 opacity-60 group-hover:opacity-100 group-focus-within:opacity-100">
                          <IconButton
                            data-row-chrome
                            label={t.chatLibMore}
                            aria-haspopup="menu"
                            onClick={(e) => {
                              e.stopPropagation()
                              const rect = (e.currentTarget as HTMLElement).getBoundingClientRect()
                              const menuW = 180
                              const x = Math.min(rect.right - menuW, window.innerWidth - menuW - 8)
                              setMenu({ id: c.id, x: Math.max(8, x), y: rect.bottom + 4 })
                            }}
                          >
                            <MoreHorizontal size={15} />
                          </IconButton>
                        </span>
                      </div>
                    )
                  })}
                </div>
              ))
            )}

            {state.loadingMore && (
              <div className="flex items-center justify-center gap-2 py-3 text-[12px] text-[var(--text-faint)]">
                <Loader2 size={14} className="animate-spin" /> {t.chatLoading}
              </div>
            )}
            {!state.loading && !state.loadingMore && hasMore && (
              <div className="flex justify-center py-2">
                <Button size="sm" variant="ghost" onClick={() => void loadPage({ append: true })}>
                  {t.chatLibLoadMore}
                </Button>
              </div>
            )}
          </div>
        </div>
      </div>

      {/* Row context menu */}
      {menu && menuConv && (
        <PortaledRowMenu anchor={{ left: menu.x, top: menu.y }} onClose={() => setMenu(null)}>
            <MenuItem
              label={menuConv.pinned ? t.chatLibUnstar : t.chatLibStar}
              icon={menuConv.pinned ? <PinOff size={13} /> : <Pin size={13} />}
              disabled={controlsBusy}
              onClick={() => void patchOne(menuConv.id, { pinned: !menuConv.pinned })}
            />
            <MenuItem
              label={t.chatRename}
              onClick={() => {
                setRenameId(menuConv.id)
                setRenameDraft(menuConv.title)
                setMenu(null)
              }}
            />
            <MenuItem
              label={t.chatRegenerateTitle}
              icon={<RefreshCw size={13} />}
              disabled={(menuConv.message_count ?? 0) === 0 || controlsBusy}
              onClick={() => void regenerateTitle(menuConv.id)}
            />
            <MenuItem
              label={menuConv.archived ? t.chatLibUnarchive : t.chatLibArchive}
              icon={menuConv.archived ? <ArchiveRestore size={13} /> : <Archive size={13} />}
              disabled={controlsBusy}
              onClick={() => void patchOne(menuConv.id, { archived: !menuConv.archived })}
            />
            <MenuItem
              label={t.chatLibExport}
              disabled={controlsBusy}
              onClick={() => {
                if (sessionBatchStore.getSnapshot().phase === 'running') return
                const id = menuConv.id
                setMenu(null)
                selectionRev.current += 1
                setSelected(new Set([id]))
                void startSessionBatch({
                  kind: 'export',
                  ids: [id],
                  targets: [{ id, title: menuConv.title || id }],
                  lang,
                })
              }}
            />
            <div className="my-1 border-t border-[var(--theme-surface-border)]" />
            <MenuItem
              label={t.chatLibDelete}
              danger
              icon={<Trash2 size={13} />}
              disabled={controlsBusy}
              onClick={() => void deleteOne(menuConv.id)}
            />
        </PortaledRowMenu>
      )}
    </div>
  )
}

/** 批量「移入集 / 移入项目」的下拉：第一项是移出归属。 */
function MoveMenu({
  clearLabel,
  targets,
  onPick,
}: {
  clearLabel: string
  targets: Array<{ id: string; name: string }>
  onPick: (id: string | null) => void
}) {
  const itemClass = 'block w-full truncate px-3 py-1.5 text-left text-[12px] text-[var(--text)] hover:bg-[var(--theme-surface-hover)]'
  return (
    <div
      role="menu"
      className="custom-scrollbar absolute left-0 top-full z-20 mt-1 max-h-56 w-48 overflow-y-auto rounded-[10px] border border-[var(--theme-surface-border)] bg-[var(--theme-surface)] py-1 shadow-lg"
    >
      <button type="button" role="menuitem" className={itemClass} onClick={() => onPick(null)}>
        {clearLabel}
      </button>
      {targets.map((target) => (
        <button key={target.id} type="button" role="menuitem" className={itemClass} onClick={() => onPick(target.id)}>
          {target.name}
        </button>
      ))}
    </div>
  )
}

function PortaledRowMenu({
  anchor,
  onClose,
  children,
}: {
  anchor: { left: number; top: number }
  onClose: () => void
  children: React.ReactNode
}) {
  const menuRef = useRef<HTMLDivElement>(null)
  const pos = useClampedMenuPosition(menuRef, anchor)

  return createPortal(
    <>
      <div className="fixed inset-0 z-[199]" onClick={onClose} />
      <div
        ref={menuRef}
        role="menu"
        className="fixed z-[200] w-[180px] rounded-[10px] border border-[var(--theme-surface-border)] bg-[var(--theme-surface)] py-1 shadow-xl"
        style={{ left: pos.left, top: pos.top }}
      >
        {children}
      </div>
    </>,
    document.body,
  )
}

function MenuItem({
  label,
  onClick,
  icon,
  danger,
  disabled,
}: {
  label: string
  onClick: () => void
  icon?: React.ReactNode
  danger?: boolean
  disabled?: boolean
}) {
  return (
    <button
      type="button"
      role="menuitem"
      onClick={onClick}
      disabled={disabled}
      className={`flex w-full items-center gap-2 px-3 py-1.5 text-left text-[13px] hover:bg-[var(--theme-surface-hover)] disabled:cursor-default disabled:opacity-40 ${
        danger ? 'text-[var(--danger)]' : 'text-[var(--text)]'
      }`}
    >
      {icon}
      <span className="flex-1">{label}</span>
    </button>
  )
}
