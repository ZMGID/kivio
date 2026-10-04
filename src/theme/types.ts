export type ThemeMode = 'system' | 'light' | 'dark'

export const THEME_FIELDS = [
  'surface', 'surfaceSoft', 'surfaceMuted', 'surfaceHover', 'surfaceActive',
  'surfaceTitlebar', 'border', 'borderStrong', 'text', 'textMuted', 'textFaint',
  'accent', 'accentHover', 'accentSoft', 'onAccent', 'danger', 'dangerSoft',
] as const

export type ThemePalette = Record<(typeof THEME_FIELDS)[number], string>

export interface ThemeDefinition {
  id: string
  name: string
  light: ThemePalette
  dark: ThemePalette
}
