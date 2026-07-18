// MCP 注册表内联浏览（无 modal 外壳，供 McpCenter「市场」tab 用）。浏览/搜索/翻页三源，
// 一键把服务器交给 onInstall。needs_config 的条目在卡片下方内联展开填参，不弹二级窗口。
// 数据层见 ../settings/mcpRegistry.ts。

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Check, ExternalLink, Loader2, Search } from 'lucide-react'
import type { ChatMcpServer } from '../api/tauri'
import { api } from '../api/tauri'
import { Button, IconButton } from '../components/Button'
import { Input } from '../settings/components'
import {
  applyMcpRegistryInstallConfig,
  MCP_REGISTRY_SOURCE_OPTIONS,
  mcpRegistryConfigInputKey,
  resolveMcpRegistryInstallDraft,
  searchMcpRegistry,
  withUniqueMcpServerId,
  type McpRegistryCard,
  type McpRegistryInstallDraft,
  type McpRegistrySource,
} from '../settings/mcpRegistry'

type Props = {
  lang?: 'zh' | 'en'
  existingServers: ChatMcpServer[]
  onInstall: (server: ChatMcpServer) => void
}

type ConfigState = { card: McpRegistryCard; draft: McpRegistryInstallDraft; values: Record<string, string> }

const PAGE_LIMIT = 24

export function McpRegistryBrowser({ lang = 'zh', existingServers, onInstall }: Props) {
  const zh = lang === 'zh'
  const [source, setSource] = useState<McpRegistrySource>('official')
  const [queryInput, setQueryInput] = useState('')
  const [query, setQuery] = useState('')
  const [items, setItems] = useState<McpRegistryCard[]>([])
  const [cursor, setCursor] = useState<string | undefined>(undefined)
  const [loading, setLoading] = useState(false)
  const [loadingMore, setLoadingMore] = useState(false)
  const [error, setError] = useState('')
  const [config, setConfig] = useState<ConfigState | null>(null)
  const [busyId, setBusyId] = useState<string | null>(null)
  const [installedIds, setInstalledIds] = useState<ReadonlySet<string>>(new Set())

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
    setCursor(undefined)
    setConfig(null)
    searchMcpRegistry({ source, query: query || undefined, limit: PAGE_LIMIT })
      .then((result) => {
        if (reqId !== reqSeq.current) return
        setItems(result.items)
        setCursor(result.nextCursor)
      })
      .catch((err) => {
        if (reqId !== reqSeq.current) return
        setError(err instanceof Error ? err.message : String(err))
      })
      .finally(() => {
        if (reqId === reqSeq.current) setLoading(false)
      })
  }, [source, query])

  const loadMore = useCallback(() => {
    if (!cursor || loadingMore) return
    const reqId = reqSeq.current
    setLoadingMore(true)
    searchMcpRegistry({ source, query: query || undefined, cursor, limit: PAGE_LIMIT })
      .then((result) => {
        if (reqId !== reqSeq.current) return
        setItems((prev) => [...prev, ...result.items])
        setCursor(result.nextCursor)
      })
      .catch((err) => {
        if (reqId !== reqSeq.current) return
        setError(err instanceof Error ? err.message : String(err))
      })
      .finally(() => {
        if (reqId === reqSeq.current) setLoadingMore(false)
      })
  }, [cursor, loadingMore, source, query])

  const commitServer = useCallback(
    (draft: McpRegistryInstallDraft, cardId: string) => {
      const unique = withUniqueMcpServerId(draft, existingServers)
      onInstall(unique.server)
      setInstalledIds((prev) => new Set(prev).add(cardId))
    },
    [existingServers, onInstall],
  )

  const handleInstall = useCallback(
    async (card: McpRegistryCard) => {
      setBusyId(card.id)
      setError('')
      try {
        const resolved = await resolveMcpRegistryInstallDraft(card)
        const draft = resolved.installDraft ?? resolved.manualDraft
        if (!draft) {
          setError(zh ? '此条目无法自动安装，请打开主页手动配置。' : 'Cannot install automatically; open homepage.')
          return
        }
        if (draft.status === 'needs_config') {
          const values: Record<string, string> = {}
          for (const input of draft.requiredConfig) values[mcpRegistryConfigInputKey(input)] = ''
          setConfig({ card: resolved, draft, values })
          return
        }
        commitServer(draft, card.id)
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err))
      } finally {
        setBusyId(null)
      }
    },
    [commitServer, zh],
  )

  const configReady = useMemo(() => {
    if (!config) return false
    return config.draft.requiredConfig
      .filter((input) => input.required)
      .every((input) => (config.values[mcpRegistryConfigInputKey(input)] ?? '').trim())
  }, [config])

  const submitConfig = useCallback(() => {
    if (!config) return
    const ready = applyMcpRegistryInstallConfig(config.draft, config.values)
    commitServer(ready, config.card.id)
    setConfig(null)
  }, [config, commitServer])

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex flex-wrap items-center gap-2 pb-3">
        <div className="inline-flex rounded-lg border border-[var(--border)] p-0.5">
          {MCP_REGISTRY_SOURCE_OPTIONS.map((opt) => (
            <button
              key={opt.value}
              type="button"
              onClick={() => setSource(opt.value)}
              className={`rounded-md px-3 py-1 text-[12.5px] font-medium transition-colors ${
                source === opt.value
                  ? 'bg-[var(--bg-active)] text-[var(--text)]'
                  : 'text-[var(--text-muted)] hover:text-[var(--text)]'
              }`}
            >
              {opt.label}
            </button>
          ))}
        </div>
        <div className="relative min-w-[180px] flex-1">
          <Search size={13} className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-[var(--text-muted)]" />
          <Input value={queryInput} onChange={setQueryInput} placeholder={zh ? '搜索 MCP 服务器…' : 'Search MCP servers…'} className="pl-8" />
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
          <div className="flex h-40 items-center justify-center text-[13px] text-[var(--text-muted)]">{zh ? '没有匹配的服务器' : 'No matching servers'}</div>
        ) : (
          <div className="grid grid-cols-1 gap-2 md:grid-cols-2">
            {items.map((card) => {
              const installed = installedIds.has(card.id)
              const configuring = config?.card.id === card.id
              return (
                <div key={card.id} className="rounded-lg border border-[var(--border)] bg-[var(--bg-panel)] p-3">
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="truncate text-[13px] font-semibold">{card.displayName}</span>
                        {card.verified && <span className="shrink-0 rounded bg-emerald-500/15 px-1.5 py-0.5 text-[10px] font-medium text-emerald-600 dark:text-emerald-400">{zh ? '已验证' : 'verified'}</span>}
                        {card.transportHints.map((hint) => (
                          <span key={hint} className="shrink-0 rounded bg-[var(--bg-active)] px-1.5 py-0.5 text-[10px] text-[var(--text-muted)]">{hint}</span>
                        ))}
                      </div>
                      {card.description && <p className="mt-1 line-clamp-2 text-[12px] text-[var(--text-muted)]">{card.description}</p>}
                      <div className="mt-1 truncate font-mono text-[10.5px] text-[var(--text-muted)]">{card.sourceId}</div>
                    </div>
                    <div className="flex shrink-0 items-center gap-1">
                      {card.detailUrl && (
                        <IconButton size="sm" variant="ghost" onClick={() => void api.openExternal(card.detailUrl!)} label={zh ? '主页' : 'Homepage'}>
                          <ExternalLink size={13} />
                        </IconButton>
                      )}
                      {installed ? (
                        <span className="inline-flex items-center gap-1 rounded-md bg-emerald-500/15 px-2 py-1 text-[12px] font-medium text-emerald-600 dark:text-emerald-400"><Check size={13} />{zh ? '已添加' : 'Added'}</span>
                      ) : (
                        <Button size="sm" onClick={() => void handleInstall(card)} disabled={busyId === card.id}>
                          {busyId === card.id ? <Loader2 size={12} className="animate-spin" /> : zh ? '添加' : 'Add'}
                        </Button>
                      )}
                    </div>
                  </div>

                  {configuring && config && (
                    <div className="mt-3 space-y-2 border-t border-[var(--divider)] pt-3">
                      {config.draft.requiredConfig.map((input) => {
                        const key = mcpRegistryConfigInputKey(input)
                        return (
                          <div key={key}>
                            <label className="mb-1 block text-[11.5px] font-medium">
                              {input.label ?? input.name}
                              {input.required && <span className="ml-1 text-red-500">*</span>}
                              <span className="ml-1.5 text-[10px] font-normal text-[var(--text-muted)]">{input.target}</span>
                            </label>
                            <Input
                              value={config.values[key] ?? ''}
                              onChange={(v) => setConfig((prev) => (prev ? { ...prev, values: { ...prev.values, [key]: v } } : prev))}
                              type={input.secret ? 'password' : 'text'}
                              placeholder={input.secret ? '••••••' : ''}
                              mono
                            />
                          </div>
                        )
                      })}
                      <div className="flex justify-end gap-2 pt-1">
                        <Button size="sm" variant="ghost" onClick={() => setConfig(null)}>{zh ? '取消' : 'Cancel'}</Button>
                        <Button size="sm" onClick={submitConfig} disabled={!configReady}>{zh ? '添加' : 'Add'}</Button>
                      </div>
                    </div>
                  )}
                </div>
              )
            })}
          </div>
        )}

        {cursor && !loading && (
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
