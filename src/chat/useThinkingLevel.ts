import { useEffect, useMemo, useState } from 'react'
import { api, type ThinkingCapabilities } from '../api/tauri'
import { useModelDatabase } from '../data/useModelDatabase'
import type { ThinkingLevel } from './types'

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

export interface ThinkingLevelOption {
  value: ThinkingLevel
  label: string
  disabled: boolean
}

export interface ThinkingLevelState {
  /** 当前模型没有可调的思考档位：不显示控件。 */
  hidden: boolean
  /** 能力尚未加载：控件只读，避免用旧档位回写。 */
  loaded: boolean
  effective: ThinkingLevel
  effectiveLabel: string
  options: ThinkingLevelOption[]
  notice: string | null
}

/**
 * 内置运行时的思考档位：按模型能力列出可选档，并把不在列表里的旧档收敛后落盘。
 * `enabled=false`（外部 CLI）时不查能力、不回写。收敛必须常驻，不能只在菜单打开时运行。
 */
export function useThinkingLevel({
  enabled,
  value,
  currentProviderId,
  currentModel,
  onChange,
  lang,
}: {
  enabled: boolean
  value: ThinkingLevel | null
  currentProviderId: string
  currentModel: string
  onChange: (level: ThinkingLevel) => void
  lang: 'zh' | 'en'
}): ThinkingLevelState {
  const { revision } = useModelDatabase()
  const capabilityKey = JSON.stringify([currentProviderId, currentModel, revision])
  const [snapshot, setSnapshot] = useState<{ key: string; capabilities: ThinkingCapabilities } | null>(null)
  const loaded = snapshot?.key === capabilityKey
  const { levels, offMode } = loaded
    ? snapshot.capabilities
    : { levels: FALLBACK_LEVELS, offMode: 'unknown' as const }

  useEffect(() => {
    if (!enabled) return
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
  }, [enabled, capabilityKey, currentProviderId, currentModel])

  // null（未显式设置）按默认档处理；存的档若不在当前模型的支持列表里（换模型最常见：
  // 在 gpt-5.6 选了 xhigh 再切回 gpt-5）就地收敛，UI 永远高亮一个真实存在的等级。
  const effective = useMemo<ThinkingLevel>(() => {
    const current = value ?? DEFAULT_LEVEL
    if (current === 'off' || levels.length === 0 || levels.includes(current)) return current
    const fixed = levels.includes(DEFAULT_LEVEL) ? DEFAULT_LEVEL : levels[levels.length - 1]
    return fixed as ThinkingLevel
  }, [value, levels])

  // 收敛结果要落盘，否则按钮显示 High、请求却仍按存着的 xhigh 发出去，直接吃 provider 的 400。
  useEffect(() => {
    if (enabled && loaded && levels.length > 0 && effective !== (value ?? DEFAULT_LEVEL)) onChange(effective)
  }, [enabled, effective, loaded, value, levels, onChange])

  const options = useMemo<ThinkingLevelOption[]>(
    () => [
      ...(offMode !== 'unsupported' || value === 'off'
        ? [{ value: 'off' as const, label: offMode === 'upfront_only' ? 'Off (up-front)' : LABELS.off, disabled: offMode === 'unsupported' }]
        : []),
      ...(levels.length === 0 && (offMode === 'supported' || value === 'off')
        ? [{ value: 'high' as const, label: 'On', disabled: false }]
        : []),
      ...levels.map((level) => ({ value: level as ThinkingLevel, label: LABELS[level] ?? level, disabled: false })),
    ],
    [levels, offMode, value],
  )

  // Toggle-only models remain editable; stale Off choices must remain visible.
  const hidden = !enabled
    || offMode === 'not_applicable'
    || (levels.length === 0 && offMode !== 'supported' && value !== 'off')

  const notice = offMode === 'upfront_only'
    ? (lang === 'zh' ? 'Off 仅关闭回答前的思考，工具调用之间仍可能思考。' : 'Off disables up-front thinking; thinking between tool calls may continue.')
    : offMode === 'unknown'
      ? (lang === 'zh' ? '尚未确认此模型是否支持关闭思考。' : 'Thinking-off support has not been verified for this model.')
      : null
  const effectiveLabel = effective === 'off' && offMode === 'unsupported'
    ? (lang === 'zh' ? 'Off 不可用' : 'Off unavailable')
    : options.find(option => option.value === effective)?.label ?? LABELS[effective] ?? effective

  return { hidden, loaded, effective, effectiveLabel, options, notice }
}
