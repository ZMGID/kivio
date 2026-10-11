import { normalizeToolCallStatus } from '../toolStatus'
import { canonicalToolName } from '../segments'
import type { ToolCallRecord } from '../types'
import {
  isCollaborationTool,
  type CollaborationToolName,
  type Delivery,
  type DeliveryStatus,
  type RoomStatus,
  type PendingAction,
} from './collaboration'

const profileTools = ['bot_create', 'bot_update', 'bot_get', 'bot_list', 'bot_set_primary', 'bot_memory_read', 'bot_memory_update', 'bot_memory_clear'] as const
export type BotActivityToolName = CollaborationToolName | typeof profileTools[number]
export function isBotActivityTool(name: string): name is BotActivityToolName {
  return isCollaborationTool(name) || (profileTools as readonly string[]).includes(name)
}

export type CollaborationCard = {
  tool: BotActivityToolName
  name?: string
  body?: string
  status?: DeliveryStatus | RoomStatus
  exchangeId?: string
  deliveryIds?: string[]
  pendingIds?: string[]
  deliveryId?: string
  error?: string
  count?: number
  attachmentCount?: number
  pending: boolean
}

function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : null
}

function text(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() ? value : undefined
}

export function toolPayload(toolCall: ToolCallRecord): unknown {
  const structured = toolCall.structured_content ?? toolCall.structuredContent
  if (record(structured) || Array.isArray(structured)) return structured
  const raw = toolCall.result ?? toolCall.output ?? toolCall.result_preview ?? toolCall.resultPreview ?? toolCall.error
  if (typeof raw !== 'string') return raw
  const trimmed = raw.trim()
  if (!trimmed.startsWith('{') && !trimmed.startsWith('[')) return raw
  try {
    return JSON.parse(trimmed) as unknown
  } catch {
    return raw
  }
}

function toolArgs(toolCall: ToolCallRecord): Record<string, unknown> | null {
  const value = toolCall.arguments ?? toolCall.args ?? toolCall.input
  if (typeof value === 'string') {
    try {
      return record(JSON.parse(value))
    } catch {
      return null
    }
  }
  return record(value)
}

function deliveryCard(tool: CollaborationToolName, payload: Record<string, unknown>, fallbackBody: string | undefined): CollaborationCard | null {
  const status = text(payload.status)
  const exchangeId = text(payload.exchangeId)
  if (!status || !exchangeId) return null
  return {
    tool,
    name: text(payload.toName),
    body: text(payload.body) ?? text(payload.error) ?? fallbackBody,
    status: status as DeliveryStatus,
    exchangeId,
    deliveryId: text(payload.id),
    error: text(payload.error),
    attachmentCount: Array.isArray(payload.attachmentIds) ? payload.attachmentIds.length : undefined,
    pending: false,
  }
}

/**
 * The tool result freezes the status from send time. Once the collaboration
 * snapshot contains that delivery, its status and error replace the frozen copy.
 * Pending calls and failures that never created a record stay on the tool result.
 */
export function deliveryCardState(
  card: CollaborationCard,
  deliveries: readonly Pick<Delivery, 'id' | 'status' | 'error'>[],
  actions: readonly Pick<PendingAction, 'id' | 'status'>[] = [],
): Pick<CollaborationCard, 'status' | 'error' | 'pending'> {
  if (card.pendingIds?.length) {
    const states = card.pendingIds.map(id => actions.find(action => action.id === id)?.status)
    if (states.every(state => state && state !== 'awaiting_user')) {
      return { status: states.includes('rejected') ? 'cancelled' : 'succeeded', pending: false }
    }
  }
  if (card.deliveryIds?.length) {
    const records = card.deliveryIds.map(id => deliveries.find(delivery => delivery.id === id))
    const problem = records.find(delivery => delivery && ['failed', 'unavailable', 'interrupted', 'cancelled'].includes(delivery.status))
    if (problem) return { status: problem.status, error: problem.error || undefined, pending: false }
    const status = records.every(delivery => delivery?.status === 'succeeded') ? 'succeeded'
      : records.some(delivery => delivery?.status === 'awaiting_user') ? 'awaiting_user'
      : records.some(delivery => delivery && ['running', 'claimed'].includes(delivery.status)) ? 'running' : 'queued'
    return { status, pending: false }
  }
  const live = card.deliveryId
    ? deliveries.find((delivery) => delivery.id === card.deliveryId)
    : undefined
  if (!live) return { status: card.status, error: card.error, pending: card.pending }
  const error = live.error?.trim()
  return { status: live.status, error: error || undefined, pending: false }
}

export function collaborationCard(toolCall: ToolCallRecord): CollaborationCard | null {
  const name = canonicalToolName(toolCall)
  if (!isBotActivityTool(name)) return null
  const args = toolArgs(toolCall)
  const payload = toolPayload(toolCall)
  const fallbackBody = text(args?.body)
  const toolStatus = normalizeToolCallStatus(toolCall.status)
  const pending = toolStatus === 'pending' || toolStatus === 'running'
  const object = record(payload)
  const error = text(toolCall.error) ?? text(object?.error)
  if (!isCollaborationTool(name)) {
    return { tool: name, name: text(object?.name) ?? text(args?.name), pending, error: error ?? (toolStatus === 'error' ? text(payload) : undefined) }
  }
  if (typeof payload === 'string') {
    return { tool: name, body: fallbackBody, error: payload, pending: false }
  }
  if (name === 'bot_room_broadcast' && Array.isArray(object?.deliveryIds)) {
    const ids = object.deliveryIds.filter((id): id is string => typeof id === 'string')
    return { tool: name, deliveryIds: ids, status: ids.length ? 'queued' : undefined, pending, error }
  }
  if (name === 'bot_peer_send' || name === 'bot_peer_reply' || name === 'bot_room_mention' || name === 'bot_room_handoff' || name === 'bot_room_broadcast') {
    const card = object ? deliveryCard(name, object, fallbackBody) : null
    return card ?? { tool: name, body: fallbackBody, pending, error }
  }
  if (name === 'bot_exchange_query') {
    const id = text(object?.id) ?? text(args?.id)
    const entries = Array.isArray(object?.entries) ? object.entries : []
    const first = record(entries[0])
    return { tool: name, exchangeId: id, count: entries.length, body: text(first?.body), pending, error }
  }
  if (name === 'bot_room_members') {
    const pendingActions = Array.isArray(object?.pending) ? object.pending : []
    return { tool: name, count: pendingActions.length, pendingIds: pendingActions.map(action => text(record(action)?.id)).filter((id): id is string => Boolean(id)), status: pendingActions.length ? 'awaiting_user' : undefined, pending, error }
  }
  if (name === 'bot_room_log') {
    const entries = Array.isArray(payload) ? payload : []
    const last = record(entries[entries.length - 1])
    return { tool: name, count: entries.length, body: text(last?.body), pending, error }
  }
  const status = text(object?.status) as RoomStatus | undefined
  return {
    tool: name,
    name: text(object?.title),
    status,
    pending,
    error,
  }
}