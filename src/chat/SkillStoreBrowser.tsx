// ClawHub 技能商店内联浏览（无 modal 外壳，供 SkillCenter「技能商店」tab 用）。
// 排序/搜索/翻页 + 一键安装（下载走后端 chat_skills_install_from_url）。数据层见 ../settings/public/skills.ts。

import { useCallback, useEffect, useRef, useState, type CSSProperties } from 'react'
import { Check, Download, ExternalLink, Loader2, Search, Star } from 'lucide-react'
import { api } from '../api/tauri'
import { Button, IconButton } from '../components/Button'
import { Select } from '../settings/public/controls'
import { useT } from '../components/i18n'
import {
  CLAWHUB_SORT_OPTIONS,
  listClawHubSkills,
  searchClawHubSkills,
  type ClawHubSkillCard,
  type ClawHubSort,
} from '../settings/public/skills'
import { useWindowStore } from '../utils/windowStore'
import { installStoreSkill, isStoreSlugInstalled, skillLifecycleStore } from './skillLifecycle'

const PAGE_LIMIT = 24

export function SkillStoreBrowser() {
  const t = useT()
  const [ops] = useWindowStore(skillLifecycleStore)
  const [sort, setSort] = useState<ClawHubSort>('downloads')
  const [queryInput, setQueryInput] = useState('')
  const [query, setQuery] = useState('')
  const [items, setItems] = useState<ClawHubSkillCard[]>([])
  const [cursor, setCursor] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const [loadingMore, setLoadingMore] = useState(false)
  const [error, setError] = useState('')

  useEffect(() => {
    const timer = setTimeout(() => setQuery(queryInput.trim()), 400)
    return () => clearTimeout(timer)
  }, [queryInput])

  const reqSeq = useRef(0)
  useEffect(() => {
    const reqId = ++reqSeq.current
    setLoading(true)
    setError('')
    setItems([])
    setCursor(null)
    const load = query
      ? searchClawHubSkills({ query, limit: PAGE_LIMIT }).then((results) => ({ items: results, nextCursor: null }))
      : listClawHubSkills({ sort, limit: PAGE_LIMIT })
    load
      .then((res) => {
        if (reqId !== reqSeq.current) return
        setItems(res.items)
        setCursor(res.nextCursor)
      })
      .catch((err) => {
        if (reqId !== reqSeq.current) return
        setError(err instanceof Error ? err.message : String(err))
      })
      .finally(() => {
        if (reqId === reqSeq.current) setLoading(false)
      })
  }, [sort, query])

  const loadMore = useCallback(() => {
    if (!cursor || loadingMore || query) return
    const reqId = reqSeq.current
    setLoadingMore(true)
    listClawHubSkills({ sort, cursor, limit: PAGE_LIMIT })
      .then((res) => {
        if (reqId !== reqSeq.current) return
        setItems((prev) => [...prev, ...res.items])
        setCursor(res.nextCursor)
      })
      .catch((err) => {
        if (reqId !== reqSeq.current) return
        setError(err instanceof Error ? err.message : String(err))
      })
      .finally(() => {
        if (reqId === reqSeq.current) setLoadingMore(false)
      })
  }, [cursor, loadingMore, query, sort])

  const banner = ops.storeInstallError || error

  const sortLabels: Record<ClawHubSort, string> = {
    downloads: t.chatSkillSortDownloads,
    stars: t.chatSkillSortStars,
    installs: t.chatSkillSortInstalls,
    updated: t.chatSkillSortUpdated,
    newest: t.chatSkillSortNewest,
  }
  const sortOptions = CLAWHUB_SORT_OPTIONS.map((o) => ({ value: o.value, label: sortLabels[o.value] }))

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {/* 工具行：搜索为主（与已安装 tab 同规格），排序退居右侧紧凑控件 */}
      <div className="flex items-center gap-2 pb-4">
        <div className="relative min-w-0 flex-1">
          <Search size={16} className="pointer-events-none absolute left-3.5 top-1/2 -translate-y-1/2 text-neutral-400" />
          <input
            type="text"
            value={queryInput}
            onChange={(e) => setQueryInput(e.target.value)}
            placeholder={t.chatSkillSearchPlaceholder}
            className="h-10 w-full rounded-md border border-neutral-200 bg-neutral-50 pl-10 pr-4 text-[14px] outline-none placeholder:text-neutral-400 focus:border-neutral-300 text-neutral-900"
            data-tauri-drag-region="false"
          />
        </div>
        {/* 排序：用共享 Select（自绘菜单）而非原生 <select> —— 原生弹层由系统绘制，
            样式无法统一，跟应用里其他所有下拉长得完全不同。
            [&_button]:h-10 是为了跟左边 h-10 的搜索框对齐（Select 默认 30px）。 */}
        <Select
          className="w-[132px] shrink-0 [&_button]:h-10"
          value={sort}
          onChange={(value) => setSort(value as ClawHubSort)}
          disabled={Boolean(query)}
          options={sortOptions}
          title={query ? t.chatSkillSortRelevanceHint : undefined}
        />
      </div>

      {banner && (
        <div className="mb-3 rounded-md border border-red-300/60 bg-red-50 px-3 py-2 text-[12px] text-red-700 dark:border-red-800/60 dark:bg-red-950/40 dark:text-red-300">
          {banner}
        </div>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto">
        {loading ? (
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {Array.from({ length: 6 }, (_, i) => (
              <div key={i} className="flex flex-col rounded-xl border border-neutral-200/80 p-3.5">
                <div className="kv-skeleton h-4 w-2/5 rounded" />
                <div className="kv-skeleton mt-2.5 h-3 w-full rounded" />
                <div className="kv-skeleton mt-1.5 h-3 w-3/4 rounded" />
                <div className="kv-skeleton mt-3 h-7 w-full rounded" />
              </div>
            ))}
          </div>
        ) : items.length === 0 ? (
          <div className="flex h-40 items-center justify-center text-[13px] text-neutral-400">{t.chatSkillStoreNoResults}</div>
        ) : (
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {items.map((card, idx) => {
              const done = isStoreSlugInstalled(ops, card.slug)
              const busy = ops.busyKeys.includes(`store:${card.slug}`)
              return (
                <div
                  key={`${card.slug}-${idx}`}
                  style={{ '--chat-motion-delay': `${Math.min(idx % PAGE_LIMIT, 8) * 24}ms` } as CSSProperties}
                  className="chat-motion-fade-up group flex flex-col rounded-xl border border-neutral-200 bg-neutral-50 p-3.5 shadow-sm transition-[border-color,box-shadow,transform] duration-[var(--kv-dur-fast)] ease-[var(--kv-ease-standard)] hover:-translate-y-0.5 hover:border-neutral-300 hover:shadow-md"
                >
                  <div className="flex items-start justify-between gap-2">
                    <span className="truncate text-[13.5px] font-semibold leading-tight text-neutral-950">{card.displayName}</span>
                    {card.latestVersion && (
                      <span className="shrink-0 rounded-full bg-neutral-100 px-2 py-0.5 text-[10.5px] tabular-nums text-neutral-500 dark:text-neutral-400">
                        v{card.latestVersion}
                      </span>
                    )}
                  </div>
                  <p className="mt-1 line-clamp-2 min-h-[2.4em] text-[12px] leading-[1.45] text-neutral-500 dark:text-neutral-400">
                    {card.summary || t.chatSkillNoSummary}
                  </p>
                  <div className="mt-2.5 flex items-center justify-between gap-2 border-t border-neutral-100 pt-2.5">
                    <div className="flex items-center gap-3 text-[11px] tabular-nums text-neutral-400 dark:text-neutral-500">
                      <span className="inline-flex items-center gap-1"><Download size={11} />{card.downloads.toLocaleString()}</span>
                      <span className="inline-flex items-center gap-1"><Star size={11} />{card.stars.toLocaleString()}</span>
                    </div>
                    <div className="flex items-center gap-1">
                      {card.webUrl && (
                        <span className="opacity-0 transition-opacity duration-[var(--kv-dur-fast)] focus-within:opacity-100 group-hover:opacity-100">
                          <IconButton size="sm" variant="ghost" onClick={() => void api.openExternal(card.webUrl!)} label={t.chatSkillHomepage}>
                            <ExternalLink size={13} />
                          </IconButton>
                        </span>
                      )}
                      {done ? (
                        <span className="chat-motion-pop inline-flex items-center gap-1 rounded-md bg-emerald-500/15 px-2 py-1 text-[12px] font-medium text-emerald-600 dark:text-emerald-400"><Check size={13} />{t.chatSkillInstalled}</span>
                      ) : (
                        <Button
                          size="sm"
                          aria-label={busy ? t.chatSkillInstalling : t.chatSkillInstall}
                          onClick={() => void installStoreSkill(card, t.chatSkillInstallFailed)}
                          disabled={busy}
                        >
                          {busy ? <Loader2 size={12} className="animate-spin" /> : t.chatSkillInstall}
                        </Button>
                      )}
                    </div>
                  </div>
                </div>
              )
            })}
          </div>
        )}

        {cursor && !query && !loading && (
          <div className="pt-3">
            <Button size="sm" variant="ghost" onClick={loadMore} disabled={loadingMore} className="w-full">
              {loadingMore ? <Loader2 size={12} className="animate-spin" /> : t.chatSkillLoadMore}
            </Button>
          </div>
        )}
      </div>
    </div>
  )
}
