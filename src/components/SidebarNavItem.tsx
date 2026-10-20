import type { ReactNode } from 'react'

/** Shared desktop navigation appearance; each surface owns its destinations. */
export function SidebarNavItem({ icon, label, onClick, disabled, active }: {
  icon: ReactNode
  label: string
  onClick?: () => void
  disabled?: boolean
  active?: boolean
}) {
  return (
    <button
      type="button"
      className="kv-sidebar-nav-item kv-nav-motion"
      onClick={onClick}
      disabled={disabled}
      aria-current={active ? 'page' : undefined}
      title={label}
      data-tauri-drag-region="false"
    >
      <span className="kv-sidebar-nav-icon">{icon}</span>
      <span className="kv-sidebar-nav-label">{label}</span>
    </button>
  )
}
