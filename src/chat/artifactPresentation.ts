import type { ChatToolArtifact, ToolCallRecord } from './types'
import { normalizeToolCallStatus } from './toolStatus'

export interface ArtifactPresentation {
  artifactIds: string[]
  caption?: string
  /** Missing on historical records, which keep their original visible delivery. */
  mode?: 'prepare' | 'preview'
}

export function artifactId(artifact: ChatToolArtifact): string {
  return typeof artifact.id === 'string' ? artifact.id.trim() : ''
}

function isNativePresentationTool(toolCall: ToolCallRecord): boolean {
  if (toolCall.source !== 'native') return false
  const name = toolCall.tool_name || toolCall.toolName || toolCall.name || ''
  return name === 'present_artifacts'
}

export function artifactPresentationFromToolCall(
  toolCall: ToolCallRecord,
): ArtifactPresentation | null {
  if (!isNativePresentationTool(toolCall)) return null
  const structured = toolCall.structured_content ?? toolCall.structuredContent
  if (!structured || typeof structured !== 'object') return null
  const value = structured as {
    type?: unknown
    artifactIds?: unknown
    artifact_ids?: unknown
    caption?: unknown
    mode?: unknown
  }
  if (value.type !== 'artifact_presentation') return null
  const rawIds = value.artifactIds ?? value.artifact_ids
  if (!Array.isArray(rawIds)) return { artifactIds: [] }
  const artifactIds = Array.from(new Set(
    rawIds
      .filter((id): id is string => typeof id === 'string')
      .map((id) => id.trim())
      .filter(Boolean),
  ))
  const caption = typeof value.caption === 'string' ? value.caption.trim() : ''
  const mode = value.mode === 'prepare' || value.mode === 'preview' ? value.mode : undefined
  return { artifactIds, ...(caption ? { caption } : {}), ...(mode ? { mode } : {}) }
}

export interface HtmlRenderReference {
  attachmentId: string
  title: string
  height: number
}

export function htmlRenderReferenceFromToolCall(toolCall: ToolCallRecord): HtmlRenderReference | null {
  if (normalizeToolCallStatus(toolCall.status) !== 'completed') return null
  const structured = toolCall.structured_content ?? toolCall.structuredContent
  if (!structured || typeof structured !== 'object'
    || !('type' in structured) || structured.type !== 'html_render'
    || !('attachmentId' in structured) || typeof structured.attachmentId !== 'string'
    || !('title' in structured) || typeof structured.title !== 'string'
    || !('height' in structured) || typeof structured.height !== 'number' || !Number.isFinite(structured.height)) return null
  return { attachmentId: structured.attachmentId, title: structured.title.trim() || 'HTML', height: structured.height }
}

export function isPresentationToolCall(toolCall: ToolCallRecord): boolean {
  if (isNativePresentationTool(toolCall)) return true
  const structured = toolCall.structured_content ?? toolCall.structuredContent
  return normalizeToolCallStatus(toolCall.status) === 'completed'
    && !!structured && typeof structured === 'object' && 'type' in structured && structured.type === 'html_render'
}

export function isVisiblePresentation(toolCall: ToolCallRecord): boolean {
  return isPresentationToolCall(toolCall) && artifactPresentationFromToolCall(toolCall)?.mode !== 'prepare'
}
