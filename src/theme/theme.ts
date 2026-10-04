import { THEME_FIELDS, type ThemeDefinition, type ThemeMode, type ThemePalette } from './types'

const HEX_COLOR = /^#[0-9a-fA-F]{6}$/
const MAX_THEME_NAME_LENGTH = 80

type PaletteSurfaces = Pick<
  ThemePalette,
  'surface' | 'surfaceSoft' | 'surfaceMuted' | 'surfaceHover' | 'surfaceActive' | 'surfaceTitlebar' | 'border' | 'borderStrong'
>
type PaletteInk = Omit<ThemePalette, keyof PaletteSurfaces>

type ThemeSelection = {
  theme: ThemeMode
  themeColor: string
  customThemes?: readonly ThemeDefinition[]
}

type ActiveTheme = {
  mode: ThemeMode
  themeColor: string
  light: ThemePalette
  dark: ThemePalette
}

/** Semantic palette fields → the custom properties preview and the document root both consume. */
const PALETTE_VARIABLES: Record<keyof ThemePalette, string> = {
  surface: '--theme-surface',
  surfaceSoft: '--theme-surface-soft',
  surfaceMuted: '--theme-surface-muted',
  surfaceHover: '--theme-surface-hover',
  surfaceActive: '--theme-surface-active',
  surfaceTitlebar: '--theme-surface-titlebar',
  border: '--theme-surface-border',
  borderStrong: '--theme-surface-border-strong',
  text: '--text',
  textMuted: '--text-muted',
  textFaint: '--text-faint',
  accent: '--accent',
  accentHover: '--accent-hover',
  accentSoft: '--accent-soft',
  onAccent: '--text-onaccent',
  danger: '--danger',
  dangerSoft: '--danger-soft',
}

/**
 * Tokens that `.dark` and settings scopes assign as concrete colors.
 * A preview sets this map inline and does not add `.dark`, so these have to be
 * restated or light text and surfaces keep the inherited global dark values.
 * `color` is the used value ancestors already computed; overriding `--text` alone does not replace it.
 */
const COMPONENT_VARIABLES: Record<string, keyof ThemePalette> = {
  '--bg': 'surface',
  '--bg-sidebar': 'surfaceMuted',
  '--bg-savebar': 'surfaceSoft',
  '--bg-input': 'surface',
  '--bg-input-subtle': 'surfaceMuted',
  '--bg-hover': 'surfaceHover',
  '--bg-active': 'surfaceActive',
  '--bg-pill': 'surfaceHover',
  '--bg-titlebar': 'surfaceTitlebar',
  '--border': 'border',
  '--border-strong': 'borderStrong',
  '--border-input': 'borderStrong',
  '--divider': 'surfaceTitlebar',
  '--color-background': 'surface',
  '--color-foreground': 'text',
  '--color-card': 'surface',
  '--color-card-foreground': 'text',
  '--color-muted': 'surfaceMuted',
  '--color-muted-foreground': 'textMuted',
  '--color-border': 'border',
  '--color-input': 'borderStrong',
  '--color-primary': 'text',
  '--color-sidebar': 'surfaceSoft',
  color: 'text',
}

/** Original light ink for Neutral, Warm, and Cool. */
const LIGHT_INK: PaletteInk = {
  text: '#1d1d1f',
  textMuted: '#737373',
  textFaint: '#9c9ca3',
  accent: '#2f6ff0',
  accentHover: '#2960d8',
  accentSoft: '#e6efff',
  onAccent: '#ffffff',
  danger: '#c4341c',
  dangerSoft: '#fdecea',
}

/**
 * Dark neutral ink from the existing `.dark` tokens.
 * Translucent accent/danger washes are flattened onto #212121 so every field stays #RRGGBB.
 */
const NEUTRAL_DARK_INK: PaletteInk = {
  text: '#f0f0f3',
  textMuted: '#a1a1aa',
  textFaint: '#6e6e76',
  accent: '#5c8df7',
  accentHover: '#6f9bf8',
  accentSoft: '#2a3243',
  onAccent: '#ffffff',
  danger: '#ef6a5a',
  dangerSoft: '#3e2b29',
}

const WARM_DARK_INK: PaletteInk = {
  text: '#f3efe6',
  textMuted: '#c4b5a2',
  textFaint: '#8d7d6b',
  accent: '#5c8df7',
  accentHover: '#6f9bf8',
  accentSoft: '#2d303c',
  onAccent: '#ffffff',
  danger: '#ef6a5a',
  dangerSoft: '#402921',
}

const COOL_DARK_INK: PaletteInk = {
  text: '#e7eef8',
  textMuted: '#a9b7cc',
  textFaint: '#6d7d96',
  accent: '#5c8df7',
  accentHover: '#6f9bf8',
  accentSoft: '#222e48',
  onAccent: '#ffffff',
  danger: '#ef6a5a',
  dangerSoft: '#35272e',
}

function palette(surfaces: PaletteSurfaces, ink: PaletteInk): ThemePalette {
  return Object.freeze({ ...surfaces, ...ink })
}

function defineTheme(id: string, name: string, light: ThemePalette, dark: ThemePalette): ThemeDefinition {
  return Object.freeze({ id, name, light, dark })
}

const neutralTheme = defineTheme(
  'neutral',
  'Neutral',
  palette({
    surface: '#fdfcfa',
    surfaceSoft: '#f9f8f6',
    surfaceMuted: '#f4f3f1',
    surfaceHover: '#eeedeb',
    surfaceActive: '#e7e6e4',
    surfaceTitlebar: '#eae9e7',
    border: '#e2e1df',
    borderStrong: '#d2d1cf',
  }, LIGHT_INK),
  palette({
    surface: '#212121',
    surfaceSoft: '#262629',
    surfaceMuted: '#2a2a2d',
    surfaceHover: '#2c2c30',
    surfaceActive: '#36363c',
    surfaceTitlebar: '#313136',
    border: '#3a3a40',
    borderStrong: '#4e4e56',
  }, NEUTRAL_DARK_INK),
)

const warmTheme = defineTheme(
  'warm',
  'Warm',
  palette({
    surface: '#f7f3ea',
    surfaceSoft: '#f2ece0',
    surfaceMuted: '#ece5d4',
    surfaceHover: '#e4dbc8',
    surfaceActive: '#dbd0ba',
    surfaceTitlebar: '#e9e1d1',
    border: '#ddd3c0',
    borderStrong: '#cbbda4',
  }, LIGHT_INK),
  palette({
    surface: '#241e18',
    surfaceSoft: '#2b241c',
    surfaceMuted: '#322a20',
    surfaceHover: '#3a3126',
    surfaceActive: '#4a3f30',
    surfaceTitlebar: '#42382b',
    border: '#5c4e3c',
    borderStrong: '#7a6854',
  }, WARM_DARK_INK),
)

const coolTheme = defineTheme(
  'cool',
  'Cool',
  palette({
    surface: '#eef3fa',
    surfaceSoft: '#e7eef7',
    surfaceMuted: '#dee8f2',
    surfaceHover: '#d4e0ee',
    surfaceActive: '#c8d7e8',
    surfaceTitlebar: '#e0e9f4',
    border: '#d2def0',
    borderStrong: '#bccfe6',
  }, LIGHT_INK),
  palette({
    surface: '#171c27',
    surfaceSoft: '#1c2433',
    surfaceMuted: '#222c3d',
    surfaceHover: '#28344a',
    surfaceActive: '#364666',
    surfaceTitlebar: '#2e3c56',
    border: '#45587a',
    borderStrong: '#5d7399',
  }, COOL_DARK_INK),
)

/*!
 * Palettes adapted from T3 Code: stock, T3 Chat, Grove, Ocean, Ember, and Iris.
 * Copyright (c) 2026 T3 Tools Inc.
 *
 * Permission is hereby granted, free of charge, to any person obtaining a copy
 * of this software and associated documentation files (the "Software"), to deal
 * in the Software without restriction, including without limitation the rights
 * to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 * copies of the Software, and to permit persons to whom the Software is
 * furnished to do so, subject to the following conditions:
 *
 * The above copyright notice and this permission notice shall be included in all
 * copies or substantial portions of the Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
 * AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 * OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
 * SOFTWARE.
 */
// Kivio's surface ramp and opaque #RRGGBB fields need no runtime conversion.
const graphiteTheme = defineTheme(
  'graphite',
  'Graphite',
  palette({
    surface: '#fcfcfc',
    surfaceSoft: '#f9f9f9',
    surfaceMuted: '#f5f5f5',
    surfaceHover: '#efefef',
    surfaceActive: '#e7e7e7',
    surfaceTitlebar: '#f1f1f2',
    border: '#e0e0e1',
    borderStrong: '#cdcdce',
  }, {
    text: '#27272a',
    textMuted: '#71717b',
    textFaint: '#71717b',
    accent: '#1b4ed8',
    accentHover: '#1846c2',
    accentSoft: '#e6ebf8',
    onAccent: '#ffffff',
    danger: '#c10007',
    dangerSoft: '#fcebec',
  }),
  palette({
    surface: '#0a0a0a',
    surfaceSoft: '#131313',
    surfaceMuted: '#1a1a1a',
    surfaceHover: '#222222',
    surfaceActive: '#2d2d2d',
    surfaceTitlebar: '#262626',
    border: '#373737',
    borderStrong: '#4c4c4c',
  }, {
    text: '#f5f5f5',
    textMuted: '#818181',
    textFaint: '#818181',
    accent: '#6c9bff',
    accentHover: '#7ba5ff',
    accentSoft: '#1b2334',
    onAccent: '#0a0a0a',
    danger: '#ff6467',
    dangerSoft: '#301214',
  }),
)

const blossomTheme = defineTheme(
  'blossom',
  'Blossom',
  palette({
    surface: '#fdf7fd',
    surfaceSoft: '#faf4fa',
    surfaceMuted: '#f7eff7',
    surfaceHover: '#f3eaf3',
    surfaceActive: '#ece1ec',
    surfaceTitlebar: '#f4ecf5',
    border: '#e7dae7',
    borderStrong: '#d7c6d8',
  }, {
    text: '#501854',
    textMuted: '#ac1668',
    textFaint: '#8b5f90',
    accent: '#c5236b',
    accentHover: '#b12060',
    accentSoft: '#f7e2ee',
    onAccent: '#ffffff',
    danger: '#9d174d',
    dangerSoft: '#fde4f1',
  }),
  palette({
    surface: '#1f1a24',
    surfaceSoft: '#28232d',
    surfaceMuted: '#2e2a33',
    surfaceHover: '#35303a',
    surfaceActive: '#403b44',
    surfaceTitlebar: '#39353e',
    border: '#48444d',
    borderStrong: '#5c5860',
  }, {
    text: '#f9f8fb',
    textMuted: '#e7d0dd',
    textFaint: '#968d9f',
    accent: '#ed83b5',
    accentHover: '#ef8fbc',
    accentSoft: '#422c3d',
    onAccent: '#1f1a24',
    danger: '#ff6467',
    dangerSoft: '#3e242b',
  }),
)

const groveTheme = defineTheme(
  'grove',
  'Grove',
  palette({
    surface: '#f3f7f4',
    surfaceSoft: '#f0f4f1',
    surfaceMuted: '#ecefed',
    surfaceHover: '#e7e9e7',
    surfaceActive: '#dee0df',
    surfaceTitlebar: '#e9ecea',
    border: '#d8dad9',
    borderStrong: '#c5c5c6',
  }, {
    text: '#241523',
    textMuted: '#746c73',
    textFaint: '#716971',
    accent: '#1b7d50',
    accentHover: '#187148',
    accentSoft: '#ddebe4',
    onAccent: '#fffaff',
    danger: '#c10007',
    dangerSoft: '#f4e7e5',
  }),
  palette({
    surface: '#1b2821',
    surfaceSoft: '#24302a',
    surfaceMuted: '#2b3731',
    surfaceHover: '#323d37',
    surfaceActive: '#3d4842',
    surfaceTitlebar: '#36413c',
    border: '#46504b',
    borderStrong: '#5b635f',
  }, {
    text: '#fffaff',
    textMuted: '#919595',
    textFaint: '#a9abab',
    accent: '#69d69a',
    accentHover: '#78daa4',
    accentSoft: '#284636',
    onAccent: '#241523',
    danger: '#ff6668',
    dangerSoft: '#3f2c28',
  }),
)

const oceanTheme = defineTheme(
  'ocean',
  'Ocean',
  palette({
    surface: '#f5f7f8',
    surfaceSoft: '#f2f4f5',
    surfaceMuted: '#eeeff1',
    surfaceHover: '#e8e9eb',
    surfaceActive: '#e0e0e3',
    surfaceTitlebar: '#ebeced',
    border: '#dadadc',
    borderStrong: '#c7c5c9',
  }, {
    text: '#241523',
    textMuted: '#746c75',
    textFaint: '#716972',
    accent: '#2672af',
    accentHover: '#22679e',
    accentSoft: '#e0eaf1',
    onAccent: '#fffaff',
    danger: '#c10007',
    dangerSoft: '#f5e6e9',
  }),
  palette({
    surface: '#17212b',
    surfaceSoft: '#202a33',
    surfaceMuted: '#27303a',
    surfaceHover: '#2e3740',
    surfaceActive: '#3a424b',
    surfaceTitlebar: '#333b44',
    border: '#434a53',
    borderStrong: '#585e66',
  }, {
    text: '#fffaff',
    textMuted: '#8d8f97',
    textFaint: '#a4a4ac',
    accent: '#70b9ee',
    accentHover: '#7ec0f0',
    accentSoft: '#263b4c',
    onAccent: '#241523',
    danger: '#ff6467',
    dangerSoft: '#3c2630',
  }),
)

const emberTheme = defineTheme(
  'ember',
  'Ember',
  palette({
    surface: '#f9f7f5',
    surfaceSoft: '#f6f4f2',
    surfaceMuted: '#f2efee',
    surfaceHover: '#ece9e8',
    surfaceActive: '#e4e0e0',
    surfaceTitlebar: '#eeeceb',
    border: '#dddada',
    borderStrong: '#cac5c7',
  }, {
    text: '#241523',
    textMuted: '#766c74',
    textFaint: '#736971',
    accent: '#ae552a',
    accentHover: '#9d4d26',
    accentSoft: '#f2e7e1',
    onAccent: '#fffaff',
    danger: '#c10007',
    dangerSoft: '#f9e7e6',
  }),
  palette({
    surface: '#291e1a',
    surfaceSoft: '#322723',
    surfaceMuted: '#382d2a',
    surfaceHover: '#3e3431',
    surfaceActive: '#493f3c',
    surfaceTitlebar: '#433835',
    border: '#524846',
    borderStrong: '#655c5a',
  }, {
    text: '#fffaff',
    textMuted: '#968e8f',
    textFaint: '#aba3a5',
    accent: '#f09a64',
    accentHover: '#f2a474',
    accentSoft: '#4b3327',
    onAccent: '#241523',
    danger: '#ff6467',
    dangerSoft: '#4a2321',
  }),
)

const irisTheme = defineTheme(
  'iris',
  'Iris',
  palette({
    surface: '#f8f7f9',
    surfaceSoft: '#f5f4f6',
    surfaceMuted: '#f1eff2',
    surfaceHover: '#ebe9ec',
    surfaceActive: '#e3e0e4',
    surfaceTitlebar: '#edecee',
    border: '#dcdadd',
    borderStrong: '#c9c5ca',
  }, {
    text: '#241523',
    textMuted: '#766c76',
    textFaint: '#736973',
    accent: '#7253b9',
    accentHover: '#674ba7',
    accentSoft: '#ebe7f3',
    onAccent: '#fffaff',
    danger: '#c10007',
    dangerSoft: '#f8e6ea',
  }),
  palette({
    surface: '#1d1929',
    surfaceSoft: '#262232',
    surfaceMuted: '#2d2938',
    surfaceHover: '#34303e',
    surfaceActive: '#3f3b49',
    surfaceTitlebar: '#383443',
    border: '#484452',
    borderStrong: '#5c5865',
  }, {
    text: '#fffaff',
    textMuted: '#8e8a95',
    textFaint: '#a29ea8',
    accent: '#9d7df2',
    accentHover: '#a78af3',
    accentSoft: '#332a4b',
    onAccent: '#241523',
    danger: '#ff6467',
    dangerSoft: '#40202e',
  }),
)

const whiteTheme = defineTheme(
  'white',
  'White',
  palette({
    surface: '#ffffff',
    surfaceSoft: '#fafafa',
    surfaceMuted: '#f5f5f5',
    surfaceHover: '#eeeeee',
    surfaceActive: '#e5e5e5',
    surfaceTitlebar: '#f0f0f0',
    border: '#e0e0e0',
    borderStrong: '#cccccc',
  }, LIGHT_INK),
  palette({
    surface: '#181818',
    surfaceSoft: '#202020',
    surfaceMuted: '#262626',
    surfaceHover: '#303030',
    surfaceActive: '#3a3a3a',
    surfaceTitlebar: '#242424',
    border: '#383838',
    borderStrong: '#505050',
  }, { ...NEUTRAL_DARK_INK, onAccent: '#181818' }),
)

// Adapted to the same semantic fields as the other built-ins; no separate UI styles.
const nordTheme = defineTheme(
  'nord',
  'Nord',
  palette({
    surface: '#eceff4',
    surfaceSoft: '#e5e9f0',
    surfaceMuted: '#d8dee9',
    surfaceHover: '#cdd5e2',
    surfaceActive: '#c1cbdc',
    surfaceTitlebar: '#dfe5ee',
    border: '#c5cedc',
    borderStrong: '#a8b5c9',
  }, {
    text: '#2e3440',
    textMuted: '#4c566a',
    textFaint: '#66748b',
    accent: '#3b6384',
    accentHover: '#2e506e',
    accentSoft: '#d3e2ee',
    onAccent: '#ffffff',
    danger: '#a33b47',
    dangerSoft: '#f2dfe3',
  }),
  palette({
    surface: '#2e3440',
    surfaceSoft: '#343c4a',
    surfaceMuted: '#3b4252',
    surfaceHover: '#434c5e',
    surfaceActive: '#4c566a',
    surfaceTitlebar: '#363e4d',
    border: '#4c566a',
    borderStrong: '#66748b',
  }, {
    text: '#eceff4',
    textMuted: '#d8dee9',
    textFaint: '#a5b1c5',
    accent: '#88c0d0',
    accentHover: '#8fbcbb',
    accentSoft: '#354f5c',
    onAccent: '#2e3440',
    danger: '#e89aa5',
    dangerSoft: '#513b48',
  }),
)

const solarizedTheme = defineTheme(
  'solarized',
  'Solarized',
  palette({
    surface: '#fdf6e3',
    surfaceSoft: '#f6efdc',
    surfaceMuted: '#eee8d5',
    surfaceHover: '#e6dfca',
    surfaceActive: '#dcd4bd',
    surfaceTitlebar: '#f0ead7',
    border: '#d9d2bd',
    borderStrong: '#bdb59d',
  }, {
    text: '#073642',
    textMuted: '#586e75',
    textFaint: '#727d77',
    accent: '#006d82',
    accentHover: '#005565',
    accentSoft: '#dcebea',
    onAccent: '#ffffff',
    danger: '#bd302f',
    dangerSoft: '#f8e2d5',
  }),
  palette({
    surface: '#002b36',
    surfaceSoft: '#04313c',
    surfaceMuted: '#073642',
    surfaceHover: '#124450',
    surfaceActive: '#20515d',
    surfaceTitlebar: '#06333e',
    border: '#28505b',
    borderStrong: '#466773',
  }, {
    text: '#eee8d5',
    textMuted: '#a9b8b5',
    textFaint: '#839496',
    accent: '#2aa198',
    accentHover: '#45b7ad',
    accentSoft: '#124846',
    onAccent: '#002b36',
    danger: '#f08072',
    dangerSoft: '#4b3339',
  }),
)

export const BUILTIN_THEMES: readonly ThemeDefinition[] = Object.freeze([
  neutralTheme,
  warmTheme,
  coolTheme,
  graphiteTheme,
  blossomTheme,
  groveTheme,
  oceanTheme,
  emberTheme,
  irisTheme,
  whiteTheme,
  nordTheme,
  solarizedTheme,
])

function clonePalette(source: ThemePalette): ThemePalette {
  const copy = {} as ThemePalette
  for (const field of THEME_FIELDS) copy[field] = source[field]
  return copy
}

export function resolveTheme(settings: Pick<ThemeSelection, 'themeColor' | 'customThemes'>): ThemeDefinition {
  return BUILTIN_THEMES.find(item => item.id === settings.themeColor)
    ?? settings.customThemes?.find(item => item.id === settings.themeColor)
    ?? neutralTheme
}

/** Pure variable map. Callers assign it to a preview node; this function does not touch the document. */
export function paletteVariables(palette: ThemePalette): Record<string, string> {
  const variables: Record<string, string> = {}
  for (const field of THEME_FIELDS) variables[PALETTE_VARIABLES[field]] = palette[field]
  for (const [name, field] of Object.entries(COMPONENT_VARIABLES)) variables[name] = palette[field]
  return variables
}

let active: ActiveTheme | null = null
let mediaQuery: MediaQueryList | null = null

function readSystemDark(systemDark?: boolean): boolean {
  if (typeof systemDark === 'boolean') return systemDark
  return Boolean(mediaQuery?.matches)
}

function paint(systemDark?: boolean): void {
  if (!active) return
  const root = document.documentElement
  const dark = active.mode === 'dark' || (active.mode === 'system' && readSystemDark(systemDark))
  const variables = paletteVariables(dark ? active.dark : active.light)
  for (const [name, value] of Object.entries(variables)) root.style.setProperty(name, value)
  root.classList.toggle('dark', dark)
  root.dataset.themeColor = active.themeColor
  if (root.dataset.themeBooting === 'true') root.style.backgroundColor = variables['--theme-surface']
}

function onSystemSchemeChange(event: MediaQueryListEvent): void {
  if (!active || active.mode !== 'system') return
  paint(event.matches)
}

function ensureSystemListener(): void {
  if (mediaQuery) return
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return
  const query = window.matchMedia('(prefers-color-scheme: dark)')
  query.addEventListener('change', onSystemSchemeChange)
  mediaQuery = query
}

/**
 * Apply one resolved palette to the document root.
 * The module keeps a single `prefers-color-scheme` listener and repaints from the latest call.
 * `systemDark` overrides the media query for this call only.
 */
export function applyThemeSettings(settings: ThemeSelection, systemDark?: boolean): void {
  const resolved = resolveTheme(settings)
  active = {
    mode: settings.theme,
    themeColor: settings.themeColor,
    light: clonePalette(resolved.light),
    dark: clonePalette(resolved.dark),
  }
  ensureSystemListener()
  try {
    // Cache only resolved paint data; never copy settings or credentials to storage.
    window.localStorage.setItem('kivio.theme.startup.v1', JSON.stringify({
      version: 1,
      mode: active.mode,
      themeColor: active.themeColor,
      light: paletteVariables(active.light),
      dark: paletteVariables(active.dark),
    }))
  } catch {
    // A disabled/full storage area must not prevent applying authoritative settings.
  }
  paint(systemDark)
}

/** Drop the system listener. The next `applyThemeSettings` installs it again. */
export function disposeTheme(): void {
  if (mediaQuery) {
    mediaQuery.removeEventListener('change', onSystemSchemeChange)
    mediaQuery = null
  }
  active = null
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function rejectUnknown(value: Record<string, unknown>, allowed: readonly string[], prefix: string): void {
  for (const key of Object.keys(value)) {
    if (!allowed.includes(key)) throw new Error(`Unsupported theme field: ${prefix}${key}`)
  }
}

function parsePalette(value: unknown, label: string): ThemePalette {
  if (!isRecord(value)) throw new Error(`Invalid theme palette: ${label}`)
  rejectUnknown(value, THEME_FIELDS, `${label}.`)
  const parsed = {} as ThemePalette
  for (const field of THEME_FIELDS) {
    const color = value[field]
    if (typeof color !== 'string' || !HEX_COLOR.test(color)) {
      throw new Error(`Invalid theme color: ${label}.${field}`)
    }
    parsed[field] = color.toLowerCase()
  }
  return parsed
}

function parseName(value: unknown): string {
  if (typeof value !== 'string') throw new Error('Theme name must be 1–80 characters')
  const name = value.trim()
  if (name.length < 1 || name.length > MAX_THEME_NAME_LENGTH) {
    throw new Error('Theme name must be 1–80 characters')
  }
  if (/\p{Cc}/u.test(name)) throw new Error('Theme name must be 1–80 characters without control characters')
  return name
}

function orderedPalette(source: ThemePalette): ThemePalette {
  const ordered = {} as ThemePalette
  for (const field of THEME_FIELDS) ordered[field] = source[field]
  return ordered
}

/**
 * Strict version-1 document: `{ version: 1, theme: { name, light, dark } }`.
 * Import always assigns a new id and ignores any caller-supplied identity. Unknown fields are rejected.
 */
export function parseThemeJson(text: string): ThemeDefinition {
  let parsed: unknown
  try {
    parsed = JSON.parse(text)
  } catch {
    throw new Error('Invalid theme JSON')
  }
  if (!isRecord(parsed)) throw new Error('Invalid theme JSON')
  if (typeof parsed.version !== 'number' || parsed.version !== 1) {
    const suffix = typeof parsed.version === 'number' || typeof parsed.version === 'string'
      ? `: ${parsed.version}`
      : ''
    throw new Error(`Unsupported theme document version${suffix}`)
  }
  rejectUnknown(parsed, ['version', 'theme'], '')
  if (!isRecord(parsed.theme)) throw new Error('Invalid theme JSON')
  rejectUnknown(parsed.theme, ['name', 'light', 'dark'], 'theme.')
  return {
    id: crypto.randomUUID(),
    name: parseName(parsed.theme.name),
    light: parsePalette(parsed.theme.light, 'light'),
    dark: parsePalette(parsed.theme.dark, 'dark'),
  }
}

export function exportThemeJson(theme: ThemeDefinition): string {
  return JSON.stringify({
    version: 1,
    theme: {
      name: theme.name,
      light: orderedPalette(theme.light),
      dark: orderedPalette(theme.dark),
    },
  }, null, 2)
}
