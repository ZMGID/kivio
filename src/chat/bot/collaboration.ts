import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import Ajv from 'ajv/dist/2020'
import collaborationSchema from '../../generated/botCollaboration.schema.json'
import type { CollaborationSnapshot, Exchange, MemberSession, Room } from '../../generated/botCollaboration'
import { isTauriRuntime } from '../utils'

export { isDedicatedMemberConversation } from '../botIdentity'
export type {
  Actor,
  CollaborationSnapshot,
  Delivery,
  DeliveryKind,
  DeliveryStatus,
  Exchange,
  ExchangeEntry,
  FinishReport,
  LogKind,
  MemberConversationIdentity,
  MemberSession,
  MemberState,
  PendingAction,
  PendingKind,
  PendingStatus,
  Receipt,
  ReceiptState,
  Room,
  RoomLogEntry,
  RoomMember,
  RoomStatus,
} from '../../generated/botCollaboration'

export const EMPTY_COLLABORATION: CollaborationSnapshot = {
  deliveries: [],
  rooms: [],
  exchanges: [],
  pending: [],
}

export type DecodeResult =
  | { ok: true; snapshot: CollaborationSnapshot }
  | { ok: false; error: string }

const UNREADABLE = '协作状态无法读取'

const ajv = new Ajv({ strict: false, allErrors: false, useDefaults: true, validateSchema: false, logger: false })
for (const format of ['int32', 'int64', 'uint32', 'uint64']) ajv.addFormat(format, true)

const snapshotValidator = ajv.compile<CollaborationSnapshot>(collaborationSchema)
const roomValidator = ajv.compile<Room>({ $ref: '#/$defs/Room', $defs: collaborationSchema.$defs })
const exchangeValidator = ajv.compile<Exchange>({ $ref: '#/$defs/Exchange', $defs: collaborationSchema.$defs })

function checked<T>(validator: { (data: unknown): data is T }, value: unknown): T | null {
  return validator(value) ? value : null
}

export function decodeExchange(value: unknown): Exchange | null {
  return checked(exchangeValidator, value)
}

export function decodeRoom(value: unknown): Room | null {
  return checked(roomValidator, value)
}

function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : null
}

export function decodeMemberSession(value: unknown): MemberSession | null {
  const item = record(value)
  if (!item || typeof item.roomId !== 'string' || typeof item.botId !== 'string'
    || typeof item.name !== 'string' || typeof item.conversationId !== 'string'
    || typeof item.created !== 'boolean') return null
  if (item.assistantId != null && typeof item.assistantId !== 'string') return null
  return item as MemberSession
}

export function decodeSnapshot(value: unknown): DecodeResult {
  const snapshot = checked(snapshotValidator, value)
  if (!snapshot) return { ok: false, error: UNREADABLE }
  return { ok: true, snapshot }
}

export const COLLABORATION_TOOLS = [
  'bot_peer_send',
  'bot_peer_reply',
  'bot_exchange_query',
  'bot_room_mention',
  'bot_room_broadcast',
  'bot_room_handoff',
  'bot_room_members',
  'bot_room_log',
  'bot_room_pause',
  'bot_room_continue',
] as const

export type CollaborationToolName = (typeof COLLABORATION_TOOLS)[number]

export function isCollaborationTool(name: string): name is CollaborationToolName {
  return (COLLABORATION_TOOLS as readonly string[]).includes(name)
}

export const collaborationApi = {
  snapshot: () => invoke<unknown>('bot_collaboration_snapshot'),
  peerSend: (toBotId: string, body: string, roomId?: string) =>
    invoke<unknown>('bot_peer_send', { toBotId, body, roomId: roomId ?? null }),
  exchange: (exchangeId: string) => invoke<unknown>('bot_exchange_query', { exchangeId }),
  createRoom: (title: string, botIds: string[]) => invoke<unknown>('bot_room_create', { title, botIds }),
  setMembers: (roomId: string, botIds: string[]) => invoke<unknown>('bot_room_set_members', { roomId, botIds }),
  mention: (roomId: string, toBotId: string, body: string) =>
    invoke<unknown>('bot_room_mention', { roomId, toBotId, body }),
  /** One round to current active members. Registered name requested from BotCollaboration. */
  broadcast: (roomId: string, body: string) => invoke<unknown>('bot_room_broadcast', { roomId, body }),
  renameRoom: (roomId: string, title: string) => invoke<unknown>('bot_room_rename', { roomId, title }),
  pauseRoom: (roomId: string) => invoke<unknown>('bot_room_pause', { roomId }),
  continueRoom: (roomId: string) => invoke<unknown>('bot_room_continue', { roomId }),
  stopRoom: (roomId: string) => invoke<unknown>('bot_room_stop', { roomId }),
  resolve: (actionId: string, approve: boolean) =>
    invoke<unknown>('bot_collaboration_resolve', { actionId, approve }),
  /** Opens or creates the private member thread. Navigation uses the returned id. */
  memberSession: async (roomId: string, botId: string) => {
    const session = decodeMemberSession(await invoke<unknown>('bot_room_member_session', { roomId, botId }))
    if (!session) throw new Error('群内会话无法打开')
    return session
  },
  onChanged: (listener: (payload: unknown) => void) => {
    if (!isTauriRuntime()) return Promise.resolve(() => {})
    return listen<unknown>('bot-collaboration-changed', (event) => listener(event.payload))
  },
}
