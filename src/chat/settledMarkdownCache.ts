import { normalizeMarkdownForRender, preserveLocalMarkdownLinks } from './markdownUtils'
import { normalizeLegacyCliReport } from './cliCommandReportData'
import { inlineArtifactReferenceLinks } from './artifactReferences'
import { parseMarkdownHeadingOutline, type ParsedMarkdownHeading } from './markdownHeadingOutline'

export type SettledMarkdownCacheEntry = {
  normalized: string
  /** Filled on demand when the conversation timeline requests an outline. */
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

function errorDetailsFence(detail: string): string {
  const longestRun = Math.max(0, ...Array.from(detail.matchAll(/`+/g), (match) => match[0].length))
  return '`'.repeat(Math.max(3, longestRun + 1))
}

// Older external-agent failures were persisted with literal HTML. ReactMarkdown intentionally
// does not enable raw HTML, so migrate only Kivio's exact legacy disclosure shape into the safe
// fenced block below. Arbitrary model-authored HTML remains inert text.
function normalizeLegacyErrorDetails(content: string): string {
  return content.replace(
    /<details>\s*<summary>错误详情<\/summary>\s*(`{3,})\s*\n([\s\S]*?)\n\1\s*<\/details>/g,
    (_match, _oldFence: string, detail: string) => {
      const fence = errorDetailsFence(detail)
      return `${fence}kivio-error-details\n${detail}\n${fence}`
    },
  )
}

export function buildMarkdownEntry(content: string): SettledMarkdownCacheEntry {
  return {
    normalized: preserveLocalMarkdownLinks(
      normalizeMarkdownForRender(inlineArtifactReferenceLinks(normalizeLegacyCliReport(normalizeLegacyErrorDetails(content)))),
    ),
  }
}

export function getSettledMarkdownOutline(content: string): SettledMarkdownCacheEntry & { outline: ParsedMarkdownHeading[] } {
  const entry = getSettledMarkdownCacheEntry(content, () => buildMarkdownEntry(content))
  const outline = entry.outline ?? parseMarkdownHeadingOutline(entry.normalized)
  if (!entry.outline) {
    entry.outline = outline
    touch(content, entry)
  }
  return { normalized: entry.normalized, outline }
}

export function clearSettledMarkdownCache(): void {
  entries.clear()
  sizes.clear()
  usedBytes = 0
}

export function settledMarkdownCacheSize(): number {
  return entries.size
}
