// @vitest-environment jsdom
import bootstrap from '../../public/theme-bootstrap.js?raw'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { THEME_FIELDS } from './types'
import { BUILTIN_THEMES, applyThemeSettings, disposeTheme, exportThemeJson, paletteVariables, parseThemeJson, resolveTheme } from './theme'

const startupKey = 'kivio.theme.startup.v1'

function reloadThemeDocument() {
  disposeTheme()
  document.documentElement.className = ''
  document.documentElement.removeAttribute('style')
  delete document.documentElement.dataset.themeColor
  delete document.documentElement.dataset.themeBooting
  new Function('window', 'document', bootstrap)(window, document)
}

function installScheme(initial = false) {
  let matches = initial
  const listeners = new Set<(event: MediaQueryListEvent) => void>()
  const query = {
    get matches() { return matches },
    addEventListener(_type: string, listener: (event: MediaQueryListEvent) => void) { listeners.add(listener) },
    removeEventListener(_type: string, listener: (event: MediaQueryListEvent) => void) { listeners.delete(listener) },
  }
  Object.defineProperty(window, 'matchMedia', { configurable: true, value: () => query })
  return (next: boolean) => {
    matches = next
    for (const listener of [...listeners]) listener({ matches: next } as MediaQueryListEvent)
  }
}

function documentForImport() {
  return JSON.parse(exportThemeJson(BUILTIN_THEMES[0]))
}

function rootValue(name: string) { return document.documentElement.style.getPropertyValue(name) }

beforeEach(() => {
  disposeTheme()
  document.documentElement.className = ''
  document.documentElement.removeAttribute('style')
  window.localStorage.removeItem(startupKey)
  delete document.documentElement.dataset.themeColor
  delete document.documentElement.dataset.themeBooting
  installScheme()
})
afterEach(disposeTheme)

describe('theme sharing', () => {
  it('roundtrips both palettes while giving each import an independent identity and editable data', () => {
    const source = parseThemeJson(exportThemeJson(BUILTIN_THEMES[0]))
    source.name = 'Forest'
    source.light.surface = '#ddeedd'
    source.dark.accent = '#70cc96'
    const exported = exportThemeJson(source)
    const first = parseThemeJson(exported)
    const second = parseThemeJson(exported)
    expect(first.id).not.toBe(source.id)
    expect(second.id).not.toBe(first.id)
    expect(first.name).toBe(source.name)
    expect(first.light).toEqual(source.light)
    expect(first.dark).toEqual(source.dark)
    first.light.surface = '#000000'
    expect(second.light.surface).toBe('#ddeedd')
    expect(source.light.surface).toBe('#ddeedd')
  })

  it('rejects unsupported document shapes, arbitrary CSS, and invalid names without applying a theme', () => {
    applyThemeSettings({ theme: 'light', themeColor: 'neutral' })
    const before = rootValue('--theme-surface')
    const invalid = [null, [], {}, { version: 2 }, { version: '1' }]
    for (const value of invalid) expect(() => parseThemeJson(JSON.stringify(value))).toThrow()
    expect(() => parseThemeJson('{')).toThrow()
    for (const name of ['', '   ', 'a'.repeat(81), 'bad\u0000name', 'bad\u0085name']) {
      const value = documentForImport()
      value.theme.name = name
      expect(() => parseThemeJson(JSON.stringify(value))).toThrow()
    }
    const css = documentForImport()
    css.theme.css = 'html { background: red }'
    expect(() => parseThemeJson(JSON.stringify(css))).toThrow()
    const id = documentForImport()
    id.theme.id = 'neutral'
    expect(() => parseThemeJson(JSON.stringify(id))).toThrow()
    expect(rootValue('--theme-surface')).toBe(before)
  })

  it.each(THEME_FIELDS)('requires a safe complete %s color in both palettes', field => {
    for (const mode of ['light', 'dark']) {
      for (const color of [undefined, '#fff', '#11223344', 'red', 'url(https://invalid.example)']) {
        const value = documentForImport()
        value.theme[mode][field] = color
        expect(() => parseThemeJson(JSON.stringify(value))).toThrow()
      }
    }
  })
})

describe('theme application', () => {
  it('uses the latest custom palette for system events and ignores them in explicit mode', () => {
    const system = installScheme()
    const first = parseThemeJson(exportThemeJson(BUILTIN_THEMES[0]))
    first.light.surface = '#ddeedd'
    first.dark.surface = '#15251b'
    first.dark.accent = '#70cc96'
    applyThemeSettings({ theme: 'system', themeColor: first.id, customThemes: [first] })
    expect(rootValue('--theme-surface')).toBe('#ddeedd')
    system(true)
    expect(document.documentElement.classList.contains('dark')).toBe(true)
    expect(rootValue('--theme-surface')).toBe('#15251b')
    expect(rootValue('--accent')).toBe('#70cc96')
    const next = { ...first, dark: { ...first.dark, surface: '#102030' } }
    applyThemeSettings({ theme: 'system', themeColor: next.id, customThemes: [next] })
    system(false)
    system(true)
    expect(rootValue('--theme-surface')).toBe('#102030')
    applyThemeSettings({ theme: 'light', themeColor: next.id, customThemes: [next] })
    system(true)
    expect(document.documentElement.classList.contains('dark')).toBe(false)
    expect(rootValue('--theme-surface')).toBe('#ddeedd')
    disposeTheme()
    system(true)
    expect(rootValue('--theme-surface')).toBe('#ddeedd')
  })

  it('keeps preview palettes independent from the active document', () => {
    const theme = parseThemeJson(exportThemeJson(BUILTIN_THEMES[0]))
    theme.light.surface = '#ddeedd'
    theme.dark.surface = '#15251b'
    applyThemeSettings({ theme: 'dark', themeColor: theme.id, customThemes: [theme] })
    const preview = paletteVariables(theme.light)
    expect(preview['--theme-surface']).toBe('#ddeedd')
    preview['--theme-surface'] = '#000000'
    expect(rootValue('--theme-surface')).toBe('#15251b')
    expect(theme.light.surface).toBe('#ddeedd')
  })

  it.each(BUILTIN_THEMES)('protects $id builtin identity and falls back safely for a deleted selection', builtin => {
    const custom = parseThemeJson(exportThemeJson(BUILTIN_THEMES[0]))
    custom.light.surface = '#abcabc'
    const reserved = { ...custom, id: builtin.id }
    expect(resolveTheme({ themeColor: builtin.id, customThemes: [reserved] })).toBe(builtin)
    expect(resolveTheme({ themeColor: custom.id, customThemes: [custom] })).toBe(custom)
    expect(resolveTheme({ themeColor: custom.id, customThemes: [] })).toBe(BUILTIN_THEMES[0])
  })
})

describe('additional theme readability', () => {
  function luminance(hex: string) {
    const channels = [1, 3, 5].map(offset => {
      const value = parseInt(hex.slice(offset, offset + 2), 16) / 255
      return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4
    })
    return channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722
  }

  it.each(['white', 'nord', 'solarized'])('%s keeps message and primary-button text readable in both modes', id => {
    const theme = resolveTheme({ themeColor: id })
    for (const mode of ['light', 'dark'] as const) {
      const palette = theme[mode]
      for (const [foreground, background] of [
        [palette.text, palette.surface],
        [palette.text, palette.surfaceActive],
        [palette.onAccent, palette.accent],
      ]) {
        const first = luminance(foreground)
        const second = luminance(background)
        expect((Math.max(first, second) + 0.05) / (Math.min(first, second) + 0.05)).toBeGreaterThanOrEqual(4.5)
      }
    }
  })
})

describe('theme refresh startup', () => {
  let previousUrl: string
  beforeEach(() => {
    previousUrl = window.location.href
    window.history.replaceState(null, '', '#chat')
  })
  afterEach(() => {
    vi.restoreAllMocks()
    window.history.replaceState(null, '', previousUrl)
    window.localStorage.removeItem(startupKey)
    document.documentElement.style.removeProperty('background-color')
    delete document.documentElement.dataset.themeBooting
  })

  it.each(['light', 'dark'] as const)('restores explicit %s before backend settings arrive despite the opposite system mode', mode => {
    installScheme(mode === 'light')
    applyThemeSettings({ theme: mode, themeColor: 'nord' })
    reloadThemeDocument()
    const palette = resolveTheme({ themeColor: 'nord' })[mode]
    expect(rootValue('--theme-surface')).toBe(palette.surface)
    expect(rootValue('--text')).toBe(palette.text)
    expect(rootValue('--bg-input')).toBe(palette.surface)
    const swatch = document.createElement('span')
    swatch.style.backgroundColor = palette.surface
    expect(document.documentElement.style.backgroundColor).toBe(swatch.style.backgroundColor)
    expect(document.documentElement.classList.contains('dark')).toBe(mode === 'dark')
  })

  it('restores the latest custom palette and reevaluates system appearance on refresh', () => {
    const system = installScheme(false)
    const custom = parseThemeJson(exportThemeJson(BUILTIN_THEMES[0]))
    custom.dark.surface = '#123456'
    custom.dark.accent = '#abcdef'
    applyThemeSettings({ theme: 'system', themeColor: custom.id, customThemes: [custom] })
    custom.dark.surface = '#234567'
    applyThemeSettings({ theme: 'system', themeColor: custom.id, customThemes: [custom] })
    system(true)
    reloadThemeDocument()
    expect(rootValue('--theme-surface')).toBe('#234567')
    expect(rootValue('--accent')).toBe('#abcdef')
    expect(document.documentElement.classList.contains('dark')).toBe(true)
    expect(document.documentElement.style.backgroundColor).toBe('rgb(35, 69, 103)')

    applyThemeSettings({ theme: 'light', themeColor: 'solarized' })
    expect(rootValue('--theme-surface')).toBe('#fdf6e3')
    reloadThemeDocument()
    expect(rootValue('--theme-surface')).toBe('#fdf6e3')
    expect(document.documentElement.dataset.themeColor).toBe('solarized')
    expect(document.documentElement.classList.contains('dark')).toBe(false)
  })

  it.each(['#lens', '#translator', '#desktop-pet'])('preserves the transparent canvas for %s', route => {
    applyThemeSettings({ theme: 'dark', themeColor: 'nord' })
    window.history.replaceState(null, '', route)
    reloadThemeDocument()
    expect(rootValue('--theme-surface')).toBe('#2e3440')
    expect(document.documentElement.style.backgroundColor).toBe('')
    expect(document.documentElement.dataset.themeBooting).toBeUndefined()
  })

  it('ignores corrupt or CSS-injected cache data without partially applying it', () => {
    for (const value of ['{', JSON.stringify({
      version: 1, mode: 'dark', themeColor: 'injected',
      dark: { '--theme-surface': '#123456', '--text': '#ffffff', '--accent': 'url(https://example.invalid)' },
    })]) {
      window.localStorage.setItem(startupKey, value)
      reloadThemeDocument()
      expect(rootValue('--theme-surface')).toBe('')
      expect(document.documentElement.dataset.themeColor).toBeUndefined()
      expect(document.documentElement.classList.contains('dark')).toBe(false)
    }
  })

  it('keeps applying authoritative themes when local storage is unavailable', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => { throw new Error('Storage disabled') })
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('Storage disabled') })
    reloadThemeDocument()
    applyThemeSettings({ theme: 'dark', themeColor: 'nord' })
    expect(rootValue('--theme-surface')).toBe('#2e3440')
    expect(rootValue('--text')).toBe('#eceff4')
    expect(document.documentElement.dataset.themeColor).toBe('nord')
  })
})
