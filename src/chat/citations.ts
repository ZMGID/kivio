// 知识库引用 `[n]` 的 mdast 切分逻辑 + remark 插件 + 引用索引。
// 抽成独立模块（非组件），既便于单测，也避免 ChatMarkdown.tsx 触发
// react-refresh/only-export-components。
import type { ToolCallRecord } from './types'
import { knowledgeSearchHits, type KbHitView } from './knowledgeBaseHits'
import { isWebCitation, webSearchCardView, type WebCitationRef } from './webSearchCitations'

/** 答案正文 `[n]` 角标可查到的来源：知识库命中（KB）或联网搜索来源（web）。 */
export type CitationView = KbHitView | WebCitationRef
export type CitationKey = number | string

/** Tool progress can replace the container without changing any visible source. */
export function citationMapsEqual(
  left: ReadonlyMap<CitationKey, CitationView> | undefined,
  right: ReadonlyMap<CitationKey, CitationView> | undefined,
): boolean {
  if (left === right) return true
  if ((left?.size ?? 0) !== (right?.size ?? 0)) return false
  for (const [n, a] of left ?? []) {
    const b = right?.get(n)
    if (!b) return false
    if (a === b) continue
    if (isWebCitation(a)) {
      if (!isWebCitation(b) || a.n !== b.n || a.title !== b.title || a.url !== b.url
        || a.host !== b.host || a.snippet !== b.snippet || a.publishedDate !== b.publishedDate) return false
    } else if (isWebCitation(b) || a.n !== b.n || a.docName !== b.docName
      || a.citation !== b.citation || a.kbId !== b.kbId || a.docId !== b.docId || a.page !== b.page
      || a.headingPath !== b.headingPath || a.score !== b.score || a.text !== b.text) return false
  }
  return true
}

/** 按唯一来源标记索引新片段，历史记录继续使用数字。 */
export function buildCitationMap(toolCalls: ToolCallRecord[]): Map<CitationKey, CitationView> {
  const map = new Map<CitationKey, CitationView>()
  const knowledgeHits = new Map<string, KbHitView>()
  for (const toolCall of toolCalls) {
    for (const hit of knowledgeSearchHits(toolCall) ?? []) {
      if (hit.citation) knowledgeHits.set(hit.citation, hit)
      else map.set(hit.n, hit)
    }
  }
  for (const toolCall of toolCalls) {
    const view = webSearchCardView(toolCall)
    if (!view) continue
    for (const citation of view.citations) {
      const existing = map.get(citation.n)
      if (!existing || isWebCitation(existing)) {
        map.set(citation.n, { kind: 'web', ...citation })
      }
    }
  }
  let ordinal = 0
  for (const key of map.keys()) {
    if (typeof key === 'number') ordinal = Math.max(ordinal, key)
  }
  for (const [key, hit] of knowledgeHits) {
    map.set(key, { ...hit, n: ++ordinal })
  }
  return map
}

// 极简 mdast 节点视图：只需 type/value/url/children 来切分文本节点。
export interface MdNode {
  type: string
  value?: string
  url?: string
  children?: MdNode[]
}

export function citationKey(marker: string): CitationKey {
  return marker.startsWith('kb:') ? marker : Number(marker)
}

/** 把一个文本节点里的 `[n]`（且 n 是有效引用）切成 text / link 混排。 */
export function splitCitations(value: string, validNs: Set<number>): MdNode[] {
  const out: MdNode[] = []
  let last = 0
  const re = /\[(\d{1,3}|kb:[a-f0-9]{32}-[1-9]\d?)\]/g
  let m: RegExpExecArray | null
  while ((m = re.exec(value))) {
    const key = citationKey(m[1])
    if (typeof key === 'number' && !validNs.has(key)) continue
    if (m.index > last) out.push({ type: 'text', value: value.slice(last, m.index) })
    out.push({ type: 'link', url: `#kb-cite-${key}`, children: [{ type: 'text', value: m[0] }] })
    last = m.index + m[0].length
  }
  if (last < value.length) out.push({ type: 'text', value: value.slice(last) })
  return out
}

/** remark 插件：遍历树，把 text 节点里的有效 `[n]` 换成 `#kb-cite-n` 链接。
 *  跳过 link/code，避免嵌套链接或污染代码。 */
export function remarkCitations(validNs: Set<number>) {
  const walk = (node: MdNode) => {
    if (!node.children || node.type === 'link' || node.type === 'linkReference') return
    const next: MdNode[] = []
    for (const child of node.children) {
      if (child.type === 'text' && child.value?.includes('[')) {
        next.push(...splitCitations(child.value, validNs))
      } else {
        walk(child)
        next.push(child)
      }
    }
    node.children = next
  }
  return () => (tree: MdNode) => walk(tree)
}
