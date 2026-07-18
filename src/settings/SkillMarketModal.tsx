// 技能市场弹层：两种安装方式。
//  1) ClawHub：浏览/排序/搜索/翻页 clawhub.ai 目录，安装前 resolveOwner 补 ownerHandle → 下载安装。
//  2) URL：粘贴 GitHub 仓库 / 直链 zip URL 直接安装。
// 浏览/消歧走前端（CORS *），下载落盘走 Rust api.chatSkillsInstallFromUrl。数据层见 ./skillMarket.ts。

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Check, Download, ExternalLink, Loader2, Search, X } from 'lucide-react'
import { api } from '../api/tauri'
import { Button, IconButton } from '../components/Button'
import { Input } from './components'
import type { Lang } from './i18n'
import {
  buildClawHubDownloadUrl,
  CLAWHUB_SORT_OPTIONS,
  listClawHubSkills,
  resolveClawHubSkillOwner,
  searchClawHubSkills,
  type ClawHubSkillCard,
  type ClawHubSort,
} from './skillMarket'

type Props = {
  lang: Lang
  onInstalled: () => void
  onClose: () => void
}

const PAGE_LIMIT = 24

export function SkillMarketModal({ lang, onInstalled, onClose }: Props) {
  const zh = lang === 'zh'
  const [mode, setMode] = useState<'clawhub' | 'url'>('clawhub')

  // ClawHub 浏览态
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

  // URL 安装态
  const [urlInput, setUrlInput] = useState('')
  const [urlBusy, setUrlBusy] = useState(false)
  const [urlError, setUrlError] = useState('')
  const [urlDone, setUrlDone] = useState('')

  useEffect(() => {
    const timer = setTimeout(() => setQuery(queryInput.trim()), 400)
    return () => clearTimeout(timer)
  }, [queryInput])

  const reqSeq = useRef(0)
  useEffect(() => {
    if (mode !== 'clawhub') return
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
  }, [mode, sort, query])

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

  const installFromUrl = useCallback(
    async (url: string) => {
      const result = await api.chatSkillsInstallFromUrl(url)
      if (!result.success) {
        throw new Error(result.error || (zh ? '安装失败' : 'Install failed'))
      }
      onInstalled()
    },
    [onInstalled, zh],
  )

  const handleInstallClaw = useCallback(
    async (card: ClawHubSkillCard) => {
      setBusySlug(card.slug)
      setError('')
      try {
        const resolved = await resolveClawHubSkillOwner(card)
        const downloadUrl = resolved.downloadUrl ?? buildClawHubDownloadUrl(resolved.slug, resolved.ownerHandle)
        await installFromUrl(downloadUrl)
        setInstalled((prev) => new Set(prev).add(card.slug))
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err))
      } finally {
        setBusySlug(null)
      }
    },
    [installFromUrl],
  )

  const handleInstallUrl = useCallback(async () => {
    const url = urlInput.trim()
    if (!url) return
    setUrlBusy(true)
    setUrlError('')
    setUrlDone('')
    try {
      await installFromUrl(url)
      setUrlDone(zh ? '已安装' : 'Installed')
      setUrlInput('')
    } catch (err) {
      setUrlError(err instanceof Error ? err.message : String(err))
    } finally {
      setUrlBusy(false)
    }
  }, [installFromUrl, urlInput, zh])

  const sortOptions = useMemo(
    () => CLAWHUB_SORT_OPTIONS.map((o) => ({ value: o.value, label: zh ? o.labelZh : o.labelEn })),
    [zh],
  )

  return (
    <div className="kv-modal-backdrop" onMouseDown={onClose}>
      <div
        className="kv-modal flex flex-col"
        style={{ width: 'min(720px, 92vw)', height: 'min(80vh, 720px)' }}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between gap-3 border-b border-[var(--divider)] px-4 py-3">
          <div className="flex items-center gap-3">
            <div className="text-[14px] font-semibold">{zh ? '技能市场' : 'Skill Market'}</div>
            <div className="inline-flex rounded-lg border border-[var(--border)] p-0.5">
              {(['clawhub', 'url'] as const).map((m) => (
                <button
                  key={m}
                  type="button"
                  onClick={() => setMode(m)}
                  className={`rounded-md px-3 py-1 text-[12.5px] font-medium transition-colors ${
                    mode === m
                      ? 'bg-[var(--bg-active)] text-[var(--text)]'
                      : 'text-[var(--text-muted)] hover:text-[var(--text)]'
                  }`}
                >
                  {m === 'clawhub' ? 'ClawHub' : 'URL'}
                </button>
              ))}
            </div>
          </div>
          <IconButton size="sm" variant="ghost" onClick={onClose} label={zh ? '关闭' : 'Close'}>
            <X size={15} />
          </IconButton>
        </div>

        {mode === 'clawhub' ? (
          <>
            <div className="flex flex-wrap items-center gap-2 px-4 py-3">
              <select
                value={sort}
                onChange={(e) => setSort(e.target.value as ClawHubSort)}
                disabled={Boolean(query)}
                className="kv-input h-[30px] w-auto"
              >
                {sortOptions.map((o) => (
                  <option key={o.value} value={o.value}>
                    {o.label}
                  </option>
                ))}
              </select>
              <div className="relative min-w-[180px] flex-1">
                <Search
                  size={13}
                  className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-[var(--text-muted)]"
                />
                <Input
                  value={queryInput}
                  onChange={setQueryInput}
                  placeholder={zh ? '搜索技能…' : 'Search skills…'}
                  className="pl-8"
                />
              </div>
            </div>

            {error && (
              <div className="mx-4 mb-2 rounded-md border border-red-300/60 bg-red-50 px-3 py-2 text-[12px] text-red-700 dark:border-red-800/60 dark:bg-red-950/40 dark:text-red-300">
                {error}
              </div>
            )}

            <div className="min-h-0 flex-1 overflow-y-auto px-4 pb-4">
              {loading ? (
                <div className="flex h-40 items-center justify-center text-[var(--text-muted)]">
                  <Loader2 size={18} className="animate-spin" />
                </div>
              ) : items.length === 0 ? (
                <div className="flex h-40 items-center justify-center text-[13px] text-[var(--text-muted)]">
                  {zh ? '没有匹配的技能' : 'No matching skills'}
                </div>
              ) : (
                <div className="space-y-2">
                  {items.map((card, idx) => {
                    const done = installed.has(card.slug)
                    return (
                      <div
                        key={`${card.slug}-${idx}`}
                        className="rounded-lg border border-[var(--border)] bg-[var(--bg-panel)] p-3"
                      >
                        <div className="flex items-start justify-between gap-3">
                          <div className="min-w-0">
                            <div className="flex items-center gap-2">
                              <span className="truncate text-[13px] font-semibold">{card.displayName}</span>
                              {card.latestVersion && (
                                <span className="shrink-0 rounded bg-[var(--bg-active)] px-1.5 py-0.5 text-[10px] text-[var(--text-muted)]">
                                  v{card.latestVersion}
                                </span>
                              )}
                            </div>
                            {card.summary && (
                              <p className="mt-1 line-clamp-2 text-[12px] text-[var(--text-muted)]">
                                {card.summary}
                              </p>
                            )}
                            <div className="mt-1 flex items-center gap-3 text-[10.5px] tabular-nums text-[var(--text-muted)]">
                              <span className="inline-flex items-center gap-1">
                                <Download size={11} />
                                {card.downloads.toLocaleString()}
                              </span>
                              <span>★ {card.stars.toLocaleString()}</span>
                              {card.ownerHandle && <span className="truncate">@{card.ownerHandle}</span>}
                            </div>
                          </div>
                          <div className="flex shrink-0 items-center gap-1">
                            {card.webUrl && (
                              <IconButton
                                size="sm"
                                variant="ghost"
                                onClick={() => void api.openExternal(card.webUrl!)}
                                label={zh ? '主页' : 'Homepage'}
                              >
                                <ExternalLink size={13} />
                              </IconButton>
                            )}
                            {done ? (
                              <span className="inline-flex items-center gap-1 rounded-md bg-emerald-500/15 px-2 py-1 text-[12px] font-medium text-emerald-600 dark:text-emerald-400">
                                <Check size={13} />
                                {zh ? '已安装' : 'Installed'}
                              </span>
                            ) : (
                              <Button
                                size="sm"
                                onClick={() => void handleInstallClaw(card)}
                                disabled={busySlug === card.slug}
                              >
                                {busySlug === card.slug ? (
                                  <Loader2 size={12} className="animate-spin" />
                                ) : zh ? (
                                  '安装'
                                ) : (
                                  'Install'
                                )}
                              </Button>
                            )}
                          </div>
                        </div>
                      </div>
                    )
                  })}

                  {cursor && !query && (
                    <div className="pt-1">
                      <Button size="sm" variant="ghost" onClick={loadMore} disabled={loadingMore} className="w-full">
                        {loadingMore ? <Loader2 size={12} className="animate-spin" /> : zh ? '加载更多' : 'Load more'}
                      </Button>
                    </div>
                  )}
                </div>
              )}
            </div>
          </>
        ) : (
          <div className="flex-1 overflow-y-auto px-5 py-5">
            <p className="mb-2 text-[12.5px] text-[var(--text-muted)]">
              {zh
                ? '粘贴 GitHub 仓库地址或直链 .zip（须含 SKILL.md）。仓库内多个技能只安装第一个。'
                : 'Paste a GitHub repo URL or a direct .zip link (must contain SKILL.md). Only the first skill in a repo is installed.'}
            </p>
            <Input
              value={urlInput}
              onChange={setUrlInput}
              placeholder="https://github.com/owner/repo"
              mono
            />
            {urlError && (
              <div className="mt-2 rounded-md border border-red-300/60 bg-red-50 px-3 py-2 text-[12px] text-red-700 dark:border-red-800/60 dark:bg-red-950/40 dark:text-red-300">
                {urlError}
              </div>
            )}
            {urlDone && (
              <div className="mt-2 inline-flex items-center gap-1 text-[12px] text-emerald-600 dark:text-emerald-400">
                <Check size={13} />
                {urlDone}
              </div>
            )}
            <div className="mt-3">
              <Button size="sm" onClick={() => void handleInstallUrl()} disabled={urlBusy || !urlInput.trim()}>
                {urlBusy ? <Loader2 size={12} className="animate-spin" /> : zh ? '安装' : 'Install'}
              </Button>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
