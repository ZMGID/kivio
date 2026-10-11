import { memo, useEffect, useMemo, useRef, useState } from 'react'
import { Brain, Check, ChevronDown } from 'lucide-react'
import { api, type ThinkingCapabilities } from '../api/tauri'
import { useT } from '../components/i18n'
import { chatTitlebarPillButtonClass } from './platform'
import type { ThinkingLevel } from './types'
import { usePopoverMenu } from './usePopoverMenu'

interface ThinkingLevelSelectorProps {
  /** 当前等级；null = 未显式设置，按默认档 DEFAULT_LEVEL 处理。 */
  value: ThinkingLevel | null
  currentProviderId: string
  currentModel: string
  onChange: (level: ThinkingLevel) => void
}

// 固定项 + 各等级标签（英文，跨语言更通用）。具体显示哪些等级由后端按模型库决定。
const LABELS: Record<string, string> = {
  off: 'Off',
  low: 'Low',
  medium: 'Medium',
  high: 'High',
  xhigh: 'XHigh',
  max: 'Max',
}
// 未显式选等级时的默认档（与后端 resolve_thinking 保持一致）。
const DEFAULT_LEVEL: ThinkingLevel = 'high'
// 未取到模型能力时的安全兜底（全模型通用子集）。
const FALLBACK_LEVELS = ['low', 'medium', 'high']

function labelFor(value: ThinkingLevel): string {
  return LABELS[value] ?? value
}

function ThinkingLevelSelectorBase({
  value,
  currentProviderId,
  currentModel,
  onChange,
}: ThinkingLevelSelectorProps) {
  const t = useT()
  const [open, setOpen] = useState(false)
  const menuRef = useRef<HTMLDivElement>(null)
  usePopoverMenu(open, () => setOpen(false), menuRef)
  const capabilityKey = JSON.stringify([currentProviderId, currentModel])
  const [snapshot, setSnapshot] = useState<{ key: string; capabilities: ThinkingCapabilities } | null>(null)
  const levelsLoaded = snapshot?.key === capabilityKey
  const { levels, offMode } = levelsLoaded
    ? snapshot.capabilities
    : { levels: FALLBACK_LEVELS, offMode: 'unknown' as const }

  useEffect(() => {
    let alive = true
    void (async () => {
      if (!currentModel) {
        setSnapshot({ key: capabilityKey, capabilities: { levels: FALLBACK_LEVELS, offMode: 'unknown' } })
        return
      }
      try {
        const capabilities = await api.thinkingCapabilitiesForModel(currentModel, currentProviderId)
        if (alive) setSnapshot({ key: capabilityKey, capabilities })
      } catch {
        // Failed/stale capability lookups must never rewrite the saved choice.
      }
    })()
    return () => { alive = false }
  }, [capabilityKey, currentProviderId, currentModel])

  // Saved values are reconciled only after the current model's capabilities load.
  const effective = useMemo<ThinkingLevel>(() => {
    const current = value === 'off' && offMode === 'unsupported' ? DEFAULT_LEVEL : value ?? DEFAULT_LEVEL
    if (current === 'off' || levels.length === 0 || levels.includes(current)) return current
    return (levels.includes(DEFAULT_LEVEL) ? DEFAULT_LEVEL : levels[levels.length - 1]) as ThinkingLevel
  }, [value, levels, offMode])

  // 收敛结果要落盘，否则按钮显示 High、请求却仍按存着的 xhigh 发出去，直接吃 provider 的 400。
  useEffect(() => {
    if (levelsLoaded && effective !== (value ?? DEFAULT_LEVEL)) onChange(effective)
  }, [effective, levelsLoaded, value, onChange])

  const options = useMemo<Array<{ value: ThinkingLevel; label: string }>>(
    () => [
      ...(offMode !== 'unsupported' ? [{ value: 'off' as const, label: offMode === 'upfront_only' ? 'Off (up-front)' : LABELS.off }] : []),
      ...(levels.length === 0 && offMode === 'supported' ? [{ value: 'high' as const, label: 'On' }] : []),
      ...levels.map((l) => ({ value: l as ThinkingLevel, label: LABELS[l] ?? l })),
    ],
    [levels, offMode],
  )

  if (offMode === 'not_applicable' || (levels.length === 0 && offMode !== 'supported')) return null

  const effectiveLabel = options.find(option => option.value === effective)?.label ?? labelFor(effective)

  return (
    <div className="relative max-w-full min-w-0" data-tauri-drag-region="false">
      <button
        type="button"
        disabled={!levelsLoaded}
        aria-busy={!levelsLoaded}
        onClick={() => setOpen(!open)}
        aria-haspopup="menu"
        aria-expanded={open}
        className={`${chatTitlebarPillButtonClass} max-w-full min-w-0`}
        title={t.chatThinkingLevel.replace('{level}', effectiveLabel)}
        aria-label={t.chatThinkingLevel.replace('{level}', effectiveLabel)}
      >
        <Brain size={15} className="shrink-0 text-neutral-500 dark:text-neutral-400" />
        <span className="chat-thinking-level-label max-w-[64px] truncate font-medium text-neutral-800">
          {effectiveLabel}
        </span>
        <ChevronDown
          size={15}
          className={`shrink-0 text-neutral-400 transition-transform ${open ? 'rotate-180' : ''}`}
        />
      </button>

      {open && (
        <>
          <div className="fixed inset-0 z-10" onClick={() => setOpen(false)} aria-hidden />
          <div role="menu" ref={menuRef} className="chat-model-selector-menu chat-motion-popover absolute left-0 top-full z-20 mt-2 min-w-[160px] overflow-y-auto kv-menu">
            {options.map((opt) => {
              const active = opt.value === effective
              return (
                <button
                  key={opt.value}
                  role="menuitemradio"
                  aria-checked={active}
                  disabled={!levelsLoaded}
                  type="button"
                  onClick={() => {
                    onChange(opt.value)
                    setOpen(false)
                  }}
                  className={`kv-menu-row justify-between transition-colors ${
                    active
                      ? 'bg-neutral-100 font-medium text-neutral-900'
                      : 'text-neutral-700 hover:bg-neutral-50'
                  }`}
                >
                  <span className="min-w-0 truncate">{opt.label}</span>
                  {active && <Check size={15} className="shrink-0 text-neutral-500" />}
                </button>
              )
            })}
          </div>
        </>
      )}
    </div>
  )
}

// memo：顶栏选择器，仅在 props 变化时重渲。
export const ThinkingLevelSelector = memo(ThinkingLevelSelectorBase)
