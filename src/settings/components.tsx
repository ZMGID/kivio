import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from 'react'
import { createPortal } from 'react-dom'
import { Check, ChevronDown, ExternalLink, X } from 'lucide-react'
import { formatHotkey, getPlatform, type SelectOption } from './utils'
import { Button, IconButton } from '../components/Button'
import { copyToClipboard, readClipboardText } from '../utils/clipboard'
import { TextEditContextMenu } from './TextEditContextMenu'

const MENU_GAP = 6
const MENU_MARGIN = 8
const MENU_MAX_HEIGHT = 260

function useSelectMenuRect(
  open: boolean,
  value: string,
  optionsLength: number,
  triggerRef: RefObject<HTMLElement | null>,
  minWidth = 0,
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
    const width = minWidth ? Math.min(Math.max(rect.width, minWidth), window.innerWidth - MENU_MARGIN * 2) : rect.width
    const left = minWidth ? Math.max(MENU_MARGIN, Math.min(rect.left, window.innerWidth - width - MENU_MARGIN)) : rect.left
    const viewportH = window.innerHeight
    const spaceBelow = viewportH - rect.bottom - MENU_GAP - MENU_MARGIN
    const spaceAbove = rect.top - MENU_GAP - MENU_MARGIN
    // 默认向下展开；下方空间不足且上方更宽裕时向上翻转。
    const flipUp = spaceBelow < MENU_MAX_HEIGHT && spaceAbove > spaceBelow
    const available = Math.max(flipUp ? spaceAbove : spaceBelow, 0)
    const maxHeight = Math.max(Math.min(MENU_MAX_HEIGHT, available), 80)
    if (flipUp) {
      // 用 bottom 定位让菜单底边贴着按钮向上生长，避免 top 计算后恒等于 MENU_MARGIN 导致飞到窗口顶部。
      setMenuRect({ left, bottom: viewportH - rect.top + MENU_GAP, width, maxHeight })
    } else {
      setMenuRect({ left, top: rect.bottom + MENU_GAP, width, maxHeight })
    }
  }, [triggerRef, minWidth])

  useLayoutEffect(() => {
    if (open) updateMenuRect()
  }, [open, value, optionsLength, updateMenuRect])

  return { menuRect, updateMenuRect }
}

/**
 * 开关切换组件 — on 态用 brand 蓝，slider 加双层阴影
 */
export function Toggle({ checked, onChange, disabled, ariaLabel }: { checked: boolean; onChange: (v: boolean) => void; disabled?: boolean; ariaLabel?: string }) {
  return (
    <button
      type="button"
      onClick={() => !disabled && onChange(!checked)}
      disabled={disabled}
      aria-label={ariaLabel}
      role="switch"
      aria-checked={checked}
      className={`kv-toggle ${checked ? 'on' : ''}`}
      data-tauri-drag-region="false"
    />
  )
}

/** 下拉菜单开合 + 点外关闭 + Esc，Select / SuggestInput 共用。 */
function useSelectMenuOpen(
  open: boolean,
  setOpen: (open: boolean) => void,
  triggerRef: RefObject<HTMLElement | null>,
  menuRef: RefObject<HTMLElement | null>,
  updateMenuRect: () => void,
) {
  useEffect(() => {
    if (!open) return
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target as Node
      if (triggerRef.current?.contains(target) || menuRef.current?.contains(target)) return
      setOpen(false)
    }
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault()
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
  }, [open, setOpen, triggerRef, menuRef, updateMenuRect])
}

/** 与 Select 同款的选项菜单（portal）。 */
function SelectMenuPortal({
  open,
  triggerRef,
  menuRef,
  menuRect,
  options,
  value,
  onPick,
}: {
  open: boolean
  triggerRef: RefObject<HTMLElement | null>
  menuRef: RefObject<HTMLDivElement | null>
  menuRect: { left: number; top?: number; bottom?: number; width: number; maxHeight: number }
  options: SelectOption[]
  value: string
  onPick: (value: string) => void
}) {
  if (!open) return null
  return createPortal(
    <div
      ref={menuRef as React.RefObject<HTMLDivElement>}
      role="listbox"
      className="kv-select-menu fixed z-[1000] overflow-y-auto custom-scrollbar"
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
            onClick={() => onPick(opt.value)}
            title={opt.title || opt.label}
            className={`kv-select-option ${active ? 'is-active' : ''}`}
            data-tauri-drag-region="false"
          >
            <Check className="kv-select-option-check" strokeWidth={2.5} aria-hidden />
            <span className="min-w-0 flex-1 truncate">{opt.label}</span>
          </button>
        )
      })}
    </div>,
    // Native dialogs occupy the top layer; their menus must stay in that layer.
    triggerRef.current?.closest('dialog[open]') ?? document.body,
  )
}

/**
 * 下拉选择 — 自绘菜单，避免 macOS 原生 select 的系统高亮/勾选反馈和受控状态不同步。
 */
export function Select({ value, onChange, options, className = '', disabled: disabledProp = false, title, ariaLabel, triggerIcon, triggerLabel }: {
  value: string
  onChange: (v: string) => void
  options: SelectOption[]
  className?: string
  disabled?: boolean
  /** 紧凑工具栏使用图标触发器，选项菜单仍显示完整名称。 */
  triggerIcon?: ReactNode
  /** 图标工具栏同时显示当前范围时使用；菜单保留完整选项名称。 */
  triggerLabel?: string
  /** 覆盖触发按钮的原生 tooltip（默认显示当前选中项）。 */
  title?: string
  /** 无可关联原生 label 时，为触发按钮提供可访问名称。 */
  ariaLabel?: string
}) {
  const [open, setOpen] = useState(false)
  const triggerRef = useRef<HTMLButtonElement | null>(null)
  const menuRef = useRef<HTMLDivElement | null>(null)
  const selected = options.find(opt => opt.value === value)
  const displayLabel = selected?.label || value
  const displayTitle = selected?.title || displayLabel
  const disabled = disabledProp || options.length === 0
  const { menuRect, updateMenuRect } = useSelectMenuRect(open, value, options.length, triggerRef, triggerIcon || triggerLabel !== undefined ? 200 : 0)
  useSelectMenuOpen(open, setOpen, triggerRef, menuRef, updateMenuRect)

  const triggerProps = {
    ref: (node: HTMLButtonElement | null) => {
      triggerRef.current = node
      // Recheck after each commit: portalled options bypass a disabled fieldset.
      if (open && node?.matches(':disabled')) setOpen(false)
    },
    disabled,
    onClick: (event: React.MouseEvent<HTMLButtonElement>) => {
      if (!event.currentTarget.matches(':disabled')) setOpen(v => !v)
    },
    onKeyDown: (event: React.KeyboardEvent<HTMLButtonElement>) => {
      if (event.currentTarget.matches(':disabled')) return
      if (event.key === 'ArrowDown' || event.key === 'Enter' || event.key === ' ') {
        event.preventDefault()
        setOpen(true)
      }
    },
    'aria-haspopup': 'listbox' as const,
    'aria-expanded': open,
    title: title ?? displayTitle,
    'data-tauri-drag-region': 'false',
  }

  return (
    <div className={`relative ${className}`}>
      {triggerLabel !== undefined ? (
        <Button {...triggerProps} variant="ghost" className="max-w-full min-w-0" aria-label={ariaLabel}>
          {triggerIcon && <span className="flex shrink-0 items-center">{triggerIcon}</span>}
          <span className="min-w-0 truncate">{triggerLabel}</span>
          <ChevronDown size={14} className={`shrink-0 transition-transform ${open ? 'rotate-180' : ''}`} aria-hidden="true" />
        </Button>
      ) : triggerIcon ? (
        <IconButton {...triggerProps} size="sm" label={ariaLabel ?? displayLabel}>
          {triggerIcon}
        </IconButton>
      ) : (
        <button
          {...triggerProps}
          type="button"
          className="kv-select kv-select-button relative h-[30px] w-full min-w-0 max-w-none text-left disabled:cursor-not-allowed disabled:opacity-50"
          aria-label={ariaLabel}
        >
          <span className="block truncate">{displayLabel}</span>
          <ChevronDown
            size={14}
            strokeWidth={2.25}
            className={`absolute right-2.5 top-1/2 -translate-y-1/2 text-neutral-400 dark:text-neutral-500 transition-transform ${open ? 'rotate-180' : ''}`}
          />
        </button>
      )}

      <SelectMenuPortal
        open={open}
        triggerRef={triggerRef}
        menuRef={menuRef}
        menuRect={menuRect}
        options={options}
        value={value}
        onPick={(next) => {
          if (triggerRef.current?.matches(':disabled')) return
          onChange(next)
          setOpen(false)
          triggerRef.current?.focus()
        }}
      />
    </div>
  )
}

/**
 * 可输入 + 右侧下拉箭头（获取模型列表后用）。
 * 菜单与 Select 共用同一套 kv-select-menu / option 样式，不另做一套下拉。
 */
export function SuggestInput({
  value,
  onChange,
  options,
  placeholder = '',
  className = '',
  mono = false,
  disabled = false,
  ariaLabel,
}: {
  value: string
  onChange: (v: string) => void
  options: SelectOption[]
  placeholder?: string
  className?: string
  mono?: boolean
  disabled?: boolean
  ariaLabel?: string
}) {
  const [open, setOpen] = useState(false)
  const rootRef = useRef<HTMLDivElement | null>(null)
  const triggerRef = useRef<HTMLButtonElement | null>(null)
  const menuRef = useRef<HTMLDivElement | null>(null)
  const canSuggest = options.length > 0 && !disabled
  // 菜单宽度对齐整行（输入框 + 箭头），锚在 root 上
  const { menuRect, updateMenuRect } = useSelectMenuRect(open, value, options.length, rootRef)
  useSelectMenuOpen(open, setOpen, rootRef, menuRef, updateMenuRect)

  return (
    <div ref={rootRef} className={`kv-suggest-input ${className}`.trim()}>
      <input
        type="text"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        disabled={disabled}
        spellCheck={false}
        aria-label={ariaLabel}
        className={`kv-input kv-suggest-input-field ${mono ? 'mono' : ''}`}
        data-tauri-drag-region="false"
      />
      {canSuggest && (
        <button
          ref={triggerRef}
          type="button"
          className="kv-suggest-input-toggle"
          aria-haspopup="listbox"
          aria-expanded={open}
          aria-label={ariaLabel ? `${ariaLabel} list` : 'Open suggestions'}
          onClick={() => setOpen((v) => !v)}
          data-tauri-drag-region="false"
        >
          <ChevronDown
            size={14}
            strokeWidth={2.25}
            className={`transition-transform ${open ? 'rotate-180' : ''}`}
          />
        </button>
      )}
      <SelectMenuPortal
        open={open && canSuggest}
        triggerRef={triggerRef}
        menuRef={menuRef}
        menuRect={menuRect}
        options={options}
        value={value}
        onPick={(next) => {
          onChange(next)
          setOpen(false)
          triggerRef.current?.focus()
        }}
      />
    </div>
  )
}

/**
 * 文本输入 — 默认 sans，需要等宽时调用方自行加 font-mono
 */
export function Input({ value, onChange, type = 'text', placeholder = '', className = '', mono = false, ...props }: {
  value: string
  onChange: (v: string) => void
  type?: string
  placeholder?: string
  className?: string
  /** 启用 font-mono（仅 baseUrl/apiKey/model 名等代码型字段使用） */
  mono?: boolean
} & Omit<React.InputHTMLAttributes<HTMLInputElement>, 'value' | 'onChange'>) {
  return (
    <input
      type={type}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={placeholder}
      className={`kv-input w-full ${mono ? 'mono' : ''} ${className}`}
      data-tauri-drag-region="false"
      {...props}
    />
  )
}

/**
 * 多行文本输入 — 默认 sans
 */
export function TextArea({
  value,
  onChange,
  placeholder = '',
  rows = 2,
  mono = false,
  className = '',
  onContextMenu,
  ...props
}: {
  value: string
  onChange: (v: string) => void
  placeholder?: string
  rows?: number
  mono?: boolean
} & Omit<React.TextareaHTMLAttributes<HTMLTextAreaElement>, 'value' | 'onChange'>) {
  const ref = useRef<HTMLTextAreaElement | null>(null)
  const caretRef = useRef<{ start: number; end: number } | null>(null)
  const [menu, setMenu] = useState<{ left: number; top: number; start: number; end: number } | null>(null)

  useLayoutEffect(() => {
    const el = ref.current
    const caret = caretRef.current
    if (!el || !caret) return
    caretRef.current = null
    el.focus()
    el.setSelectionRange(caret.start, caret.end)
  }, [value])

  const applyEdit = (next: string, start: number, end: number) => {
    const el = ref.current
    if (!el || el.matches(':disabled') || el.readOnly) return
    caretRef.current = { start, end }
    onChange(next)
  }

  return (
    <>
      <textarea
        ref={node => {
          ref.current = node
          // Ancestor fieldset changes are visible only after the DOM commit.
          if (menu && node?.matches(':disabled')) setMenu(null)
        }}
        {...props}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        rows={rows}
        className={`kv-textarea custom-scrollbar w-full ${mono ? 'mono' : ''} ${className}`}
        data-tauri-drag-region="false"
        onContextMenu={(event) => {
          onContextMenu?.(event)
          if (event.defaultPrevented || event.currentTarget.matches(':disabled')) return
          event.preventDefault()
          event.stopPropagation()
          const el = event.currentTarget
          setMenu({
            left: event.clientX,
            top: event.clientY,
            start: el.selectionStart,
            end: el.selectionEnd,
          })
        }}
      />
      {menu && (
        <TextEditContextMenu
          anchor={{ left: menu.left, top: menu.top }}
          portalTarget={ref.current?.closest('dialog[open]') ?? document.body}
          readOnly={props.readOnly || ref.current?.matches(':disabled')}
          hasSelection={menu.end > menu.start}
          onCut={() => {
            const { start, end } = menu
            const selected = value.slice(start, end)
            if (!selected) return
            void copyToClipboard(selected)
            applyEdit(`${value.slice(0, start)}${value.slice(end)}`, start, start)
          }}
          onCopy={() => {
            const selected = value.slice(menu.start, menu.end)
            if (selected) void copyToClipboard(selected)
          }}
          onPaste={() => {
            const { start, end } = menu
            void readClipboardText().then((text) => {
              applyEdit(`${value.slice(0, start)}${text}${value.slice(end)}`, start + text.length, start + text.length)
            })
          }}
          onSelectAll={() => {
            const el = ref.current
            if (!el) return
            el.focus()
            el.setSelectionRange(0, value.length)
          }}
          onClose={() => setMenu(null)}
        />
      )}
    </>
  )
}

/**
 * 字段标签
 */
export function Label({ children, className = '' }: { children: ReactNode; className?: string }) {
  return (
    <label className={`kv-field-label ${className}`}>
      {children}
    </label>
  )
}

/**
 * 设置项行（左 label + 可选 description，右控件）
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

/**
 * 纵向字段块：标签 + 说明在上，控件在下。
 * 原在 SettingsShell 内部，抽 tab 组件后需跨模块共享，移到这里。
 */
export function FieldBlock({
  label,
  description,
  children,
  className = '',
  htmlFor,
}: {
  label: ReactNode
  description?: string
  children: ReactNode
  className?: string
  htmlFor?: string
}) {
  return (
    <div className={`py-2 ${className}`}>
      <div className="mb-2">
        {htmlFor
          ? <label className="kv-row-label" htmlFor={htmlFor}>{label}</label>
          : <div className="kv-row-label">{label}</div>}
        {description && <p className="kv-row-desc">{description}</p>}
      </div>
      {children}
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

/** A labelled range slider row with a value badge and min/max ticks. */
export function SliderField({ label, value, min, max, step = 1, onChange, hint, suffix = '' }: {
  label: ReactNode
  value: number
  min: number
  max: number
  step?: number
  onChange: (v: number) => void
  hint?: string
  suffix?: string
}) {
  return (
    <div className="kv-row-stack">
      <div className="flex items-center justify-between gap-3">
        <span className="kv-row-label">{label}</span>
        <span className="rounded-md border border-zinc-200 bg-neutral-50 px-2 py-0.5 font-mono text-xs text-zinc-700">
          {value}{suffix}
        </span>
      </div>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
        className="mt-2 w-full"
        style={{ accentColor: 'var(--accent)' }}
      />
      <div className="flex justify-between text-[11px] text-zinc-400">
        <span>{min}</span>
        <span>{max}</span>
      </div>
      {hint && <p className="kv-row-desc mt-1.5">{hint}</p>}
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
        <Button
          size="sm"
          onClick={onOpen}
          data-tauri-drag-region="false"
        >
          <ExternalLink size={11} />
          {actionLabel}
        </Button>
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
      className="kv-kbd"
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
 * 快捷键输入(含录制态)
 * onClear / clearLabel: 提供时,值非空且非录制态会渲染 X 按钮以清空(给"想关掉某个功能的热键"留出口);留空则不显示
 * error: 客户端冲突等校验消息,以红色小字显示在输入框下方
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
  inline = false,
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
  inline?: boolean
}) {
  const showClear = !!onClear && !!value && !recording
  return (
    <div className={`space-y-1 ${inline ? 'flex flex-col items-end' : ''}`}>
      <div className="flex items-center gap-2">
        <div
          className={`kv-hotkey ${inline ? '' : 'flex-1'} ${recording ? 'recording' : ''} ${error ? 'error' : ''}`}
          title={!value && !recording ? placeholder : undefined}
        >
          {recording ? (
            <span className="kv-hotkey-record-label animate-pulse">{recordingPlaceholder}</span>
          ) : value ? (
            <HotkeyDisplay hotkey={value} />
          ) : (
            <span className="min-w-0 truncate text-[12px] leading-[19px] text-neutral-400 dark:text-neutral-500">
              {placeholder}
            </span>
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
