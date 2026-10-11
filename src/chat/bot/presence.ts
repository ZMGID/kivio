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
  if (input.attention) return 'attention'
  if (input.generating || input.deliveries.some((delivery) => IN_FLIGHT.has(delivery.status))) return 'working'
  if (input.pending) return 'attention'
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
