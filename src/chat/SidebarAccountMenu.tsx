import { useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { BarChart3, Globe, PawPrint } from 'lucide-react'
import { useCloseAnimation } from './useCloseAnimation'
import { i18n, type Lang } from '../components/i18n'
import { api } from '../api/tauri'
import { formatTokensCompact } from '../utils/tokens'
import { Toggle } from '../settings/public/controls'

interface SidebarAccountMenuProps {
  /** 触发行的视口矩形：菜单开在它上方（bottom 贴 rect.top），不遮住触发行本身。 */
  triggerRect: { left: number; top: number; width: number }
  lang: Lang
  onSelectLang: (lang: Lang) => void
  /** 打开设置「用量统计」页。行右侧另显示今日 token，点整行进详情。 */
  onOpenUsage: () => void
  onClose: () => void
}

export function SidebarAccountMenu({
  triggerRect,
  lang,
  onSelectLang,
  onOpenUsage,
  onClose: onCloseProp,
}: SidebarAccountMenuProps) {
  const t = i18n[lang]
  const menuRef = useRef<HTMLDivElement>(null)
  const { closing, startClose, onAnimationEnd } = useCloseAnimation(onCloseProp)
  const onClose = startClose
  const [todayTokens, setTodayTokens] = useState<number | null>(null)
  const [petEnabled, setPetEnabled] = useState<boolean | null>(null)
  const [petBusy, setPetBusy] = useState(false)
  const [petError, setPetError] = useState<'load' | 'save' | null>(null)
  const petVersion = useRef(0)
  const petActive = useRef(false)

  useEffect(() => {
    let cancelled = false
    let unlisten: (() => void) | undefined
    petActive.current = true
    void (async () => {
      try {
        // Subscribe first so tray actions and native right-click hide cannot
        // disappear between the initial snapshot and listener registration.
        unlisten = await api.onDesktopPetEnabledChanged(enabled => {
          if (cancelled) return
          petVersion.current += 1
          setPetEnabled(enabled)
          setPetError(null)
        })
        if (cancelled) {
          unlisten()
          return
        }
        const version = petVersion.current
        const enabled = await api.desktopPetGetEnabled()
        if (!cancelled && version === petVersion.current) setPetEnabled(enabled)
      } catch (error) {
        console.error('Failed to load desktop pet visibility:', error)
        if (!cancelled) setPetError('load')
      }
    })()
    return () => {
      cancelled = true
      petActive.current = false
      unlisten?.()
    }
  }, [])

  const changePet = async (enabled: boolean) => {
    if (petEnabled === null || petBusy) return
    setPetBusy(true)
    setPetError(null)
    const version = petVersion.current
    try {
      const applied = await api.desktopPetSetEnabled(enabled)
      if (petActive.current && version === petVersion.current) setPetEnabled(applied)
    } catch (error) {
      console.error('Failed to change desktop pet visibility:', error)
      if (petActive.current) setPetError('save')
    } finally {
      if (petActive.current) setPetBusy(false)
    }
  }

  useEffect(() => {
    let cancelled = false
    void (async () => {
      try {
        const stats = await api.usageGetStats({ range: 'today', limit: 0 })
        if (!cancelled) setTodayTokens(stats.summary.totalTokens)
      } catch (err) {
        console.error('Failed to load today usage:', err)
        if (!cancelled) setTodayTokens(0)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [])

  useEffect(() => {
    const onPointerDown = (e: MouseEvent) => {
      if (menuRef.current?.contains(e.target as Node)) return
      onClose()
    }
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('mousedown', onPointerDown)
    window.addEventListener('keydown', onKeyDown)
    return () => {
      window.removeEventListener('mousedown', onPointerDown)
      window.removeEventListener('keydown', onKeyDown)
    }
  }, [onClose])

  const menu = (
    <div
      ref={menuRef}
      className={`kv-menu ${
        closing ? 'chat-motion-popover-out' : 'chat-motion-popover chat-motion-menu-cascade'
      } fixed z-[200]`}
      // bottom 定位：菜单向上生长，触发行始终可见。宽度对齐触发行，读作「从这一行展开」。
      style={{
        left: triggerRect.left,
        bottom: window.innerHeight - triggerRect.top + 6,
        width: triggerRect.width,
        ['--chat-popover-origin' as string]: 'bottom left',
      }}
      role="menu"
      onAnimationEnd={onAnimationEnd}
    >
      <button
        type="button"
        role="menuitem"
        className="kv-menu-item"
        onClick={() => {
          onOpenUsage()
          onClose()
        }}
      >
        <BarChart3 strokeWidth={1.75} />
        {t.modelUsage}
        <span className="ml-auto tabular-nums text-[11px] text-neutral-500 dark:text-neutral-400">
          {todayTokens == null ? '' : formatTokensCompact(todayTokens)}
        </span>
      </button>

      <div className="kv-menu-sep" />

      {/* 语言：二选一不值一个子菜单、更不值两行。压进单行，右侧紧凑分段控件。 */}
      <div className="kv-menu-item" style={{ cursor: 'default' }}>
        <Globe strokeWidth={1.75} />
        {t.language}
        <div className="ml-auto flex shrink-0 items-center gap-px rounded-[5px] bg-neutral-900/[0.05] p-px">
          {(
            [
              ['zh', '中'],
              ['en', 'EN'],
            ] as const
          ).map(([value, label]) => (
            <button
              key={value}
              type="button"
              role="menuitemradio"
              aria-checked={lang === value}
              onClick={() => onSelectLang(value)}
              className={`rounded-[4px] px-1.5 py-0.5 text-[11px] font-medium leading-none transition-colors ${
                lang === value
                  ? 'bg-[var(--theme-surface-active)] text-neutral-900 shadow-sm'
                  : 'text-neutral-500 hover:text-neutral-800 dark:text-neutral-400'
              }`}
            >
              {label}
            </button>
          ))}
        </div>
      </div>

      <div className="kv-menu-sep" />
      <div className="kv-menu-item" style={{ cursor: 'default' }} aria-busy={petBusy || (petEnabled === null && petError === null)}>
        <PawPrint strokeWidth={1.75} />
        {t.desktopPet}
        <div className="ml-auto flex shrink-0 items-center">
          <Toggle
            checked={petEnabled === true}
            disabled={petEnabled === null || petBusy}
            onChange={enabled => { void changePet(enabled) }}
            ariaLabel={t.desktopPet}
          />
        </div>
      </div>
      {petError && (
        <p role="alert" className="px-3 pb-2 text-xs text-[var(--danger)]">
          {petError === 'load' ? t.desktopPetLoadFailed : t.desktopPetSaveFailed}
        </p>
      )}
    </div>
  )

  return createPortal(menu, document.body)
}
