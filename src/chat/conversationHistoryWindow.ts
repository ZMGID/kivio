import { keepNewerContextMeasurement } from './contextPanel'
import { keepNewerTodoState } from './agentTodoState'
import type { ChatMessage, ChatToolArtifact, Conversation } from './types'
import { referencedArtifactIds } from './artifactReferences'
import { artifactId, artifactPresentationFromToolCall } from './artifactPresentation'

export const EMPTY_HISTORY_DIRECTORY: NonNullable<Conversation['history_directory']> = []
export const EMPTY_HISTORY_ARTIFACTS: ChatToolArtifact[] = []

export interface ConversationHistoryPage {
  revision: number
  history_source?: string
  start: number
  end: number
  total: number
  messages: ChatMessage[]
  history_directory?: Conversation['history_directory']
  history_artifacts?: ChatToolArtifact[]
}

/** Development adapter equivalent of the native artifact window projection. */
export function historyReferenceArtifacts(messages: ChatMessage[], start: number, end: number): ChatToolArtifact[] {
  const needed = new Set<string>()
  const present = new Set<string>()
  const artifactsOf = (message: ChatMessage) => [
    ...(message.artifacts ?? []),
    ...(message.tool_calls ?? message.toolCalls ?? []).flatMap(tool => tool.artifacts ?? []),
  ]
  for (const message of messages.slice(start, end)) {
    for (const id of referencedArtifactIds([message.content, ...(message.segments ?? []).map(segment => segment.text ?? '')])) needed.add(id)
    for (const tool of message.tool_calls ?? message.toolCalls ?? []) {
      for (const id of artifactPresentationFromToolCall(tool)?.artifactIds ?? []) needed.add(id)
    }
    for (const artifact of artifactsOf(message)) present.add(artifactId(artifact))
  }
  for (const id of present) needed.delete(id)
  if (!needed.size) return []
  const selected = new Map<string, ChatToolArtifact>()
  for (const message of messages) {
    for (const artifact of artifactsOf(message)) {
      const id = artifactId(artifact)
      if (needed.has(id)) selected.set(id, artifact)
    }
  }
  return [...selected.values()]
}

/** Match the native window edge so multi-answer arms do not start mid-group. */
export function historyWindowStart(messages: ChatMessage[], end: number, turnLimit = 10): number {
  let start = end
  let bytes = 0
  let turns = 0
  const encoder = new TextEncoder()
  while (start > Math.max(0, end - 500) && turns < turnLimit) {
    const size = encoder.encode(JSON.stringify(messages[start - 1])).byteLength
    // One oversized message must still be reachable. Its process details are
    // paged in MessageBubble, never truncated in the stored conversation.
    if (start < end && bytes + size > 1024 * 1024) break
    bytes += size
    start -= 1
    if (messages[start].role === 'user') turns++
  }
  if (!turns) start = Math.max(start, end - 75)
  if (start === 0 || start >= end || messages[start].role !== 'assistant') return start
  const group = messages[start].group_id ?? messages[start].groupId
  if (group) {
    const groupFloor = Math.max(0, start - 4)
    while (start > groupFloor
      && (messages[start - 1].group_id ?? messages[start - 1].groupId) === group) start -= 1
  }
  if (start > 0 && messages[start].role === 'assistant' && messages[start - 1].role === 'user') start -= 1
  return start
}

export function historyWindowEnd(messages: ChatMessage[], start: number, limit: number, turnLimit = 10): number {
  let end = start
  let bytes = 0
  const encoder = new TextEncoder()
  let turns = 0
  while (end < Math.min(start + 500, limit)) {
    if (messages[end].role === 'user' && turns >= turnLimit) break
    const size = encoder.encode(JSON.stringify(messages[end])).byteLength
    if (end > start && bytes + size > 1024 * 1024) break
    bytes += size
    if (messages[end].role === 'user') turns++
    end++
  }
  if (!turns) end = Math.min(end, start + 75)
  if (end > start && end < limit && messages[end - 1].role === 'user' && messages[end].role === 'assistant') end++
  const previous = messages[end - 1]
  const group = previous?.group_id ?? previous?.groupId
  if (group) {
    const ceiling = Math.min(end + 4, limit)
    while (end < ceiling && (messages[end].group_id ?? messages[end].groupId) === group) end++
  }
  return end
}

/** Pages only extend the same snapshot. Edits, truncations and background
 * completions change revision and must be reloaded instead of spliced in. */
export function mergeConversationHistoryPage(
  current: Conversation,
  page: ConversationHistoryPage,
): Conversation | null {
  if (current.history_start == null || current.history_total == null) return null
  if (current.history_source && page.history_source && page.history_source !== current.history_source) return null
  if (page.revision !== current.revision || page.total !== current.history_total
    || page.start < 0 || page.end > page.total || page.start >= page.end
    || page.messages.length !== page.end - page.start) return null
  let start = current.history_start
  let gap = current.history_gap
  let messages: ChatMessage[]
  if (page.end === start) {
    messages = [...page.messages, ...current.messages]
    start = page.start
  } else if (gap && (page.end === gap.end || (page.start === gap.start && page.end <= gap.end))) {
    const first = Math.max(page.start, gap.start)
    const split = gap.start - start
    messages = [...current.messages.slice(0, split), ...page.messages.slice(first - page.start), ...current.messages.slice(split)]
    if (page.end === gap.end) gap = first === gap.start ? undefined : { ...gap, end: first }
    else gap = { ...gap, start: page.end }
  } else {
    return null
  }
  return {
    ...current,
    messages,
    // Legacy pages lack source identity and must be rebound before durable replay.
    history_source: current.history_source && page.history_source,
    history_start: start,
    history_gap: gap,
    history_directory: [...new Map([
      ...(current.history_directory ?? []), ...(page.history_directory ?? []),
    ].map(entry => [entry.id, entry])).values()].sort((a, b) => a.message_index - b.message_index),
    history_artifacts: [...new Map([
      ...(current.history_artifacts ?? []), ...(page.history_artifacts ?? []),
    ].map(artifact => [artifactId(artifact), artifact])).values()],
  }
}

export function isPartialConversation(conversation: Conversation | null): boolean {
  return Boolean(conversation && ((conversation.history_start ?? 0) > 0 || conversation.history_gap))
}

/** A metadata-only response does not replace the messages or the paging state of the displayed snapshot. */
export function mergeConversationMetadata(current: Conversation, updated: Conversation): Conversation {
  return {
    ...keepNewerContextMeasurement(keepNewerTodoState(updated, current), current),
    messages: current.messages,
    history_gap: current.history_gap,
    history_start: current.history_start,
    history_source: updated.revision === current.revision ? current.history_source : undefined,
    history_total: current.history_total,
    history_turn_count: current.history_turn_count,
    history_usage: current.history_usage,
    history_directory: current.history_directory,
    history_artifacts: current.history_artifacts,
  }
}
