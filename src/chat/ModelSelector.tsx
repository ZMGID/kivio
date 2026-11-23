import { providerModelLabel } from '../data/modelCatalog'
import { memo, useCallback, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode, type RefObject } from 'react'
import { createPortal } from 'react-dom'
import { Check, ChevronDown, Search, Star } from 'lucide-react'
import { useT } from '../components/i18n'
import { ModelIcon } from '../components/ModelIcon'
import {
  SEARCH_THRESHOLD,
  favKey,
  modelRowButtonClass,
  modelRowClass,
  useAutoFocusSearch,
  useBuiltinModelCatalog,
  useRevealCurrentRow,
  type BuiltinModelCatalog,
} from './builtinModelCatalog'
import { usePopoverMaxHeight } from './usePopoverMaxHeight'
import { chatTitlebarPillButtonClass } from './platform'
import { usePopoverMenu } from './usePopoverMenu'

interface ModelSelectorProps {
  currentProviderId: string
  currentModel: string
  onModelChange: (providerId: string, model: string) => void
  /** Escape clipping when hosted inside a scrollable tool panel. */
  portal?: boolean
}

/** 行尾的选中对勾：固定占位，名字不随选中跳动。 */
export function ModelRowCheck({ selected }: { selected: boolean }) {
  return (
    <span className="grid size-6 shrink-0 place-items-center" aria-hidden="true">
      {selected && <Check size={14} strokeWidth={2.2} className="text-[var(--accent)]" />}
    </span>
  )
}

/** 模型多时出现的搜索框。贴在 4px 内边距的弹层顶部，分隔线通栏。 */
export function ModelSearchField({
  value,
  onChange,
  onSubmit,
  inputRef,
  placeholder,
  trailing,
}: {
  value: string
  onChange: (value: string) => void
  onSubmit: () => void
  inputRef?: RefObject<HTMLInputElement>
  placeholder: string
  /** 搜索框右侧的附加信息（如「已绑定」标记）。 */
  trailing?: ReactNode
}) {
  return (
    <div className="-mx-1 -mt-1 mb-1 flex h-10 shrink-0 items-center gap-2 border-b border-[var(--theme-surface-border)] px-4 text-neutral-400 dark:text-neutral-500">
      <Search size={14} strokeWidth={1.8} className="shrink-0" />
      <input
        ref={inputRef}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        onKeyDown={(event) => {
          // IME 合成中（中/日/韩输入法按回车上屏）跳过。WebKit（Tauri macOS）里上屏那次回车的
          // isComposing 已是 false，只剩 keyCode 229 可认；漏掉它就会把上屏当成「选中首项」并关掉弹层。
          if (event.nativeEvent.isComposing || event.keyCode === 229) return
          // 有搜索词时 Esc 先清空，不直接关掉整个弹层。
          if (event.key === 'Escape' && value) {
            event.preventDefault()
            event.stopPropagation()
            onChange('')
            return
          }
          if (event.key !== 'Enter') return
          event.preventDefault()
          onSubmit()
        }}
        placeholder={placeholder}
        aria-label={placeholder}
        className="min-w-0 flex-1 border-0 bg-transparent text-[13px] text-neutral-800 outline-none placeholder:text-neutral-400 dark:placeholder:text-neutral-500"
        data-tauri-drag-region="false"
      />
      {trailing}
    </div>
  )
}

/**
 * 内置模型列表：收藏置顶、按服务商分组；模型超过阈值时顶部出现搜索框。
 * 只负责列表本身，弹层外壳与定位归调用方。
 */
export function BuiltinModelList({
  catalog,
  currentProviderId,
  currentModel,
  providerLocked = false,
  onSelect,
  autoFocusSearch = false,
  alwaysShowSearch = false,
  searchPlaceholder,
  searchTrailing,
}: {
  catalog: BuiltinModelCatalog
  currentProviderId: string
  currentModel: string
  providerLocked?: boolean
  onSelect: (providerId: string, model: string) => void
  autoFocusSearch?: boolean
  /** 合一选择器里搜索框兼作右栏标题，始终显示。 */
  alwaysShowSearch?: boolean
  searchPlaceholder?: string
  searchTrailing?: ReactNode
}) {
  const t = useT()
  const [query, setQuery] = useState('')
  const searchRef = useRef<HTMLInputElement>(null)
  const listRef = useRef<HTMLDivElement>(null)
  const { visibleProviders, favoriteEntries, favorites, toggleFavorite } = catalog
  useRevealCurrentRow(listRef)

  // 按模型名或服务商名过滤，搜索期间不重复显示收藏组。
  const normalizedQuery = query.trim().toLowerCase()
  const totalModels = visibleProviders.reduce((sum, entry) => sum + entry.models.length, 0)
  const showSearch = alwaysShowSearch || totalModels > SEARCH_THRESHOLD
  useAutoFocusSearch(autoFocusSearch && showSearch, searchRef)
  const sections = useMemo(() => {
    const matches = (providerName: string, model: string) => !normalizedQuery
      || model.toLowerCase().includes(normalizedQuery)
      || providerName.toLowerCase().includes(normalizedQuery)
    const result: { key: string; title: string; favorite: boolean; rows: { providerId: string; model: string }[] }[] = []
    if (!normalizedQuery && favoriteEntries.length > 0) {
      result.push({ key: 'fav', title: t.chatFavorites, favorite: true, rows: favoriteEntries })
    }
    for (const { provider, models } of visibleProviders) {
      const rows = models.filter(model => matches(provider.name, model) || providerModelLabel(model, provider).toLowerCase().includes(normalizedQuery)).map(model => ({ providerId: provider.id, model }))
      if (rows.length > 0) result.push({ key: provider.id, title: provider.name, favorite: false, rows })
    }
    return result
  }, [favoriteEntries, normalizedQuery, t.chatFavorites, visibleProviders])

  const firstAvailable = sections.flatMap(section => section.rows).find(row => !providerLocked || row.providerId === currentProviderId)
  const enterTarget = normalizedQuery ? firstAvailable : undefined

  const renderModelRow = (providerId: string, model: string, inFavorites: boolean) => {
    const selected = currentProviderId === providerId && currentModel === model
    const isFav = favorites.includes(favKey(providerId, model))
    const isEnterTarget = enterTarget?.providerId === providerId && enterTarget.model === model
    // 收藏组里每行都是收藏，常亮的星标是重复信息，只在悬停时出现（用来取消收藏）。
    const starClass = isFav && !inFavorites
      ? 'text-amber-500'
      : isFav
        ? 'text-amber-500 opacity-0 focus-visible:opacity-100 group-hover:opacity-100'
        : 'text-neutral-400 opacity-0 hover:text-amber-500 focus-visible:opacity-100 group-hover:opacity-100 dark:text-neutral-500'
    return (
      <div key={`${providerId}:${model}:${inFavorites ? 'fav' : 'grp'}`} className={modelRowClass(isEnterTarget)}>
        <button
          type="button"
          aria-current={selected ? 'true' : undefined}
          disabled={providerLocked && providerId !== currentProviderId}
          onClick={() => onSelect(providerId, model)}
          className={modelRowButtonClass(selected)}
        >
          <ModelIcon model={model} size={16} />
          <span className="min-w-0 truncate" title={model}>{providerModelLabel(model, visibleProviders.find(entry => entry.provider.id === providerId)?.provider)}</span>
        </button>
        <button
          type="button"
          aria-label={isFav ? t.chatUnfavorite : t.chatFavorite}
          title={isFav ? t.chatUnfavorite : t.chatFavorite}
          onClick={(e) => {
            e.stopPropagation()
            toggleFavorite(providerId, model)
          }}
          className={`grid size-6 shrink-0 place-items-center rounded-md outline-none transition-[color,opacity] ${starClass}`}
          data-menu-skip=""
          data-tauri-drag-region="false"
        >
          <Star size={13} fill={isFav ? 'currentColor' : 'none'} />
        </button>
        <ModelRowCheck selected={selected} />
      </div>
    )
  }

  return (
    <>
      {showSearch && (
        <ModelSearchField
          value={query}
          onChange={setQuery}
          inputRef={searchRef}
          placeholder={searchPlaceholder ?? t.chatSearchModels}
          trailing={searchTrailing}
          onSubmit={() => {
            if (firstAvailable) onSelect(firstAvailable.providerId, firstAvailable.model)
          }}
        />
      )}
      <div ref={listRef} className="custom-scrollbar min-h-0 flex-1 overflow-y-auto px-1 pb-1">
        {sections.map((section, index) => (
          <div
            key={section.key}
            role="group"
            aria-label={section.title}
            // 组与组之间只靠留白分开，不再叠一条分隔线。
            className={index > 0 ? 'mt-2' : undefined}
          >
            {/* 吸顶组名：滚动时始终知道当前在哪个服务商下，底部也不会只露一个孤立组名。 */}
            <div className="sticky top-0 z-[1] flex h-8 items-end gap-1 bg-[var(--theme-surface)] px-2 pb-1.5 text-[11px] font-medium text-neutral-400 dark:text-neutral-500">
              <span className="inline-flex min-w-0 items-center gap-1">
                {section.favorite && <Star size={10} fill="currentColor" className="shrink-0 text-amber-500" />}
                <span className="min-w-0 truncate">{section.title}</span>
              </span>
            </div>
            {section.rows.map(row => renderModelRow(row.providerId, row.model, section.favorite))}
          </div>
        ))}
        {sections.length === 0 && (
          <div className="px-4 py-6 text-center text-[12px] text-neutral-500">
            {visibleProviders.length === 0 ? t.chatNoModels : t.chatNoMatchingModels}
          </div>
        )}
      </div>
    </>
  )
}

function ModelSelectorBase({
  currentProviderId,
  currentModel,
  onModelChange,
  portal = false,
}: ModelSelectorProps) {
  const t = useT()
  const [open, setOpen] = useState(false)
  const [pointerOpened, setPointerOpened] = useState(false)
  const menuRef = useRef<HTMLDivElement>(null)
  const triggerRef = useRef<HTMLButtonElement>(null)
  const [position, setPosition] = useState<CSSProperties>({ left: 8, top: 8, width: 320, maxHeight: 400 })
  const closeMenu = useCallback(() => setOpen(false), [])
  usePopoverMenu(open, closeMenu, menuRef)
  const maxH = usePopoverMaxHeight(open && !portal, menuRef, 'down', 400)
  useLayoutEffect(() => {
    if (!open || !portal) return
    const measure = () => {
      const anchor = triggerRef.current?.getBoundingClientRect()
      if (!anchor) return
      const width = Math.min(320, window.innerWidth - 16)
      const below = window.innerHeight - anchor.bottom - 16
      const above = anchor.top - 16
      const upwards = below < 160 && above > below
      setPosition({
        left: Math.max(8, Math.min(anchor.left, window.innerWidth - width - 8)),
        ...(upwards ? { bottom: window.innerHeight - anchor.top + 8 } : { top: anchor.bottom + 8 }),
        width,
        maxHeight: Math.min(400, Math.max(80, upwards ? above : below)),
      })
    }
    const onScroll = (event: Event) => { if (!menuRef.current?.contains(event.target as Node)) measure() }
    measure()
    window.addEventListener('resize', measure)
    window.addEventListener('scroll', onScroll, true)
    return () => { window.removeEventListener('resize', measure); window.removeEventListener('scroll', onScroll, true) }
  }, [open, portal])

  const catalog = useBuiltinModelCatalog(currentProviderId, currentModel)
  const { currentProvider } = catalog
  const displayName = providerModelLabel(currentModel || currentProvider?.enabledModels[0] || t.chatSelectModel, currentProvider)
  // Tooltip 用模型原始元数据（服务商名 + 完整模型 ID），不截取 UI 上被省略的文本。
  const tooltipText = currentProvider && currentModel
    ? `${currentProvider.name}：${currentModel}`
    : (currentModel || '')

  const menu = open ? (
    <>
      <div className={portal ? 'fixed inset-0 z-[1000]' : 'fixed inset-0 z-10'} onClick={closeMenu} aria-hidden />
      <div ref={menuRef} style={portal ? position : { maxHeight: maxH }} className={`chat-model-selector-menu chat-motion-popover kv-menu flex flex-col overflow-hidden ${portal ? 'fixed z-[1001]' : 'absolute left-0 top-full z-20 mt-2 w-max min-w-[260px] max-w-[360px]'}`}>
        <BuiltinModelList
          catalog={catalog}
          currentProviderId={currentProviderId}
          currentModel={currentModel}
          autoFocusSearch={pointerOpened}
          onSelect={(providerId, model) => {
            onModelChange(providerId, model)
            closeMenu()
          }}
        />
      </div>
    </>
  ) : null

  return (
    <div className="relative max-w-full min-w-0" data-tauri-drag-region="false">
      <button
        ref={triggerRef}
        type="button"
        onClick={(event) => {
          if (open) {
            closeMenu()
            return
          }
          // detail 为 0 是键盘触发的 click（Enter / Space）。
          setPointerOpened(event.detail > 0)
          setOpen(true)
        }}
        aria-haspopup="menu"
        aria-expanded={open}
        title={tooltipText || undefined}
        className={`${chatTitlebarPillButtonClass} max-w-full min-w-0`}
      >
        {currentModel && <ModelIcon model={currentModel} size={16} />}
        <span className="chat-model-selector-label max-w-[200px] truncate font-medium text-neutral-800">{displayName}</span>
        <ChevronDown size={15} className={`chat-model-selector-caret shrink-0 text-neutral-400 transition-transform ease-kv-standard ${open ? 'rotate-180' : ''}`} />
      </button>
      {portal ? createPortal(menu, document.body) : menu}
    </div>
  )
}

// memo：顶栏选择器，仅在 props 变化时重渲，避免 Chat 重渲时跟着白渲。
export const ModelSelector = memo(ModelSelectorBase)
