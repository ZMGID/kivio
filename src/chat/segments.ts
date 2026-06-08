import type { ChatMessageSegment } from './types'

export function segmentToolCallId(segment: ChatMessageSegment): string {
  return segment.tool_call_id ?? segment.toolCallId ?? ''
}

export function segmentStepNumber(segment: ChatMessageSegment): number | null | undefined {
  return segment.step_number ?? segment.stepNumber
}

function segmentDisplayRank(segment: ChatMessageSegment): number {
  if (segment.kind === 'reasoning') return 0
  if (segment.kind === 'text') return 1
  return 2
}

export function compareTimelineSegments(
  a: ChatMessageSegment,
  b: ChatMessageSegment,
): number {
  const aStepNumber = segmentStepNumber(a)
  const bStepNumber = segmentStepNumber(b)
  const sameModelStep =
    aStepNumber != null &&
    aStepNumber === bStepNumber &&
    (a.round ?? null) === (b.round ?? null) &&
    a.phase === b.phase
  if (sameModelStep) {
    const rankDelta = segmentDisplayRank(a) - segmentDisplayRank(b)
    if (rankDelta !== 0) return rankDelta
  }
  return a.order - b.order
}
