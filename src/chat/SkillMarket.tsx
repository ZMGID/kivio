// 技能市场：拉取远程 JSON 索引、按分类/搜索浏览、安装前确认后下载安装。
// 货源索引地址内联可编辑（存回 settings.chatTools.skillMarket.indexUrl），不写死来源。
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Download, RefreshCw, Search, Package, ExternalLink, CircleAlert } from 'lucide-react'
import { api, type MarketSkill, type MarketInstalledInfo } from '../api/tauri'
import { Button } from '../components/Button'
import { computeSkillState, type SkillMarketState } from './skillMarketState'

const ALL = '__all__'

function hostOf(url: string): string {
  try {
    return new URL(url).host
  } catch {
    return url
  }
}

export function SkillMarket({
  indexUrl,
  onChangeIndexUrl,
  onInstalled,
}: {
  indexUrl: string
  onChangeIndexUrl: (url: string) => void
  onInstalled: () => void
}) {
  const [urlDraft, setUrlDraft] = useState(indexUrl)
  const [skills, setSkills] = useState<MarketSkill[]>([])
  const [installed, setInstalled] = useState<MarketInstalledInfo[]>([])
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')
  const [query, setQuery] = useState('')
  const [category, setCategory] = useState<string>(ALL)
  const [confirming, setConfirming] = useState<MarketSkill | null>(null)
  const [installingId, setInstallingId] = useState<string | null>(null)

  useEffect(() => setUrlDraft(indexUrl), [indexUrl])

  const fetchIndex = useCallback(async (url: string) => {
    if (!url.trim()) {
      setSkills([])
      setInstalled([])
      setError('')
      return
    }
    setLoading(true)
    setError('')
    try {
      const res = await api.chatSkillsMarketFetch(url.trim())
      setInstalled(res.installed ?? [])
      if (res.success) {
        setSkills(res.skills ?? [])
      } else {
        setSkills([])
        setError(res.error || '拉取失败')
      }
    } catch (err) {
      setSkills([])
      setError(typeof err === 'string' ? err : (err as Error).message || '拉取失败')
    } finally {
      setLoading(false)
    }
  }, [])

  const fetchRef = useRef(fetchIndex)
  fetchRef.current = fetchIndex
  useEffect(() => {
    void fetchRef.current(indexUrl)
  }, [indexUrl])

  const categories = useMemo(() => {
    const set = new Set<string>()
    skills.forEach((s) => s.category && set.add(s.category))
    return Array.from(set).sort()
  }, [skills])

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase()
    return skills.filter((s) => {
      if (category !== ALL && s.category !== category) return false
      if (!q) return true
      return (
        s.name.toLowerCase().includes(q) ||
        s.description.toLowerCase().includes(q) ||
        s.tags.some((t) => t.toLowerCase().includes(q))
      )
    })
  }, [skills, query, category])

  const commitUrl = () => {
    const next = urlDraft.trim()
    if (next !== indexUrl) onChangeIndexUrl(next)
    else void fetchIndex(next)
  }

  const doInstall = useCallback(async (skill: MarketSkill) => {
    setConfirming(null)
    setInstallingId(skill.id)
    setError('')
    try {
      const res = await api.chatSkillsMarketInstall(skill, indexUrl.trim())
      if (res.success) {
        onInstalled()
        await fetchIndex(indexUrl) // 刷新三态
      } else {
        setError(res.error || '安装失败')
      }
    } catch (err) {
      setError(typeof err === 'string' ? err : (err as Error).message || '安装失败')
    } finally {
      setInstallingId(null)
    }
  }, [fetchIndex, indexUrl, onInstalled])

  const btnLabel: Record<SkillMarketState, string> = {
    install: '安装',
    installed: '已安装',
    update: '更新',
  }

  return (
    <div className="mt-6">
      {/* 索引地址（内联可编辑） */}
      <div className="flex items-center gap-2">
        <div className="relative min-w-0 flex-1">
          <Package size={16} className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-neutral-400" />
          <input
            value={urlDraft}
            onChange={(e) => setUrlDraft(e.target.value)}
            onBlur={commitUrl}
            onKeyDown={(e) => {
              if (e.key === 'Enter') e.currentTarget.blur()
            }}
            placeholder="技能市场索引地址（JSON URL）"
            className="h-11 w-full rounded-xl border border-neutral-200 bg-white pl-10 pr-4 text-[13px] outline-none placeholder:text-neutral-400 focus:border-neutral-300 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100"
            data-tauri-drag-region="false"
          />
        </div>
        <Button variant="ghost" size="sm" onClick={() => void fetchIndex(indexUrl)} disabled={loading} data-tauri-drag-region="false">
          <RefreshCw size={14} className={loading ? 'animate-spin' : ''} />
          刷新
        </Button>
      </div>

      {/* 搜索 + 分类 */}
      {skills.length > 0 && (
        <div className="mt-3 flex items-center gap-2">
          <div className="relative min-w-0 flex-1">
            <Search size={15} className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-neutral-400" />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="搜索市场技能..."
              className="h-9 w-full rounded-lg border border-neutral-200 bg-white pl-9 pr-3 text-[13px] outline-none placeholder:text-neutral-400 focus:border-neutral-300 dark:border-neutral-700 dark:bg-neutral-900 dark:text-neutral-100"
              data-tauri-drag-region="false"
            />
          </div>
          {categories.length > 0 && (
            <div className="flex shrink-0 items-center gap-1 overflow-x-auto">
              {[ALL, ...categories].map((cat) => (
                <button
                  key={cat}
                  type="button"
                  onClick={() => setCategory(cat)}
                  className={`h-9 shrink-0 rounded-lg px-3 text-[12px] font-medium transition-colors ${
                    category === cat
                      ? 'bg-neutral-900 text-white dark:bg-neutral-100 dark:text-neutral-900'
                      : 'text-neutral-600 hover:bg-neutral-100 dark:text-neutral-300 dark:hover:bg-neutral-800'
                  }`}
                  data-tauri-drag-region="false"
                >
                  {cat === ALL ? '全部' : cat}
                </button>
              ))}
            </div>
          )}
        </div>
      )}

      {error && (
        <div className="mt-4 rounded-lg border border-red-200 bg-red-50 px-3 py-2 text-[12px] text-red-700 dark:border-red-900/60 dark:bg-red-950/30 dark:text-red-300">
          {error}
        </div>
      )}

      {/* 卡片网格 / 空态 */}
      <div className="mt-5">
        {!indexUrl.trim() ? (
          <div className="grid min-h-[220px] place-items-center rounded-2xl border border-dashed border-neutral-200 px-6 text-center text-[13px] text-neutral-400 dark:border-neutral-800">
            填入技能市场索引地址后即可浏览与安装。
          </div>
        ) : loading && skills.length === 0 ? (
          <div className="grid min-h-[220px] place-items-center text-[13px] text-neutral-400">正在加载市场...</div>
        ) : filtered.length === 0 ? (
          <div className="grid min-h-[220px] place-items-center rounded-2xl border border-dashed border-neutral-200 px-6 text-center text-[13px] text-neutral-400 dark:border-neutral-800">
            {skills.length === 0 ? '该索引暂无技能。' : '没有匹配的技能。'}
          </div>
        ) : (
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            {filtered.map((skill) => {
              const state = computeSkillState(skill, installed)
              const busy = installingId === skill.id
              return (
                <div
                  key={skill.id}
                  className="flex gap-3 rounded-2xl border border-neutral-200 bg-white p-3.5 dark:border-neutral-800 dark:bg-neutral-900/40"
                >
                  <div className="grid size-11 shrink-0 place-items-center overflow-hidden rounded-xl bg-neutral-100 dark:bg-neutral-800">
                    {skill.iconUrl ? (
                      <img src={skill.iconUrl} alt="" className="size-full object-cover" />
                    ) : (
                      <Package size={20} className="text-neutral-400" />
                    )}
                  </div>
                  <div className="flex min-w-0 flex-1 flex-col">
                    <div className="flex min-w-0 items-center gap-1.5">
                      <span className="min-w-0 truncate text-[14px] font-semibold text-neutral-900 dark:text-neutral-100">
                        {skill.name}
                      </span>
                      {skill.version && (
                        <span className="shrink-0 text-[11px] text-neutral-400">v{skill.version}</span>
                      )}
                    </div>
                    <p className="mt-0.5 line-clamp-2 text-[12px] leading-relaxed text-neutral-500 dark:text-neutral-400">
                      {skill.description}
                    </p>
                    <div className="mt-2 flex items-center gap-1.5">
                      {skill.category && (
                        <span className="rounded-full bg-neutral-100 px-2 py-0.5 text-[10.5px] text-neutral-500 dark:bg-neutral-800 dark:text-neutral-400">
                          {skill.category}
                        </span>
                      )}
                      {skill.author && (
                        <span className="truncate text-[11px] text-neutral-400">{skill.author}</span>
                      )}
                      <div className="ml-auto shrink-0">
                        <Button
                          size="sm"
                          variant={state === 'installed' ? 'ghost' : 'primary'}
                          disabled={state === 'installed' || busy}
                          onClick={() => setConfirming(skill)}
                          data-tauri-drag-region="false"
                        >
                          {busy ? <RefreshCw size={12} className="animate-spin" /> : state !== 'installed' && <Download size={12} />}
                          {btnLabel[state]}
                        </Button>
                      </div>
                    </div>
                  </div>
                </div>
              )
            })}
          </div>
        )}
      </div>

      {/* 安装前确认弹窗 */}
      {confirming && (
        <div
          className="chat-motion-fade fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-6"
          data-tauri-drag-region="false"
          onClick={() => setConfirming(null)}
        >
          <div
            role="dialog"
            aria-modal="true"
            className="chat-motion-modal-in flex w-full max-w-[440px] flex-col gap-3 rounded-2xl border border-neutral-200 bg-white p-5 shadow-2xl dark:border-neutral-700 dark:bg-neutral-900"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-start gap-2.5">
              <CircleAlert size={18} className="mt-0.5 shrink-0 text-amber-500" />
              <div className="min-w-0">
                <h3 className="text-[15px] font-semibold text-neutral-900 dark:text-neutral-100">
                  安装「{confirming.name}」？
                </h3>
                <p className="mt-1.5 text-[12.5px] leading-relaxed text-neutral-500 dark:text-neutral-400">
                  该技能含可运行脚本，被激活时可能在本机执行代码。请仅从可信来源安装。
                </p>
                <div className="mt-2 flex items-center gap-1 text-[11.5px] text-neutral-400">
                  <ExternalLink size={12} className="shrink-0" />
                  <span className="truncate">来源：{hostOf(confirming.downloadUrl)}</span>
                </div>
              </div>
            </div>
            <div className="mt-1 flex justify-end gap-2">
              <Button variant="ghost" size="sm" onClick={() => setConfirming(null)} data-tauri-drag-region="false">
                取消
              </Button>
              <Button size="sm" onClick={() => void doInstall(confirming)} data-tauri-drag-region="false">
                确认安装
              </Button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
