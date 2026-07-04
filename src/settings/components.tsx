import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from 'react'
import { createPortal } from 'react-dom'
import { Check, ChevronDown, ExternalLink, X, type LucideIcon } from 'lucide-react'
import { formatHotkey, getPlatform, type SelectOption } from './utils'

const MENU_GAP = 6
const MENU_MARGIN = 8
const MENU_MAX_HEIGHT = 260

function useSelectMenuRect(
  open: boolean,
  value: string,
  optionsLength: number,
  triggerRef: RefObject<HTMLButtonElement | null>,
) {
  const [menuRect, setMenuRect] = useState<{
    left: number
    top?: number
    bottom?: number
    width: number
    maxHeight: number
  }>({ left: 0, top: 0, width: 0, maxHeight: MENU_MAX_HEIGHT })

  const updateMenuRect = useCallback(() => {
    const trigger = triggerRef.current
    if (!trigger) return
    const rect = trigger.getBoundingClientRect()
    const viewportH = window.innerHeight
    const spaceBelow = viewportH - rect.bottom - MENU_GAP - MENU_MARGIN
    const spaceAbove = rect.top - MENU_GAP - MENU_MARGIN
    // 預設向下展開；下方空間不足且上方更寬裕時向上翻轉。
    const flipUp = spaceBelow < MENU_MAX_HEIGHT && spaceAbove > spaceBelow
    const available = Math.max(flipUp ? spaceAbove : spaceBelow, 0)
    const maxHeight = Math.max(Math.min(MENU_MAX_HEIGHT, available), 80)
    if (flipUp) {
      // 用 bottom 定位讓選單底邊貼著按鈕向上生長，避免 top 計算後恆等於 MENU_MARGIN 導致飛到視窗頂部。
      setMenuRect({ left: rect.left, bottom: viewportH - rect.top + MENU_GAP, width: rect.width, maxHeight })
    } else {
      setMenuRect({ left: rect.left, top: rect.bottom + MENU_GAP, width: rect.width, maxHeight })
    }
  }, [triggerRef])

  useLayoutEffect(() => {
    if (open) updateMenuRect()
  }, [open, value, optionsLength, updateMenuRect])

  return { menuRect, updateMenuRect }
}

/**
 * 開關切換元件 — on 態用 brand 藍，slider 加雙層陰影
 */
export function Toggle({ checked, onChange }: { checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      type="button"
      onClick={() => onChange(!checked)}
      role="switch"
      aria-checked={checked}
      className={`kv-toggle ${checked ? 'on' : ''}`}
      data-tauri-drag-region="false"
    />
  )
}

/**
 * 下拉選擇 — 自繪選單，避免 macOS 原生 select 的系統高亮/勾選反饋和受控狀態不同步。
 */
export function Select({ value, onChange, options, className = '' }: {
  value: string
  onChange: (v: string) => void
  options: SelectOption[]
  className?: string
}) {
  const [open, setOpen] = useState(false)
  const triggerRef = useRef<HTMLButtonElement | null>(null)
  const menuRef = useRef<HTMLDivElement | null>(null)
  const selected = options.find(opt => opt.value === value)
  const displayLabel = selected?.label || value
  const displayTitle = selected?.title || displayLabel
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
        className="kv-select kv-select-button relative w-full h-[30px] text-left disabled:opacity-50 disabled:cursor-not-allowed"
        aria-haspopup="listbox"
        aria-expanded={open}
        title={displayTitle}
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
          className="kv-select-menu fixed z-[1000] overflow-y-auto custom-scrollbar p-1"
          style={{ left: menuRect.left, top: menuRect.top, bottom: menuRect.bottom, width: menuRect.width, maxHeight: menuRect.maxHeight }}
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
                title={opt.title || opt.label}
                className={`relative flex w-full items-center gap-2 rounded-md px-2.5 py-1.5 pr-8 text-left text-[12.5px] leading-5 transition-colors ${
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
 * 文字輸入 — 預設 sans，需要等寬時呼叫方自行加 font-mono
 */
export function Input({ value, onChange, type = 'text', placeholder = '', className = '', list, mono = false, ...props }: {
  value: string
  onChange: (v: string) => void
  type?: string
  placeholder?: string
  className?: string
  list?: string
  /** 啟用 font-mono（僅 baseUrl/apiKey/model 名等程式碼型欄位使用） */
  mono?: boolean
} & Omit<React.InputHTMLAttributes<HTMLInputElement>, 'value' | 'onChange'>) {
  return (
    <input
      type={type}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={placeholder}
      list={list}
      className={`kv-input w-full ${mono ? 'mono' : ''} ${className}`}
      data-tauri-drag-region="false"
      {...props}
    />
  )
}

/**
 * 多行文字輸入 — 預設 sans
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
      className={`kv-textarea w-full ${mono ? 'mono' : ''}`}
      data-tauri-drag-region="false"
    />
  )
}

/**
 * 欄位標籤
 */
export function Label({ children, className = '' }: { children: ReactNode; className?: string }) {
  return (
    <label className={`kv-field-label ${className}`}>
      {children}
    </label>
  )
}

/**
 * 設定項行（左 label + 可選 description，右控制元件）
 */
export function SettingRow({ label, description, children, className = '', stack = false }: {
  label: ReactNode
  description?: string
  children: ReactNode
  className?: string
  stack?: boolean
}) {
  return (
    <div className={`${stack ? 'kv-row-stack' : 'kv-row'} ${className}`}>
      <div className="kv-row-text">
        <span className="kv-row-label">{label}</span>
        {description && (
          <p className="kv-row-desc">{description}</p>
        )}
      </div>
      {stack ? children : <div className="kv-row-control">{children}</div>}
    </div>
  )
}

export function SettingsGroup({ title, children, className = '' }: {
  title?: ReactNode
  children: ReactNode
  className?: string
}) {
  return (
    <section className={`kv-group ${className}`}>
      {title && <div className="kv-group-title">{title}</div>}
      {children}
    </section>
  )
}

/**
 * 許可權狀態項（macOS）
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
    <div className="kv-row">
      <div className="kv-row-text flex items-center gap-2.5">
        <span className={`relative flex h-2 w-2 shrink-0`}>
          {!granted && (
            <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-amber-400 opacity-50" />
          )}
          <span className={`relative inline-flex rounded-full h-2 w-2 ${granted ? 'bg-emerald-500' : 'bg-amber-500'}`} />
        </span>
        <div className="min-w-0">
          <p className="kv-row-label">{label}</p>
          <p className={`text-[11px] mt-0.5 ${granted ? 'text-emerald-600 dark:text-emerald-400' : 'text-amber-600 dark:text-amber-400'}`}>
            {granted ? grantedText : missingText}
          </p>
        </div>
      </div>
      {!granted && (
        <button
          type="button"
          onClick={onOpen}
          className="kv-btn sm"
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
 * 鍵盤按鍵徽章
 */
export function KeyBadge({ children }: { children: ReactNode }) {
  return (
    <kbd
      className="kv-kbd"
    >
      {children}
    </kbd>
  )
}

/**
 * 快捷鍵展示
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
 * 快捷鍵輸入(含錄製態)
 * onClear / clearLabel: 提供時,值非空且非錄製態會渲染 X 按鈕以清空(給"想關掉某個功能的熱鍵"留出口);留空則不顯示
 * error: 客戶端衝突等校驗訊息,以紅色小字顯示在輸入框下方
 */
export function HotkeyInput({
  value,
  placeholder,
  recording,
  onToggleRecording,
  recordLabel,
  recordingLabel,
  recordingPlaceholder,
  onClear,
  clearLabel,
  error,
}: {
  value: string
  placeholder: string
  recording: boolean
  onToggleRecording: () => void
  recordLabel: string
  recordingLabel: string
  recordingPlaceholder: string
  onClear?: () => void
  clearLabel?: string
  error?: string
}) {
  const showClear = !!onClear && !!value && !recording
  return (
    <div className="space-y-1">
      <div className="flex items-center gap-2">
        <div
          className={`kv-hotkey flex-1 ${recording ? 'recording' : ''} ${error ? 'error' : ''}`}
        >
          {recording ? (
            <span className="kv-hotkey-record-label animate-pulse">{recordingPlaceholder}</span>
          ) : value ? (
            <HotkeyDisplay hotkey={value} />
          ) : (
            <span className="text-[12px] text-neutral-400 dark:text-neutral-500">{placeholder}</span>
          )}
          {showClear && (
            <button
              type="button"
              onClick={onClear}
              title={clearLabel}
              aria-label={clearLabel}
              className="kv-hotkey-clear"
              data-tauri-drag-region="false"
            >
              <X size={12} strokeWidth={2.5} />
            </button>
          )}
        </div>
        <button
          type="button"
          onClick={onToggleRecording}
          className={`kv-btn ${recording ? 'accent' : ''}`}
          data-tauri-drag-region="false"
        >
          {recording ? recordingLabel : recordLabel}
        </button>
      </div>
      {error && (
        <p className="text-[11px] text-red-500 dark:text-red-400 leading-snug">{error}</p>
      )}
    </div>
  )
}

/**
 * 區塊標題 — 小號灰 uppercase + 左側 brand 細色條
 * 讓標題謙遜，把視覺重心交給卡片本身
 */
export function SectionTitle({ children, icon: Icon }: { children: ReactNode; icon?: LucideIcon }) {
  return (
    <div className="kv-group-title flex items-center gap-1.5">
      {Icon && <Icon size={12} className="text-neutral-500 dark:text-neutral-400" />}
      <span>{children}</span>
    </div>
  )
}

/**
 * 分段控制器標籤按鈕（輕量樣式）
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
