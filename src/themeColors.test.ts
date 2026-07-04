import { describe, expect, it } from 'vitest'
import {
  getThemeColorPreset,
  normalizeThemeColorId,
  THEME_COLOR_PRESETS,
} from './themeColors'

describe('theme color presets', () => {
  it('keeps legacy and editor theme ids valid', () => {
    const ids = THEME_COLOR_PRESETS.map((preset) => preset.id)
    expect(ids).toEqual([
      'neutral',
      'warm',
      'cool',
      'tokyonight',
      'everforest',
      'ayu',
      'catppuccin',
      'catppuccin-macchiato',
      'gruvbox',
      'kanagawa',
      'nord',
      'one-dark',
    ])
    expect(normalizeThemeColorId('tokyonight')).toBe('tokyonight')
    expect(normalizeThemeColorId('unknown')).toBe('neutral')
  })

  it('keeps light and dark preview colors for every theme', () => {
    expect(THEME_COLOR_PRESETS.every((preset) => preset.lightHex.startsWith('#'))).toBe(true)
    expect(THEME_COLOR_PRESETS.every((preset) => preset.darkHex.startsWith('#'))).toBe(true)
    expect(getThemeColorPreset('neutral').lightHex).toBe('#FFFFFF')
    expect(getThemeColorPreset('tokyonight').lightHex).toBe('#E1E2E7')
    expect(getThemeColorPreset('tokyonight').darkHex).toBe('#1A1B26')
    expect(getThemeColorPreset('ayu').accentHex).toBe('#FFB454')
  })
})
