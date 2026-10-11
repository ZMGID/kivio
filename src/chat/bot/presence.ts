import type { BlobMood } from '../KivioBlob'
import type { I18n } from '../../components/i18n'
import type { DeliveryStatus, RoomStatus } from './collaboration'

export type BotSignal = 'working' | 'attention' | 'quiet'

const IN_FLIGHT = new Set<DeliveryStatus>(['queued', 'claimed', 'running'])
const ATTENTION = new Set<DeliveryStatus>(['awaiting_user', 'failed', 'interrupted', 'unavailable'])

export function contactSignal(input: {
  generating: boolean
  pending: boolean
  deliveries: readonly { id: string; status: DeliveryStatus; updatedAt: number }[]
  /** Private approval or question. Wins over an in-flight working mark. */
  attention?: boolean
}): BotSignal {
  if (input.attention || input.pending || input.deliveries.some(item => item.status === 'awaiting_user')) return 'attention'
  if (input.generating || input.deliveries.some((delivery) => IN_FLIGHT.has(delivery.status))) return 'working'
  const latest = [...input.deliveries].sort((left, right) => right.updatedAt - left.updatedAt || right.id.localeCompare(left.id))[0]
  return latest && ATTENTION.has(latest.status) ? 'attention' : 'quiet'
}

export function deliveryStatusLabel(status: DeliveryStatus, t: I18n): string {
  switch (status) {
    case 'queued': return t.chatBotDeliveryQueued
    case 'awaiting_user': return t.chatBotNeedsYou
    case 'claimed': return t.chatBotDeliveryClaimed
    case 'running': return t.chatBotDeliveryRunning
    case 'succeeded': return t.chatBotDeliverySucceeded
    case 'failed': return t.chatBotDeliveryFailed
    case 'cancelled': return t.chatBotDeliveryCancelled
    case 'interrupted': return t.chatBotDeliveryInterrupted
    case 'unavailable': return t.chatBotDeliveryUnavailable
  }
}

export function roomStatusLabel(status: RoomStatus, t: I18n): string {
  switch (status) {
    case 'active': return t.chatBotGroupActive
    case 'paused': return t.chatBotGroupPaused
    case 'stopped': return t.chatBotGroupStopped
  }
}

/** Approval wins over both execution and failure; unknown active work stays generic. */
export function contactAvatarMood(input: Parameters<typeof contactSignal>[0] & { error?: boolean }): BlobMood {
  if (input.attention || input.pending || input.deliveries.some(item => item.status === 'awaiting_user')) return 'wait'
  if (input.error) return 'error'
  if (contactSignal(input) === 'working') return 'work'
  const latest = [...input.deliveries].sort((a, b) => b.updatedAt - a.updatedAt || b.id.localeCompare(a.id))[0]
  return latest && ATTENTION.has(latest.status) ? 'error' : 'idle'
}
