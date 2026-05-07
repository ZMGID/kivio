import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from 'react'
import { createPortal } from 'react-dom'
import { Check, ChevronDown, ExternalLink, type LucideIcon } from 'lucide-react'
import { formatHotkey, getPlatform } from './utils'

function useSelectMenuRect(
  open: boolean,
  value: string,
  optionsLength: number,
  triggerRef: RefObject<HTMLButtonElement | null>,
) {
  const [menuRect, setMenuRect] = useState({ left: 0, top: 0, width: 0 })

  const updateMenuRect = useCallback(() => {
    const trigger = triggerRef.current
    if (!trigger) return
    const rect = trigger.getBoundingClientRect()
    setMenuRect({
      left: rect.left,
      top: rect.bottom + 6,
      width: rect.width,
    })
  }, [triggerRef])

  useLayoutEffect(() => {
    if (open) updateMenuRect()
  }, [open, value, optionsLength, updateMenuRect])

  return { menuRect, updateMenuRect }
}

/**
 * 开关切换组件 — on 态用 brand 蓝，slider 加双层阴影
 */
export function Toggle({ checked, onChange }: { checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      type="button"
      onClick={() => onChange(!checked)}
      role="switch"
      aria-checked={checked}
      className={`relative w-[36px] h-[22px] rounded-full transition-colors duration-200 ease-out ${
        checked
          ? 'bg-[#2563eb] dark:bg-blue-500'
          : 'bg-neutral-300/80 dark:bg-neutral-700'
      }`}
      data-tauri-drag-region="false"
    >
      <span
        className={`absolute top-[2px] left-[2px] w-[18px] h-[18px] bg-white dark:bg-white rounded-full transition-transform duration-200 ease-out ${
          checked ? 'translate-x-[14px]' : ''
        }`}
        style={{ boxShadow: '0 1px 2px rgba(0,0,0,0.18), 0 2px 4px rgba(0,0,0,0.08)' }}
      />
    </button>
  )
}

/**
 * 下拉选择 — 自绘菜单，避免 macOS 原生 select 的系统高亮/勾选反馈和受控状态不同步。
 */
export function Select({ value, onChange, options, className = '' }: {
  value: string
  onChange: (v: string) => void
  options: { value: string; label: string }[]
  className?: string
}) {
  const [open, setOpen] = useState(false)
  const triggerRef = useRef<HTMLButtonElement | null>(null)
  const menuRef = useRef<HTMLDivElement | null>(null)
  const selected = options.find(opt => opt.value === value)
  const displayLabel = selected?.label || value
  const disabled = options.length === 0
  const { menuRect, updateMenuRect } = useSelectMenuRect(open, value, options.length, triggerRef)

  useEffect(() => {
    if (!open) return
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target as Node
      if (triggerRef.current?.contains(target) || menuRef.current?.contains(target)) return
      setOpen(false)
    }
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setOpen(false)
        triggerRef.current?.focus()
      }
    }
    const handleLayoutChange = () => updateMenuRect()

    document.addEventListener('pointerdown', handlePointerDown)
    document.addEventListener('keydown', handleKeyDown)
    window.addEventListener('resize', handleLayoutChange)
    window.addEventListener('scroll', handleLayoutChange, true)
    return () => {
      document.removeEventListener('pointerdown', handlePointerDown)
      document.removeEventListener('keydown', handleKeyDown)
      window.removeEventListener('resize', handleLayoutChange)
      window.removeEventListener('scroll', handleLayoutChange, true)
    }
  }, [open, updateMenuRect])

  return (
    <div className={`relative ${className}`}>
      <button
        ref={triggerRef}
        type="button"
        disabled={disabled}
        onClick={() => setOpen(v => !v)}
        onKeyDown={(event) => {
          if (event.key === 'ArrowDown' || event.key === 'Enter' || event.key === ' ') {
            event.preventDefault()
            setOpen(true)
          }
        }}
        className="settings-control w-full h-[32px] px-3 py-1.5 pr-8 text-[13px] font-medium text-left disabled:opacity-50 disabled:cursor-not-allowed"
        aria-haspopup="listbox"
        aria-expanded={open}
        data-tauri-drag-region="false"
      >
        <span className="block truncate">{displayLabel}</span>
        <ChevronDown
          size={14}
          strokeWidth={2.25}
          className={`absolute right-2.5 top-1/2 -translate-y-1/2 text-neutral-400 dark:text-neutral-500 transition-transform ${open ? 'rotate-180' : ''}`}
        />
      </button>

      {open && createPortal(
        <div
          ref={menuRef}
          role="listbox"
          className="fixed z-[1000] max-h-[260px] overflow-y-auto rounded-lg border border-black/10 dark:border-white/10 bg-white/95 dark:bg-neutral-900/95 shadow-[0_12px_36px_rgba(0,0,0,0.18)] backdrop-blur-xl custom-scrollbar p-1"
          style={{ left: menuRect.left, top: menuRect.top, width: menuRect.width }}
          data-tauri-drag-region="false"
        >
          {options.map(opt => {
            const active = opt.value === value
            return (
              <button
                key={opt.value}
                type="button"
                role="option"
                aria-selected={active}
                onClick={() => {
                  onChange(opt.value)
                  setOpen(false)
                  triggerRef.current?.focus()
                }}
                className={`relative flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 pr-8 text-left text-[13px] leading-5 transition-colors ${
                  active
                    ? 'bg-blue-600 text-white'
                    : 'text-neutral-800 dark:text-neutral-100 hover:bg-black/[0.05] dark:hover:bg-white/[0.08]'
                }`}
                data-tauri-drag-region="false"
              >
                <span className="min-w-0 flex-1 truncate">{opt.label}</span>
                {active && (
                  <Check
                    size={14}
                    strokeWidth={2.5}
                    className="absolute right-2.5 top-1/2 -translate-y-1/2"
                  />
                )}
              </button>
            )
          })}
        </div>,
        document.body,
      )}
    </div>
  )
}

/**
 * 文本输入 — 默认 sans，需要等宽时调用方自行加 font-mono
 */
export function Input({ value, onChange, type = 'text', placeholder = '', className = '', list, mono = false, ...props }: {
  value: string
  onChange: (v: string) => void
  type?: string
  placeholder?: string
  className?: string
  list?: string
  /** 启用 font-mono（仅 baseUrl/apiKey/model 名等代码型字段使用） */
  mono?: boolean
} & Omit<React.InputHTMLAttributes<HTMLInputElement>, 'value' | 'onChange'>) {
  return (
    <input
      type={type}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={placeholder}
      list={list}
      className={`settings-control w-full px-3 py-1.5 text-[13px] ${mono ? 'font-mono' : ''} ${className}`}
      data-tauri-drag-region="false"
      {...props}
    />
  )
}

/**
 * 多行文本输入 — 默认 sans
 */
export function TextArea({ value, onChange, placeholder = '', rows = 2, mono = false }: {
  value: string
  onChange: (v: string) => void
  placeholder?: string
  rows?: number
  mono?: boolean
}) {
  return (
    <textarea
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={placeholder}
      rows={rows}
      className={`settings-control w-full px-3 py-2 text-[13px] resize-none leading-relaxed ${mono ? 'font-mono' : ''}`}
      data-tauri-drag-region="false"
    />
  )
}

/**
 * 字段标签
 */
export function Label({ children, className = '' }: { children: ReactNode; className?: string }) {
  return (
    <label className={`block text-[12px] font-medium text-neutral-600 dark:text-neutral-300 mb-1.5 ${className}`}>
      {children}
    </label>
  )
}

/**
 * 设置项行（左 label + 可选 description，右控件）
 */
export function SettingRow({ label, description, children, className = '' }: {
  label: string
  description?: string
  children: ReactNode
  className?: string
}) {
  return (
    <div className={`flex items-center justify-between gap-4 py-3 px-4 ${className}`}>
      <div className="flex-1 min-w-0">
        <span className="text-[13px] text-neutral-900 dark:text-neutral-100">{label}</span>
        {description && (
          <p className="text-[11px] text-neutral-500 dark:text-neutral-400 mt-0.5 leading-snug">{description}</p>
        )}
      </div>
      <div className="shrink-0 flex items-center">{children}</div>
    </div>
  )
}

/**
 * 权限状态项（macOS）
 */
export function PermissionItem({
  label,
  granted,
  grantedText,
  missingText,
  actionLabel,
  onOpen,
}: {
  label: string
  granted: boolean
  grantedText: string
  missingText: string
  actionLabel: string
  onOpen: () => void
}) {
  return (
    <div className="flex items-center justify-between gap-3 py-3 px-4">
      <div className="min-w-0 flex items-center gap-2.5">
        <span className={`relative flex h-2 w-2 shrink-0`}>
          {!granted && (
            <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-amber-400 opacity-50" />
          )}
          <span className={`relative inline-flex rounded-full h-2 w-2 ${granted ? 'bg-emerald-500' : 'bg-amber-500'}`} />
        </span>
        <div className="min-w-0">
          <p className="text-[13px] text-neutral-900 dark:text-neutral-100">{label}</p>
          <p className={`text-[11px] mt-0.5 ${granted ? 'text-emerald-600 dark:text-emerald-400' : 'text-amber-600 dark:text-amber-400'}`}>
            {granted ? grantedText : missingText}
          </p>
        </div>
      </div>
      {!granted && (
        <button
          type="button"
          onClick={onOpen}
          className="inline-flex items-center gap-1 px-2.5 py-1 text-[11px] rounded-md border border-black/10 dark:border-white/10 text-neutral-600 dark:text-neutral-300 hover:text-neutral-900 dark:hover:text-white hover:bg-black/5 dark:hover:bg-white/5 transition-all"
          data-tauri-drag-region="false"
        >
          <ExternalLink size={11} />
          {actionLabel}
        </button>
      )}
    </div>
  )
}

/**
 * 键盘按键徽章
 */
export function KeyBadge({ children }: { children: ReactNode }) {
  return (
    <kbd
      className="inline-flex items-center justify-center min-w-[24px] h-[24px] px-1.5 rounded-md bg-white dark:bg-neutral-800 border border-neutral-300/80 dark:border-neutral-600 text-[11px] font-medium text-neutral-700 dark:text-neutral-200"
      style={{ boxShadow: '0 1px 0 rgba(0,0,0,0.06), inset 0 -1px 0 rgba(0,0,0,0.04)' }}
    >
      {children}
    </kbd>
  )
}

/**
 * 快捷键展示
 */
export function HotkeyDisplay({ hotkey }: { hotkey: string }) {
  const platform = getPlatform()
  const keys = formatHotkey(hotkey, platform)
  return (
    <div className="flex items-center gap-1">
      {keys.map((k, i) => (
        <KeyBadge key={i}>{k}</KeyBadge>
      ))}
    </div>
  )
}

/**
 * 快捷键输入（含录制态）
 */
export function HotkeyInput({
  value,
  placeholder,
  recording,
  onToggleRecording,
  recordLabel,
  recordingLabel,
  recordingPlaceholder,
}: {
  value: string
  placeholder: string
  recording: boolean
  onToggleRecording: () => void
  recordLabel: string
  recordingLabel: string
  recordingPlaceholder: string
}) {
  return (
    <div className="flex items-center gap-2">
      <div
        className={`flex-1 flex items-center gap-1 min-h-[36px] px-2.5 rounded-md border transition-all ${
          recording
            ? 'border-amber-400/70 dark:border-amber-300/60 bg-amber-50/60 dark:bg-amber-900/15 ring-2 ring-amber-400/20 dark:ring-amber-300/20'
            : 'border-black/[0.06] dark:border-white/[0.07] bg-black/[0.03] dark:bg-white/[0.04]'
        }`}
      >
        {recording ? (
          <span className="text-[12px] text-amber-600 dark:text-amber-300 animate-pulse">{recordingPlaceholder}</span>
        ) : value ? (
          <HotkeyDisplay hotkey={value} />
        ) : (
          <span className="text-[12px] text-neutral-400 dark:text-neutral-500">{placeholder}</span>
        )}
      </div>
      <button
        type="button"
        onClick={onToggleRecording}
        className={`px-3 h-[36px] rounded-md text-[12px] font-medium border transition-all ${
          recording
            ? 'border-amber-400/70 text-amber-700 dark:text-amber-300 bg-amber-50/80 dark:bg-amber-900/25'
            : 'border-black/10 dark:border-white/10 text-neutral-600 dark:text-neutral-300 hover:text-neutral-900 dark:hover:text-neutral-100 hover:bg-black/5 dark:hover:bg-white/5'
        }`}
        data-tauri-drag-region="false"
      >
        {recording ? recordingLabel : recordLabel}
      </button>
    </div>
  )
}

/**
 * 默认提示词预览（折叠在卡片底部，灰底等宽）
 */
export function DefaultPrompt({ label, content }: { label: string; content: string }) {
  return (
    <div className="mt-2 rounded-md border border-black/[0.05] dark:border-white/[0.05] bg-neutral-50 dark:bg-neutral-800/40 px-3 py-2">
      <div className="text-[10px] font-semibold uppercase tracking-wider text-neutral-400 dark:text-neutral-500 mb-1">
        {label}
      </div>
      <pre className="whitespace-pre-wrap text-[11px] text-neutral-600 dark:text-neutral-300 font-mono leading-relaxed">
        {content.trim()}
      </pre>
    </div>
  )
}

/**
 * 区块标题 — 小号灰 uppercase + 左侧 brand 细色条
 * 让标题谦逊，把视觉重心交给卡片本身
 */
export function SectionTitle({ children, icon: Icon }: { children: ReactNode; icon?: LucideIcon }) {
  return (
    <div className="flex items-center gap-2 mb-2.5 pl-0.5">
      <span className="w-[3px] h-3 rounded-full bg-[#2563eb] dark:bg-blue-400" />
      {Icon && <Icon size={12} className="text-neutral-500 dark:text-neutral-400" />}
      <h3 className="text-[11px] font-semibold uppercase tracking-[0.08em] text-neutral-500 dark:text-neutral-400">
        {children}
      </h3>
    </div>
  )
}

/**
 * 分段控制器标签按钮（轻量样式）
 */
export function TabButton({ active, onClick, label }: {
  active: boolean
  onClick: () => void
  label: string
}) {
  return (
    <button
      onClick={onClick}
      className={`flex-1 px-3 py-1.5 rounded-md text-[12px] font-medium transition-all duration-200 ${active
        ? 'bg-white dark:bg-neutral-700 text-neutral-900 dark:text-white shadow-sm'
        : 'text-neutral-500 dark:text-neutral-400 hover:text-neutral-700 dark:hover:text-neutral-300'
        }`}
      data-tauri-drag-region="false"
    >
      {label}
    </button>
  )
}
