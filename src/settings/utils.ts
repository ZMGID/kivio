import { type ModelProvider } from '../api/tauri'
import { i18n, type Lang } from './i18n'

export type Platform = 'macos' | 'windows' | 'linux'

export type SelectOption = {
  value: string
  label: string
  title?: string
}

// 修飾鍵集合（錄製快捷鍵時忽略）
const modifierKeys = new Set(['Shift', 'Meta', 'Control', 'Alt', 'AltGraph'])

// 鍵盤按鍵別名對映
const keyAliasMap: Record<string, string> = {
  Escape: 'Esc',
  ' ': 'Space',
  Spacebar: 'Space',
  ArrowUp: 'Up',
  ArrowDown: 'Down',
  ArrowLeft: 'Left',
  ArrowRight: 'Right',
}

/**
 * 從鍵盤 code 提取字母/數字鍵值
 */
const normalizeKeyFromCode = (code: string) => {
  if (code.startsWith('Key')) return code.slice(3)
  if (code.startsWith('Digit')) return code.slice(5)
  return ''
}

/**
 * 將鍵盤事件轉換為快捷鍵字串
 */
export const normalizeHotkeyKey = (event: KeyboardEvent) => {
  const { key, code } = event
  if (!key) return ''
  if (modifierKeys.has(key)) return ''
  if (/^F\d{1,2}$/.test(key)) return key.toUpperCase()
  const alias = keyAliasMap[key]
  if (alias) return alias
  const fromCode = normalizeKeyFromCode(code)
  if (fromCode) return fromCode.toUpperCase()
  if (key === 'Dead' || key === 'Process') return ''
  if (key.length === 1 && key !== '+') return key.toUpperCase()
  return ''
}

/**
 * 建置完整的快捷鍵字串（如 CommandOrControl+Alt+T）
 */
export const buildHotkey = (event: KeyboardEvent) => {
  const key = normalizeHotkeyKey(event)
  if (!key) return ''
  const parts: string[] = []
  if (event.metaKey || event.ctrlKey) parts.push('CommandOrControl')
  if (event.altKey || event.getModifierState('AltGraph')) parts.push('Alt')
  if (event.shiftKey) parts.push('Shift')
  parts.push(key)
  return parts.join('+')
}

/**
 * 平臺偵測（用於快捷鍵視覺化）
 */
export const getPlatform = (): Platform => {
  if (navigator.platform.startsWith('Mac')) return 'macos'
  if (navigator.platform.startsWith('Win')) return 'windows'
  return 'linux'
}

/**
 * 將快捷鍵字串解析為視覺化按鍵陣列
 */
export const formatHotkey = (hotkey: string, platform: 'macos' | 'windows' | 'linux'): string[] => {
  const parts = hotkey.split('+')
  return parts.map((part) => {
    switch (part) {
      case 'CommandOrControl':
        return platform === 'macos' ? '⌘' : 'Ctrl'
      case 'Command':
        return '⌘'
      case 'Control':
        return 'Ctrl'
      case 'Alt':
        return platform === 'macos' ? '⌥' : 'Alt'
      case 'Shift':
        return platform === 'macos' ? '⇧' : 'Shift'
      case 'Escape':
        return 'Esc'
      case 'Space':
        return 'Space'
      case 'ArrowUp':
        return '↑'
      case 'ArrowDown':
        return '↓'
      case 'ArrowLeft':
        return '←'
      case 'ArrowRight':
        return '→'
      default:
        return part.length === 1 ? part.toUpperCase() : part
    }
  })
}

export const modelPairValue = (providerId: string, model: string) =>
  JSON.stringify([providerId, model])

export const parseModelPairValue = (value: string): [string, string] => {
  try {
    const parsed = JSON.parse(value)
    if (Array.isArray(parsed) && parsed.length >= 2) {
      return [String(parsed[0] || ''), String(parsed[1] || '')]
    }
  } catch {
    // 相容舊版本用 "provider:model" 拼接的下拉值。
  }
  const separator = value.indexOf(':')
  if (separator < 0) return [value, '']
  return [value.slice(0, separator), value.slice(separator + 1)]
}

export const isProviderEnabled = (provider: ModelProvider) => provider.enabled !== false

export const buildModelPairOptions = (providers: ModelProvider[]): SelectOption[] =>
  providers
    .filter(provider => isProviderEnabled(provider))
    .flatMap(provider =>
      provider.enabledModels.map(model => ({
        value: modelPairValue(provider.id, model),
        label: `${provider.name} - ${model}`,
        title: `${provider.name} - ${model}`,
      })),
    )

/**
 * 與 JSON.stringify 等價但對物件 key 做遞迴排序,用於 dirty diff:
 * 後端 sanitize 與前端 spread 都可能改變欄位順序,普通 JSON.stringify 會
 * 把"語義無差異、欄位順序不同"誤判為髒。陣列順序保留(陣列順序在 settings
 * 裡語義上是有意義的,如 apiKeys 的 primary/backup 順序)。
 */
export const stableStringify = (value: unknown): string =>
  JSON.stringify(value, (_key, v) => {
    if (v && typeof v === 'object' && !Array.isArray(v)) {
      const sorted: Record<string, unknown> = {}
      for (const k of Object.keys(v as Record<string, unknown>).sort()) {
        sorted[k] = (v as Record<string, unknown>)[k]
      }
      return sorted
    }
    return v
  })

type HotkeyErrorPayload = {
  kind: 'conflict' | 'duplicate' | 'empty' | 'other'
  scope: 'translator' | 'screenshot' | 'screenshot_text' | 'lens'
  hotkey: string
  raw?: string
}

const SCOPE_KEY: Record<HotkeyErrorPayload['scope'], keyof typeof i18n.zh> = {
  translator: 'hotkeyScopeTranslator',
  screenshot: 'hotkeyScopeScreenshot',
  screenshot_text: 'hotkeyScopeScreenshotText',
  lens: 'hotkeyScopeLens',
}

const KIND_KEY: Record<HotkeyErrorPayload['kind'], keyof typeof i18n.zh> = {
  conflict: 'hotkeyErrorConflict',
  duplicate: 'hotkeyErrorDuplicate',
  empty: 'hotkeyErrorEmpty',
  other: 'hotkeyErrorOther',
}

/**
 * 把後端 register_hotkeys 拋出的 JSON 錯誤陣列翻譯成使用者語言的可讀訊息。
 * 解析失敗(普通字串錯誤)時原樣返回,保證所有非熱鍵錯誤也能正常顯示。
 */
export const formatHotkeyError = (raw: string, lang: Lang): string => {
  let parsed: unknown
  try {
    parsed = JSON.parse(raw)
  } catch {
    return raw
  }
  if (!Array.isArray(parsed) || parsed.length === 0) return raw
  const table = i18n[lang]
  const messages: string[] = []
  for (const item of parsed) {
    if (
      !item ||
      typeof item !== 'object' ||
      !(SCOPE_KEY as Record<string, unknown>)[(item as HotkeyErrorPayload).scope] ||
      !(KIND_KEY as Record<string, unknown>)[(item as HotkeyErrorPayload).kind]
    ) {
      return raw
    }
    const e = item as HotkeyErrorPayload
    const scope = table[SCOPE_KEY[e.scope]]
    const template = table[KIND_KEY[e.kind]]
    messages.push(
      template
        .replace('{scope}', scope)
        .replace('{hotkey}', e.hotkey)
        .replace('{raw}', e.raw ?? ''),
    )
  }
  return messages.join(' / ')
}
