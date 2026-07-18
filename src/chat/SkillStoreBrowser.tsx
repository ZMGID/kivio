// ClawHub 技能商店内联浏览（无 modal 外壳，供 SkillCenter「技能商店」tab 用）。
// 排序/搜索/翻页 + 一键安装（下载走后端 chat_skills_install_from_url）。数据层见 ../settings/skillMarket.ts。

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Check, Download, ExternalLink, Loader2, Search } from 'lucide-react'
import { api } from '../api/tauri'
import { Button, IconButton } from '../components/Button'
import { Input } from '../settings/components'
import {
  buildClawHubDownloadUrl,
  CLAWHUB_SORT_OPTIONS,
  listClawHubSkills,
  resolveClawHubSkillOwner,
  searchClawHubSkills,
  type ClawHubSkillCard,
  type ClawHubSort,
} from '../settings/skillMarket'

type Props = {
  lang?: 'zh' | 'en'
  onInstalled: () => void
}

const PAGE_LIMIT = 24

export function SkillStoreBrowser({ lang = 'zh', onInstalled }: Props) {
  const zh = lang === 'zh'
  const [sort, setSort] = useState<ClawHubSort>('downloads')
  const [queryInput, setQueryInput] = useState('')
  const [query, setQuery] = useState('')
  const [items, setItems] = useState<ClawHubSkillCard[]>([])
  const [cursor, setCursor] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const [loadingMore, setLoadingMore] = useState(false)
  const [error, setError] = useState('')
  const [busySlug, setBusySlug] = useState<string | null>(null)
  const [installed, setInstalled] = useState<ReadonlySet<string>>(new Set())

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

  const handleInstall = useCallback(
    async (card: ClawHubSkillCard) => {
      setBusySlug(card.slug)
      setError('')
      try {
        const resolved = await resolveClawHubSkillOwner(card)
        const downloadUrl = resolved.downloadUrl ?? buildClawHubDownloadUrl(resolved.slug, resolved.ownerHandle)
        const result = await api.chatSkillsInstallFromUrl(downloadUrl)
        if (!result.success) throw new Error(result.error || (zh ? '安装失败' : 'Install failed'))
        setInstalled((prev) => new Set(prev).add(card.slug))
        onInstalled()
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err))
      } finally {
        setBusySlug(null)
      }
    },
    [onInstalled, zh],
  )

  const sortOptions = useMemo(
    () => CLAWHUB_SORT_OPTIONS.map((o) => ({ value: o.value, label: zh ? o.labelZh : o.labelEn })),
    [zh],
  )

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex flex-wrap items-center gap-2 pb-3">
        <select value={sort} onChange={(e) => setSort(e.target.value as ClawHubSort)} disabled={Boolean(query)} className="kv-input h-[30px] w-auto">
          {sortOptions.map((o) => (
            <option key={o.value} value={o.value}>{o.label}</option>
          ))}
        </select>
        <div className="relative min-w-[180px] flex-1">
          <Search size={13} className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-[var(--text-muted)]" />
          <Input value={queryInput} onChange={setQueryInput} placeholder={zh ? '搜索技能…' : 'Search skills…'} className="pl-8" />
        </div>
      </div>

      {error && (
        <div className="mb-2 rounded-md border border-red-300/60 bg-red-50 px-3 py-2 text-[12px] text-red-700 dark:border-red-800/60 dark:bg-red-950/40 dark:text-red-300">
          {error}
        </div>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto">
        {loading ? (
          <div className="flex h-40 items-center justify-center text-[var(--text-muted)]"><Loader2 size={18} className="animate-spin" /></div>
        ) : items.length === 0 ? (
          <div className="flex h-40 items-center justify-center text-[13px] text-[var(--text-muted)]">{zh ? '没有匹配的技能' : 'No matching skills'}</div>
        ) : (
          <div className="grid grid-cols-1 gap-2 md:grid-cols-2 xl:grid-cols-3">
            {items.map((card, idx) => {
              const done = installed.has(card.slug)
              return (
                <div key={`${card.slug}-${idx}`} className="flex flex-col rounded-lg border border-[var(--border)] bg-[var(--bg-panel)] p-3">
                  <div className="flex items-start justify-between gap-2">
                    <span className="truncate text-[13px] font-semibold">{card.displayName}</span>
                    {card.latestVersion && <span className="shrink-0 rounded bg-[var(--bg-active)] px-1.5 py-0.5 text-[10px] text-[var(--text-muted)]">v{card.latestVersion}</span>}
                  </div>
                  {card.summary && <p className="mt-1 line-clamp-2 text-[12px] text-[var(--text-muted)]">{card.summary}</p>}
                  <div className="mt-2 flex items-center justify-between gap-2">
                    <div className="flex items-center gap-3 text-[10.5px] tabular-nums text-[var(--text-muted)]">
                      <span className="inline-flex items-center gap-1"><Download size={11} />{card.downloads.toLocaleString()}</span>
                      <span>★ {card.stars.toLocaleString()}</span>
                    </div>
                    <div className="flex items-center gap-1">
                      {card.webUrl && (
                        <IconButton size="sm" variant="ghost" onClick={() => void api.openExternal(card.webUrl!)} label={zh ? '主页' : 'Homepage'}>
                          <ExternalLink size={13} />
                        </IconButton>
                      )}
                      {done ? (
                        <span className="inline-flex items-center gap-1 rounded-md bg-emerald-500/15 px-2 py-1 text-[12px] font-medium text-emerald-600 dark:text-emerald-400"><Check size={13} />{zh ? '已安装' : 'Installed'}</span>
                      ) : (
                        <Button size="sm" onClick={() => void handleInstall(card)} disabled={busySlug === card.slug}>
                          {busySlug === card.slug ? <Loader2 size={12} className="animate-spin" /> : zh ? '安装' : 'Install'}
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
          <div className="pt-2">
            <Button size="sm" variant="ghost" onClick={loadMore} disabled={loadingMore} className="w-full">
              {loadingMore ? <Loader2 size={12} className="animate-spin" /> : zh ? '加载更多' : 'Load more'}
            </Button>
          </div>
        )}
      </div>
    </div>
  )
}
