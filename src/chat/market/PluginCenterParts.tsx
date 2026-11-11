// 插件中心 Skill / MCP 两页共用的页面骨架：页头、二级标签、可折叠分组、空态与「添加」菜单。
// 两页的业务状态各归各的页面；这里只统一外观与交互，样式在 market.css「Skill / MCP 页」。
import { useState, type ReactNode } from 'react'
import { ChevronDown, ChevronRight } from 'lucide-react'
import { Button } from '../../components/Button'
import { DockContextMenu, type DockMenuAnchor, type DockMenuItem } from '../dock/DockContextMenu'
import './market.css'

export function CenterHeader({ heading, subtitle, actions }: {
  heading: ReactNode
  subtitle: string
  actions: ReactNode
}) {
  return (
    <header className="kv-market-heading">
      <div className="min-w-0">
        {heading}
        <p>{subtitle}</p>
      </div>
      <div className="kv-market-toolbar" data-tauri-drag-region="false">{actions}</div>
    </header>
  )
}

export type CenterTab<T extends string> = { id: T; label: string; count?: number }

/** 页内二级标签：下划线跟随选中项。 */
export function CenterTabs<T extends string>({ tabs, value, onChange }: {
  tabs: readonly CenterTab<T>[]
  value: T
  onChange: (value: T) => void
}) {
  return (
    <nav className="kv-center-tabs" data-tauri-drag-region="false">
      {tabs.map((tab) => (
        <button
          key={tab.id}
          type="button"
          className="kv-center-tab"
          aria-current={value === tab.id ? 'page' : undefined}
          onClick={() => onChange(tab.id)}
        >
          {tab.label}
          {tab.count ? <span className="kv-center-tab-count">{tab.count}</span> : null}
          {value === tab.id && <span className="kv-center-tab-underline chat-motion-tab-underline" aria-hidden="true" />}
        </button>
      ))}
    </nav>
  )
}

/** 主动作「添加」：一个按钮收拢所有新增来源，菜单贴按钮右缘展开。 */
export function AddMenuButton({ label, items, disabled }: {
  label: string
  items: DockMenuItem[]
  disabled?: boolean
}) {
  const [anchor, setAnchor] = useState<DockMenuAnchor | null>(null)
  return (
    <>
      <Button
        variant="primary"
        disabled={disabled}
        aria-haspopup="menu"
        aria-expanded={Boolean(anchor)}
        onClick={(event) => {
          const rect = event.currentTarget.getBoundingClientRect()
          setAnchor(anchor ? null : { left: rect.right - 200, top: rect.bottom + 6 })
        }}
      >
        {label}
        <ChevronDown size={14} />
      </Button>
      {anchor && <DockContextMenu anchor={anchor} items={items} onClose={() => setAnchor(null)} />}
    </>
  )
}

/**
 * 分组：标题行可点击折叠，右侧是来源说明（过长截断，悬停看全文）。
 * 空分组只占一行说明，不再画大块虚线框。
 */
export function CenterSection({ title, count, meta, note, collapsed, onToggle, empty, children }: {
  title: string
  count: number
  /** 标题后的弱化信息，例如「已启用 3 个」 */
  meta?: string
  note?: string
  collapsed?: boolean
  onToggle?: () => void
  /** 分组为空时的一行说明（可带操作） */
  empty?: ReactNode
  children?: ReactNode
}) {
  const head = (
    <>
      {onToggle && (
        <ChevronRight size={14} className={`kv-center-section-chevron${collapsed ? '' : ' is-open'}`} aria-hidden="true" />
      )}
      <h3>{title}</h3>
      <span className="kv-center-section-count">{count}</span>
      {meta && <span className="kv-center-section-meta">{meta}</span>}
    </>
  )
  return (
    <section className={`kv-center-section${count === 0 ? ' is-empty' : ''}`}>
      <div className="kv-center-section-head">
        {onToggle ? (
          <button type="button" className="kv-center-section-toggle" aria-expanded={!collapsed} onClick={onToggle} data-tauri-drag-region="false">
            {head}
          </button>
        ) : (
          <div className="kv-center-section-toggle">{head}</div>
        )}
        {note && <span className="kv-center-section-note" title={note}>{note}</span>}
      </div>
      {collapsed ? null : count === 0 ? (
        empty ? <div className="kv-center-section-empty">{empty}</div> : null
      ) : (
        <div className="chat-motion-search-reveal">{children}</div>
      )}
    </section>
  )
}

/** 整页空态：图标、标题、一句话说明与下一步动作。 */
export function CenterEmpty({ icon, title, description, actions }: {
  icon: ReactNode
  title: string
  description?: string
  actions?: ReactNode
}) {
  return (
    <div className="kv-center-empty chat-motion-fade-up">
      <span className="kv-center-empty-icon" aria-hidden="true">{icon}</span>
      <strong>{title}</strong>
      {description && <p>{description}</p>}
      {actions && <div className="kv-center-empty-actions" data-tauri-drag-region="false">{actions}</div>}
    </div>
  )
}
