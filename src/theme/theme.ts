import builtinThemes from './builtins.json'
import { THEME_FIELDS, type ThemeDefinition, type ThemeMode, type ThemePalette } from './types'

const HEX_COLOR = /^#[0-9a-fA-F]{6}$/
const MAX_THEME_NAME_LENGTH = 80

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

/** Shared authored palettes consumed by the web and native clients. */
export const BUILTIN_THEMES: readonly ThemeDefinition[] = Object.freeze(builtinThemes)
const neutralTheme = BUILTIN_THEMES[0]

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
