import { useEffect, useState } from 'react'

export function readThemeToken(name: string, fallback: string): string {
  if (typeof document === 'undefined') return fallback
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback
}

const THEME_TOKENS = [
  '--theme-surface', '--theme-surface-soft', '--theme-surface-muted',
  '--theme-surface-border', '--theme-surface-border-strong', '--text', '--text-muted',
  '--accent', '--accent-soft', '--text-onaccent', '--danger', '--warn', '--success',
  '--font-sans', '--font-mono',
] as const

export function readThemeKey(): string {
  if (typeof document === 'undefined') return ''
  const root = document.documentElement
  const style = getComputedStyle(root)
  return [root.classList.contains('dark') ? 'dark' : 'light', ...THEME_TOKENS.map(name => style.getPropertyValue(name).trim())].join('|')
}

export function useThemeKey(): string {
  const [key, setKey] = useState(readThemeKey)
  useEffect(() => {
    const root = document.documentElement
    const sync = () => {
      const next = readThemeKey()
      setKey(current => current === next ? current : next)
    }
    sync()
    const observer = new MutationObserver(sync)
    observer.observe(root, { attributes: true, attributeFilter: ['class', 'style', 'data-theme-color'] })
    return () => observer.disconnect()
  }, [])
  return key
}
