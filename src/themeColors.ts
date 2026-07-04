import { type Lang } from './settings/i18n'

export type ThemeColorId =
  | 'neutral'
  | 'warm'
  | 'cool'
  | 'tokyonight'
  | 'everforest'
  | 'ayu'
  | 'catppuccin'
  | 'catppuccin-macchiato'
  | 'gruvbox'
  | 'kanagawa'
  | 'nord'
  | 'one-dark'

export type ThemeColorPreset = {
  id: ThemeColorId
  lightHex: string
  darkHex: string
  accentHex: string
  labels: Record<Lang, string>
}

export const DEFAULT_THEME_COLOR: ThemeColorId = 'neutral'

export const THEME_COLOR_PRESETS: ThemeColorPreset[] = [
  { id: 'neutral', lightHex: '#FFFFFF', darkHex: '#212121', accentHex: '#2F6FF0', labels: { zh: '中性', 'zh-TW': '中性', en: 'Neutral' } },
  { id: 'warm', lightHex: '#FAF9F5', darkHex: '#28231D', accentHex: '#B86200', labels: { zh: '暖白', 'zh-TW': '暖白', en: 'Warm' } },
  { id: 'cool', lightHex: '#F6F8FB', darkHex: '#1F2630', accentHex: '#2F6FF0', labels: { zh: '冷白', 'zh-TW': '冷白', en: 'Cool' } },
  { id: 'tokyonight', lightHex: '#E1E2E7', darkHex: '#1A1B26', accentHex: '#7AA2F7', labels: { zh: 'Tokyo Night', 'zh-TW': 'Tokyo Night', en: 'Tokyo Night' } },
  { id: 'everforest', lightHex: '#FFFBEF', darkHex: '#2D353B', accentHex: '#A7C080', labels: { zh: 'Everforest', 'zh-TW': 'Everforest', en: 'Everforest' } },
  { id: 'ayu', lightHex: '#FAFAFA', darkHex: '#0A0E14', accentHex: '#FFB454', labels: { zh: 'Ayu', 'zh-TW': 'Ayu', en: 'Ayu' } },
  { id: 'catppuccin', lightHex: '#EFF1F5', darkHex: '#1E1E2E', accentHex: '#CBA6F7', labels: { zh: 'Catppuccin', 'zh-TW': 'Catppuccin', en: 'Catppuccin' } },
  { id: 'catppuccin-macchiato', lightHex: '#EFF1F5', darkHex: '#24273A', accentHex: '#C6A0F6', labels: { zh: 'Catppuccin Macchiato', 'zh-TW': 'Catppuccin Macchiato', en: 'Catppuccin Macchiato' } },
  { id: 'gruvbox', lightHex: '#FBF1C7', darkHex: '#282828', accentHex: '#FABD2F', labels: { zh: 'Gruvbox', 'zh-TW': 'Gruvbox', en: 'Gruvbox' } },
  { id: 'kanagawa', lightHex: '#F2ECBC', darkHex: '#1F1F28', accentHex: '#7E9CD8', labels: { zh: 'Kanagawa', 'zh-TW': 'Kanagawa', en: 'Kanagawa' } },
  { id: 'nord', lightHex: '#ECEFF4', darkHex: '#2E3440', accentHex: '#88C0D0', labels: { zh: 'Nord', 'zh-TW': 'Nord', en: 'Nord' } },
  { id: 'one-dark', lightHex: '#FAFAFA', darkHex: '#282C34', accentHex: '#61AFEF', labels: { zh: 'One', 'zh-TW': 'One', en: 'One' } },
]

export function normalizeThemeColorId(value: string | null | undefined): ThemeColorId {
  return THEME_COLOR_PRESETS.some((preset) => preset.id === value)
    ? value as ThemeColorId
    : DEFAULT_THEME_COLOR
}

export function getThemeColorPreset(value: string | null | undefined): ThemeColorPreset {
  const id = normalizeThemeColorId(value)
  return THEME_COLOR_PRESETS.find((preset) => preset.id === id) ?? THEME_COLOR_PRESETS[0]
}
