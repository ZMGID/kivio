import { type Lang } from './settings/i18n'

export type ThemeColorId = 'neutral' | 'warm' | 'cool'

export type ThemeColorPreset = {
  id: ThemeColorId
  hex: string
  labels: Record<Lang, string>
}

export const DEFAULT_THEME_COLOR: ThemeColorId = 'neutral'

export const THEME_COLOR_PRESETS: ThemeColorPreset[] = [
  { id: 'neutral', hex: '#FFFFFF', labels: { zh: '中性', 'zh-TW': '中性', en: 'Neutral' } },
  { id: 'warm', hex: '#FAF9F5', labels: { zh: '暖白', 'zh-TW': '暖白', en: 'Warm' } },
  { id: 'cool', hex: '#F6F8FB', labels: { zh: '冷白', 'zh-TW': '冷白', en: 'Cool' } },
]

export function normalizeThemeColorId(value: string | null | undefined): ThemeColorId {
  return THEME_COLOR_PRESETS.some((preset) => preset.id === value)
    ? value as ThemeColorId
    : DEFAULT_THEME_COLOR
}
