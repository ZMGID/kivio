import type { ParsedMarkdownHeading } from './markdownHeadingOutline'

export type SettledMarkdownCacheEntry = {
  normalized: string
  /** 只在已挂载的正式回答请求目录时填充，避免普通 Markdown 多做一次 AST 解析。 */
  outline?: ParsedMarkdownHeading[]
}

const MAX_ENTRIES = 96
// Text is budgeted at two bytes per character; heading objects include estimated overhead.
const MAX_BYTES = 4 * 1024 * 1024
const sizes = new Map<string, number>()
let usedBytes = 0
const entries = new Map<string, SettledMarkdownCacheEntry>()

function touch(key: string, entry: SettledMarkdownCacheEntry): SettledMarkdownCacheEntry {
  const bytes = 2 * (key.length + entry.normalized.length + (entry.outline ?? []).reduce((sum, heading) => sum + heading.title.length, 0)) + (entry.outline?.length ?? 0) * 64
  usedBytes -= sizes.get(key) ?? 0
  sizes.delete(key)
  entries.delete(key)
  if (bytes > MAX_BYTES / 4) return entry
  entries.set(key, entry)
  sizes.set(key, bytes)
  usedBytes += bytes
  while (entries.size > MAX_ENTRIES || usedBytes > MAX_BYTES) {
    const oldest = entries.keys().next().value
    if (oldest === undefined) break
    usedBytes -= sizes.get(oldest) ?? 0
    sizes.delete(oldest)
    entries.delete(oldest)
  }
  return entry
}

export function getSettledMarkdownCacheEntry(
  content: string,
  build: () => SettledMarkdownCacheEntry,
): SettledMarkdownCacheEntry {
  const cached = entries.get(content)
  if (cached) return touch(content, cached)
  return touch(content, build())
}

export function getSettledMarkdownOutline(
  content: string,
  build: () => SettledMarkdownCacheEntry,
  buildOutline: (normalized: string) => ParsedMarkdownHeading[],
): SettledMarkdownCacheEntry {
  const entry = getSettledMarkdownCacheEntry(content, build)
  if (entry.outline !== undefined) return entry
  entry.outline = buildOutline(entry.normalized)
  return touch(content, entry)
}

export function clearSettledMarkdownCache(): void {
  entries.clear()
  sizes.clear()
  usedBytes = 0
}

export function settledMarkdownCacheSize(): number {
  return entries.size
}
