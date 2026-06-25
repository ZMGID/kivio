// `knowledge_search` 工具结果解析：把 structured_content.hits 转成来源卡视图模型。
// 拆成独立模块（非组件），避免 ToolCallBlock.tsx 触发 react-refresh/only-export-components。
import type { ToolCallRecord } from './types'

export interface KbHitView {
  n: number
  docName: string
  headingPath?: string
  score: number
  text: string
}

function rawToolName(toolCall: ToolCallRecord): string {
  return toolCall.tool_name || toolCall.toolName || toolCall.name || ''
}

function asObject(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null
}

function asString(value: unknown): string {
  return typeof value === 'string' ? value : ''
}

function asNumber(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : 0
}

/** Parse `knowledge_search` structured hits into source-card view models. */
export function knowledgeSearchHits(toolCall: ToolCallRecord): KbHitView[] | null {
  if (rawToolName(toolCall) !== 'knowledge_search') return null
  const structured = asObject(toolCall.structured_content ?? toolCall.structuredContent)
  const hits = structured?.hits
  if (!Array.isArray(hits) || hits.length === 0) return null
  const views = hits
    .map((raw): KbHitView | null => {
      const o = asObject(raw)
      if (!o) return null
      const text = asString(o.text)
      if (!text) return null
      return {
        n: asNumber(o.n),
        docName: asString(o.docName) || asString(o.doc_name) || '',
        headingPath: asString(o.headingPath) || asString(o.heading_path) || undefined,
        score: asNumber(o.score),
        text,
      }
    })
    .filter((view): view is KbHitView => Boolean(view))
  return views.length > 0 ? views : null
}
