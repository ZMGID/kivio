import { toString } from 'mdast-util-to-string'
import remarkParse from 'remark-parse'
import { unified } from 'unified'

export type MarkdownHeadingDepth = 1 | 2 | 3

export type ParsedMarkdownHeading = {
  depth: MarkdownHeadingDepth
  title: string
  ordinal: number
}

export type MarkdownHeadingOutlineItem = ParsedMarkdownHeading & {
  sourceId: string
  anchorId: string
}

type MdastHeading = {
  type: 'heading'
  depth: number
}

type MdastRoot = {
  children: unknown[]
}

function isTopLevelHeading(value: unknown): value is MdastHeading {
  if (!value || typeof value !== 'object') return false
  const candidate = value as { type?: unknown; depth?: unknown }
  return candidate.type === 'heading'
    && typeof candidate.depth === 'number'
    && candidate.depth >= 1
    && candidate.depth <= 3
}

/**
 * 目录只采用根级 Markdown heading。这样 fenced code 自动不是 heading，引用/列表内的
 * heading 也不会污染答案目录；原始 HTML details 块由 remark 当作 HTML block 保留。
 */
export function parseMarkdownHeadingOutline(source: string): ParsedMarkdownHeading[] {
  if (!source.trim()) return []
  const tree = unified().use(remarkParse).parse(source) as unknown as MdastRoot
  const headings: ParsedMarkdownHeading[] = []
  for (const node of tree.children) {
    if (!isTopLevelHeading(node)) continue
    const title = toString(node as Parameters<typeof toString>[0]).replace(/\s+/g, ' ').trim()
    if (!title) continue
    headings.push({
      depth: node.depth as MarkdownHeadingDepth,
      title,
      ordinal: headings.length,
    })
  }
  return headings
}

export function markdownHeadingAnchorId(sourceId: string, ordinal: number): string {
  // Streamdown 的 sanitize 阶段会给 Markdown heading id 加 `user-content-` 前缀；目录
  // 用最终 DOM id，remark 插件则写入未加前缀的 source id（见 markdownHeadingSourceId）。
  return `user-content-${markdownHeadingSourceId(sourceId, ordinal)}`
}

export function markdownHeadingSourceId(sourceId: string, ordinal: number): string {
  return `chat-heading-${encodeURIComponent(sourceId)}-${ordinal}`
}

export function outlineItemsForSource(sourceId: string, source: string): MarkdownHeadingOutlineItem[] {
  return parseMarkdownHeadingOutline(source).map((heading) => ({
    ...heading,
    sourceId,
    anchorId: markdownHeadingAnchorId(sourceId, heading.ordinal),
  }))
}

export function primaryHeadingDepth(items: readonly MarkdownHeadingOutlineItem[]): MarkdownHeadingDepth | null {
  if (items.length === 0) return null
  return items.reduce<MarkdownHeadingDepth>(
    (depth, item) => Math.min(depth, item.depth) as MarkdownHeadingDepth,
    items[0].depth,
  )
}
