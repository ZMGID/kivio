// 复刻 T3 Code threadDetailsPanelStyles / ThreadDetailsControl：32px 圆角行，静止无底色，悬停淡填充；
// 拆分按钮 = 主按钮 + 悬停才显示的分隔线 + 右侧 chevron，下拉与整组同宽。
// 动效复用全局 chat-motion-popover / -out / -menu-cascade（见 workspacePanelMotion），reduced-motion 由全局兜底。
import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactNode, type RefObject } from 'react'
import { createPortal } from 'react-dom'
import { ChevronDown } from 'lucide-react'
import { motionClass, usePresence } from './workspacePanelMotion'

const SURFACE = 'hover:bg-black/[0.035] dark:hover:bg-white/[0.08]'
const OPEN_SURFACE = 'bg-black/[0.035] dark:bg-white/[0.08]'
const CONTROL = 'inline-flex h-8 min-w-0 items-center gap-2.5 rounded-lg px-2.5 text-left text-[13px] font-medium text-neutral-700 outline-none transition focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-neutral-300/70 disabled:pointer-events-none disabled:opacity-60 dark:text-neutral-200 dark:focus-visible:ring-white/20'
const PRESS = 'active:scale-[0.98] motion-reduce:active:scale-100'

export const PANEL_ICON = 'size-4 shrink-0 text-neutral-400 dark:text-neutral-500'
/** 面板与其下拉共用的标记：点在带标记的元素内不算「点面板外」。 */
export const PANEL_SCOPE_ATTR = 'data-workspace-panel'

/** onClick 缺省时渲染为静态行（不可聚焦）；trailing 只在悬停或键盘聚焦时出现。 */
export function PanelRow({ icon, children, trailing, onClick, disabled, title }: {
  icon: ReactNode
  children: ReactNode
  trailing?: ReactNode
  onClick?: () => void
  disabled?: boolean
  title?: string
}) {
  const content = (
    <>
      {icon}
      {children}
      {trailing && (
        <span className="ms-auto flex shrink-0 opacity-0 transition-opacity duration-150 group-hover/row:opacity-100 group-focus-visible/row:opacity-100">
          {trailing}
        </span>
      )}
    </>
  )
  if (!onClick) return <div title={title} className={`${CONTROL} w-full`}>{content}</div>
  return (
    <button type="button" title={title} disabled={disabled} onClick={onClick} className={`group/row ${CONTROL} w-full ${SURFACE} ${PRESS}`}>
      {content}
    </button>
  )
}

export function PanelSection({ children }: { children: ReactNode }) {
  return <div className="flex flex-col border-t border-neutral-200/60 p-1.5 first:border-t-0 dark:border-white/10">{children}</div>
}

/** 加载中的占位行，宽度不同避免整齐的条纹感。 */
export function PanelSkeletonRows({ widths }: { widths: number[] }) {
  return (
    <>
      {widths.map((width, index) => (
        <div key={index} aria-hidden className="flex h-8 items-center gap-2.5 px-2.5">
          <span className="kv-skeleton size-4 shrink-0 rounded" />
          <span className="kv-skeleton h-3 rounded" style={{ width }} />
        </div>
      ))}
    </>
  )
}

/**
 * menu 为渲染下拉内容的函数，参数用于选中后关闭下拉。onPrimary 缺省时点标签也打开下拉。
 * meta 是主按钮右侧的弱化说明（同步状态、不可用原因）。
 */
export function SplitRow({ icon, label, meta, onPrimary, disabled, hint, menuLabel, menu }: {
  icon: ReactNode
  label: ReactNode
  meta?: ReactNode
  onPrimary?: () => void
  disabled?: boolean
  /** 主按钮不可用的完整原因，悬停整行可见。 */
  hint?: string
  menuLabel: string
  menu: ((close: () => void) => ReactNode) | null
}) {
  const [open, setOpen] = useState(false)
  const presence = usePresence(open)
  const groupRef = useRef<HTMLDivElement>(null)
  const close = () => setOpen(false)
  const toggle = () => setOpen((prev) => !prev)
  const primary = onPrimary ?? (menu ? toggle : undefined)
  return (
    <div ref={groupRef} title={hint} className={`group/split relative flex w-full items-center rounded-lg transition-colors ${SURFACE} ${open ? OPEN_SURFACE : ''}`}>
      <button
        type="button"
        disabled={disabled || !primary}
        onClick={primary}
        aria-haspopup={onPrimary ? undefined : 'menu'}
        aria-expanded={onPrimary ? undefined : open}
        className={`${CONTROL} flex-1 justify-start rounded-e-none ${SURFACE} ${PRESS}`}
      >
        {icon}
        <span className="min-w-0 flex-1 truncate">{label}</span>
        {meta && <span className="shrink-0 text-[12px] font-normal tabular-nums text-neutral-400 dark:text-neutral-500">{meta}</span>}
      </button>
      {menu && (
        <>
          <span aria-hidden className={`h-4 w-px shrink-0 bg-transparent transition-colors group-hover/split:bg-neutral-300/70 dark:group-hover/split:bg-white/20 ${open ? '!bg-neutral-300/70 dark:!bg-white/20' : ''}`} />
          <button
            type="button"
            aria-label={menuLabel}
            aria-haspopup="menu"
            aria-expanded={open}
            onClick={toggle}
            className={`${CONTROL} w-8 justify-center rounded-s-none px-0 ${SURFACE}`}
          >
            <ChevronDown className={`${PANEL_ICON} transition-transform duration-200 ${open ? 'rotate-180' : ''}`} />
          </button>
          {presence.present && (
            <SplitMenu anchor={groupRef} closing={presence.closing} onClose={close}>{menu(close)}</SplitMenu>
          )}
        </>
      )}
    </div>
  )
}

const MENU_ITEM_SELECTOR = '[role^="menuitem"]:not(:disabled)'

/** 下拉挂到 body：不被面板滚动区裁切；点外部、滚动或 Esc 关闭，Esc 不连带关掉面板。方向键在项间移动。 */
function SplitMenu({ anchor, closing, onClose, children }: {
  anchor: RefObject<HTMLDivElement | null>
  closing: boolean
  onClose: () => void
  children: ReactNode
}) {
  const menuRef = useRef<HTMLDivElement>(null)
  const [rect, setRect] = useState<DOMRect | null>(null)
  useLayoutEffect(() => { setRect(anchor.current?.getBoundingClientRect() ?? null) }, [anchor])
  useEffect(() => {
    if (closing) return
    const menu = menuRef.current
    ;(menu?.querySelector<HTMLElement>('input') ?? menu?.querySelector<HTMLElement>(MENU_ITEM_SELECTOR))?.focus({ preventScroll: true })
    const onDown = (event: PointerEvent) => {
      const target = event.target as Node
      if (!menuRef.current?.contains(target) && !anchor.current?.contains(target)) onClose()
    }
    const onKey = (event: globalThis.KeyboardEvent) => {
      if (event.key !== 'Escape') return
      event.stopPropagation()
      onClose()
      anchor.current?.querySelector<HTMLElement>('[aria-haspopup="menu"]:last-of-type')?.focus({ preventScroll: true })
    }
    const onScroll = (event: Event) => {
      if (!menuRef.current?.contains(event.target as Node)) onClose()
    }
    document.addEventListener('pointerdown', onDown)
    window.addEventListener('keydown', onKey, true)
    window.addEventListener('scroll', onScroll, true)
    window.addEventListener('resize', onClose)
    return () => {
      document.removeEventListener('pointerdown', onDown)
      window.removeEventListener('keydown', onKey, true)
      window.removeEventListener('scroll', onScroll, true)
      window.removeEventListener('resize', onClose)
    }
  }, [anchor, closing, onClose])
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const items = [...(menuRef.current?.querySelectorAll<HTMLElement>(MENU_ITEM_SELECTOR) ?? [])]
    if (!items.length) return
    const index = items.indexOf(document.activeElement as HTMLElement)
    const next = event.key === 'ArrowDown' ? (index + 1) % items.length
      : event.key === 'ArrowUp' ? (index - 1 + items.length) % items.length
        : event.key === 'Home' ? 0
          : event.key === 'End' ? items.length - 1
            : -1
    if (next < 0) return
    event.preventDefault()
    items[next].focus({ preventScroll: true })
  }
  if (!rect) return null
  const below = window.innerHeight - rect.bottom - 12
  const placeAbove = below < 160 && rect.top > below
  return createPortal(
    <div
      ref={menuRef}
      role="menu"
      aria-hidden={closing || undefined}
      onKeyDown={onKeyDown}
      {...{ [PANEL_SCOPE_ATTR]: '' }}
      data-tauri-drag-region="false"
      className={`kv-menu custom-scrollbar fixed z-50 overflow-y-auto overscroll-contain ${motionClass(closing)}`}
      style={{
        left: rect.left,
        width: rect.width,
        maxHeight: Math.min(320, Math.max(placeAbove ? rect.top - 12 : below, 96)),
        ['--chat-popover-origin' as string]: placeAbove ? 'bottom center' : 'top center',
        ['--chat-popover-start-y' as string]: placeAbove ? '3px' : '-3px',
        ...(placeAbove ? { bottom: window.innerHeight - rect.top + 4 } : { top: rect.bottom + 4 }),
      }}
    >
      {children}
    </div>,
    document.body,
  )
}
