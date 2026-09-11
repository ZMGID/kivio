import { compareTimelineSegments } from './segments'
import type { ChatMessage, ChatMessageSegment } from './types'

/** Display/copy projection only: never rewrite stored content or model transcripts. */
export function messageBodySegments(message: Pick<ChatMessage, 'content' | 'segments'>): ChatMessageSegment[] {
  const parts: string[] = []
  const rawParts: string[] = []
  const finalParts: string[] = []
  const segments = [...(message.segments ?? [])].sort(compareTimelineSegments).filter(segment => {
    if (segment.kind !== 'text') return true
    rawParts.push(segment.text ?? '')
    if (!segment.text?.trim()) return true
    const text = segment.text.trim()
    // normalize_assistant_segments appended this known fallback when there was no
    // Plain/Synthesis text. Only suppress a proven copy of preceding body text.
    if (/^seg_\d+_synthesis_text$/.test(segment.id)
      && (segment.phase === 'plain' || segment.phase === 'synthesis')
      && text === parts.join('\n\n')) return false
    parts.push(text)
    if (segment.phase === 'plain' || segment.phase === 'synthesis') finalParts.push(text)
    return true
  })
  // The stored content may be a mirror of a segment, a full body, or an answer
  // absent from an older/partial timeline. Preserve the last case as a fallback.
  const content = message.content.trim()
  const isContentMirror = parts.includes(content)
    || content === parts.join('\n\n')
    // Live content concatenates raw deltas; persisted content joins only
    // Plain/Synthesis text. Neither mirror is an additional body segment.
    || content === rawParts.join('').trim()
    || content === finalParts.join('\n\n')
  if (segments.length && content && !isContentMirror) {
    segments.push({
      id: 'body-content-fallback', kind: 'text', phase: 'plain',
      order: segments.reduce((max, segment) => Math.max(max, segment.order), 0) + 1,
      text: message.content,
    })
  }
  return segments
}

export function messageBodyText(message: Pick<ChatMessage, 'role' | 'content' | 'segments'>): string {
  if (message.role !== 'assistant') return message.content
  const body = messageBodySegments(message)
    .filter(segment => segment.kind === 'text')
    .map(segment => (segment.text ?? '').trim())
    .filter(Boolean)
    .join('\n\n')
  return body || message.content
}
