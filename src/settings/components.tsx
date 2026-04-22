import type { ReactNode } from 'react'
import { ExternalLink } from 'lucide-react'
import { formatHotkey, getPlatform } from './utils'

/**
 * 开关切换组件
 */
export function Toggle({ checked, onChange }: { checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      type="button"
      onClick={() => onChange(!checked)}
      className={`relative w-[34px] h-5 rounded-full transition-all duration-200 ease-in-out ${checked ? 'bg-neutral-900 dark:bg-white' : 'bg-neutral-200 dark:bg-neutral-700'}`}
      data-tauri-drag-region="false"
    >
      <span className={`absolute top-[2px] left-[2px] w-4 h-4 bg-white dark:bg-neutral-900 rounded-full shadow-sm transition-transform duration-200 ${checked ? 'translate-x-[14px]' : ''}`} />
    </button>
  )
}

/**
 * 下拉选择组件
 */
export function Select({ value, onChange, options, className = '' }: {
  value: string
  onChange: (v: string) => void
  options: { value: string; label: string }[]
  className?: string
}) {
  return (
    <div className="relative">
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className={`w-full appearance-none px-3 py-1.5 pr-8 rounded-lg border border-black/5 dark:border-white/5 bg-neutral-100 dark:bg-neutral-800 text-[13px] text-neutral-900 dark:text-neutral-100 focus:outline-none focus:ring-1 focus:ring-neutral-400 dark:focus:ring-neutral-500 transition-all ${className}`}
        data-tauri-drag-region="false"
      >
        {options.map(opt => <option key={opt.value} value={opt.value}>{opt.label}</option>)}
      </select>
      <div className="absolute right-2.5 top-1/2 -translate-y-1/2 pointer-events-none text-neutral-400">
        <svg width="10" height="6" viewBox="0 0 10 6" fill="none" xmlns="http://www.w3.org/2000/svg">
          <path d="M1 1L5 5L9 1" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </div>
    </div>
  )
}

/**
 * 文本输入组件
 */
export function Input({ value, onChange, type = 'text', placeholder = '', className = '', list, ...props }: {
  value: string
  onChange: (v: string) => void
  type?: string
  placeholder?: string
  className?: string
  list?: string
} & Omit<React.InputHTMLAttributes<HTMLInputElement>, 'value' | 'onChange'>) {
  return (
    <input
      type={type}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={placeholder}
      list={list}
      className={`w-full px-3 py-1.5 rounded-lg border border-black/5 dark:border-white/5 bg-neutral-100 dark:bg-neutral-800 text-[13px] font-mono text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 focus:outline-none focus:ring-1 focus:ring-neutral-400 dark:focus:ring-neutral-500 transition-all ${className}`}
      data-tauri-drag-region="false"
      {...props}
    />
  )
}

/**
 * 多行文本输入组件
 */
export function TextArea({ value, onChange, placeholder = '', rows = 2 }: {
  value: string
  onChange: (v: string) => void
  placeholder?: string
  rows?: number
}) {
  return (
    <textarea
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={placeholder}
      rows={rows}
      className="w-full px-3 py-2 rounded-lg border border-black/5 dark:border-white/5 bg-neutral-100 dark:bg-neutral-800 text-[13px] font-mono text-neutral-900 dark:text-neutral-100 placeholder-neutral-400 focus:outline-none focus:ring-1 focus:ring-neutral-400 dark:focus:ring-neutral-500 transition-all resize-none"
      data-tauri-drag-region="false"
    />
  )
}

/**
 * 标签组件
 */
export function Label({ children, className = '' }: { children: ReactNode; className?: string }) {
  return <label className={`block text-[12px] font-medium text-neutral-700 dark:text-neutral-200 mb-1 ${className}`}>{children}</label>
}

/**
 * 设置项行组件（左label右control）
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
          <p className="text-[11px] text-neutral-400 dark:text-neutral-500 mt-0.5">{description}</p>
        )}
      </div>
      <div className="shrink-0 flex items-center">{children}</div>
    </div>
  )
}

/**
 * 权限状态项组件（macOS）
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
      <div className="min-w-0">
        <p className="text-[13px] text-neutral-900 dark:text-neutral-100">{label}</p>
        <p className={`text-[11px] mt-0.5 ${granted ? 'text-emerald-600 dark:text-emerald-400' : 'text-amber-600 dark:text-amber-400'}`}>
          {granted ? grantedText : missingText}
        </p>
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
 * 键盘按键徽章组件
 */
export function KeyBadge({ children }: { children: ReactNode }) {
  return (
    <kbd className="inline-flex items-center justify-center min-w-[22px] h-[22px] px-1.5 rounded-[4px] bg-white dark:bg-neutral-800 border border-neutral-300 dark:border-neutral-600 text-[11px] font-medium text-neutral-700 dark:text-neutral-200 shadow-[0_1px_0_rgba(0,0,0,0.1)] dark:shadow-none">
      {children}
    </kbd>
  )
}

/**
 * 快捷键可视化展示组件
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
 * 快捷键输入组件（支持录制模式，显示为键盘徽章）
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
        className={`flex-1 flex items-center gap-1 min-h-[34px] px-2.5 rounded-lg border bg-neutral-100 dark:bg-neutral-800 transition-all ${
          recording
            ? 'border-amber-400/60 dark:border-amber-300/50 ring-1 ring-amber-400/40 dark:ring-amber-300/30'
            : 'border-black/5 dark:border-white/5'
        }`}
      >
        {recording ? (
          <span className="text-[12px] text-amber-500 animate-pulse">{recordingPlaceholder}</span>
        ) : value ? (
          <HotkeyDisplay hotkey={value} />
        ) : (
          <span className="text-[12px] text-neutral-400">{placeholder}</span>
        )}
      </div>
      <button
        type="button"
        onClick={onToggleRecording}
        className={`px-3 py-1.5 rounded-lg text-[11px] font-medium border transition-all ${
          recording
            ? 'border-amber-400/60 text-amber-600 dark:text-amber-300 bg-amber-50/70 dark:bg-amber-900/20'
            : 'border-black/10 dark:border-white/10 text-neutral-500 hover:text-neutral-900 dark:hover:text-neutral-200 hover:bg-black/5 dark:hover:bg-white/5'
        }`}
        data-tauri-drag-region="false"
      >
        {recording ? recordingLabel : recordLabel}
      </button>
    </div>
  )
}

/**
 * 默认提示词展示组件
 */
export function DefaultPrompt({ label, content }: { label: string; content: string }) {
  return (
    <div className="mt-2 rounded-lg border border-black/5 dark:border-white/5 bg-neutral-50/80 dark:bg-neutral-800/40 px-3 py-2">
      <div className="text-[10px] font-medium text-neutral-400 dark:text-neutral-500 mb-1">{label}</div>
      <pre className="whitespace-pre-wrap text-[11px] text-neutral-600 dark:text-neutral-300 font-mono">
        {content.trim()}
      </pre>
    </div>
  )
}

/**
 * 区块标题组件（灰色小字，放在卡片外）
 */
export function SectionTitle({ children }: { children: ReactNode }) {
  return (
    <div className="px-1 mb-1.5">
      <span className="text-[11px] font-semibold text-neutral-500 dark:text-neutral-400 uppercase tracking-wide">
        {children}
      </span>
    </div>
  )
}

/**
 * 分段控制器标签按钮（无图标，轻量样式）
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
