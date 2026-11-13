// 工作区面板的分支选择器，参照 T3 Code 的 BranchPicker / BranchPickerRefItem：
// 搜索框 + 本地与远程分支（当前、默认优先，其余按最近提交）+ 右侧弱化标签；
// 搜索词不是已有分支时提供「创建分支」。远程分支检出为同名本地跟踪分支（后端负责）。
import { useEffect, useMemo, useState } from 'react'
import { Plus, Search } from 'lucide-react'
import { i18n, type Lang } from '../../components/i18n'
import type { GitBranchesResult } from '../../api/dockContracts'
import { copyToClipboard } from '../../utils/clipboard'
import { orderBranchRefs, sanitizeBranchName } from './workspaceBranches'

export type BranchPick = { kind: 'switch'; name: string } | { kind: 'create'; name: string }

const ITEM = 'kv-menu-row w-full text-neutral-800 transition-colors hover:bg-neutral-100 focus-visible:bg-neutral-100 outline-none disabled:opacity-50'

export function WorkspaceBranchPicker({ data, lang, disabled, onPick }: {
  data: GitBranchesResult | null
  lang: Lang
  disabled: boolean
  onPick: (pick: BranchPick) => void
}) {
  const t = i18n[lang]
  const [query, setQuery] = useState('')
  // 参照 T3：右键分支复制分支名；结果直接标在该行，不弹通知。
  const [copied, setCopied] = useState<string | null>(null)
  useEffect(() => {
    if (!copied) return
    const timer = window.setTimeout(() => setCopied(null), 1500)
    return () => window.clearTimeout(timer)
  }, [copied])
  const copyName = (name: string) => {
    void copyToClipboard(name).then((ok) => { if (ok) setCopied(name) })
  }
  const refs = useMemo(() => (data ? orderBranchRefs(data) : []), [data])
  const needle = query.trim().toLowerCase()
  const filtered = needle ? refs.filter((ref) => ref.name.toLowerCase().includes(needle)) : refs
  const newName = sanitizeBranchName(query)
  const canCreate = Boolean(newName) && !refs.some((ref) => ref.name === newName)
  const badgeLabel = { current: t.workspaceRefCurrent, default: t.workspaceRefDefault, remote: t.workspaceRefRemote }
  const first = filtered.find((ref) => !ref.current)

  return (
    <div className="flex flex-col">
      <label className="sticky -top-1 z-10 -mx-1 -mt-1 mb-1 flex h-9 items-center gap-2 border-b border-neutral-200/70 bg-[var(--theme-surface)] px-3 text-neutral-400 dark:border-white/10">
        <Search className="size-3.5 shrink-0" />
        <input
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={(event) => {
            if (event.key !== 'Enter' || event.nativeEvent.isComposing) return
            event.preventDefault()
            if (first && !disabled) onPick({ kind: 'switch', name: first.name })
            else if (canCreate && !disabled) onPick({ kind: 'create', name: newName })
          }}
          placeholder={t.workspaceSearchBranches}
          aria-label={t.workspaceSearchBranches}
          className="min-w-0 flex-1 border-0 bg-transparent text-[12px] text-neutral-800 outline-none placeholder:text-neutral-400 dark:text-neutral-100"
        />
      </label>
      {data === null ? (
        <div role="status" className="px-2 py-2 text-[12px] text-neutral-400">{t.dockLoading}</div>
      ) : (
        <div className="flex flex-col">
          {filtered.map((ref) => (
            <button
              key={ref.name}
              type="button"
              role="menuitemradio"
              aria-checked={ref.current}
              disabled={ref.current || disabled}
              title={`${ref.name}\n${t.workspaceCopyBranchHint}`}
              className={`${ITEM} ${ref.current ? 'font-medium disabled:opacity-100' : ''}`}
              onClick={() => onPick({ kind: 'switch', name: ref.name })}
              onContextMenu={(event) => {
                event.preventDefault()
                event.stopPropagation()
                copyName(ref.name)
              }}
            >
              <span className="min-w-0 flex-1 truncate">{ref.name}</span>
              {copied === ref.name ? (
                <span role="status" className="chat-motion-pop shrink-0 text-[11px] text-emerald-600 dark:text-emerald-400">{t.workspaceBranchCopied}</span>
              ) : ref.badge && <span className="shrink-0 text-[11px] text-neutral-400/80 dark:text-neutral-500">{badgeLabel[ref.badge]}</span>}
            </button>
          ))}
          {canCreate && (
            <button type="button" role="menuitem" disabled={disabled} className={ITEM} onClick={() => onPick({ kind: 'create', name: newName })}>
              <Plus size={13} strokeWidth={1.8} className="shrink-0 text-neutral-500" />
              <span className="min-w-0 flex-1 truncate">{t.workspaceCreateBranch.replace('{name}', newName)}</span>
            </button>
          )}
          {filtered.length === 0 && !canCreate && (
            <div className="px-2 py-2 text-[12px] text-neutral-400">{t.workspaceNoBranches}</div>
          )}
        </div>
      )}
    </div>
  )
}
