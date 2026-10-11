import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type {
  ApprovalSource,
  BotApprovalWrite,
  BotCapabilityWrite,
  BotCreateRequest,
  BotEditRequest,
  BotListItem,
  BotMemoryView,
  BotModelWrite,
  BotProfileDetail,
  ModelSource,
} from '../../generated/botProfile'
import { isTauriRuntime } from '../utils'
import type { ChatAssistant } from '../types'

export type {
  ApprovalSource,
  BotCreateRequest,
  BotEditRequest,
  BotListItem,
  BotMemoryView,
  BotProfileDetail,
  CapabilitySource,
  ModelSource,
  ResolvedModelSource,
} from '../../generated/botProfile'

export type MemoryLayer = 'l1' | 'l2'
export type BotPersonaWrite = Pick<BotCreateRequest, 'name' | 'description' | 'systemPrompt' | 'icon' | 'color'>

function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : null
}

function whole(value: unknown): number | null {
  return typeof value === 'number' && Number.isInteger(value) ? value : null
}

function revisionOf(value: unknown): number | null {
  const revision = whole(value)
  return revision != null && revision >= 0 ? revision : null
}

function ids(value: unknown): boolean {
  return Array.isArray(value) && value.every((item) => typeof item === 'string')
}

function notes(value: unknown): boolean {
  return Array.isArray(value) && value.every((entry) => {
    const note = record(entry)
    return note != null && typeof note.id === 'string' && typeof note.content === 'string' && whole(note.createdAt) != null
  })
}

const CAPABILITY_SOURCES = ['granted_defaults', 'explicit', 'legacy_whitelist'] as const
const MODEL_SOURCES = ['inherit_global', 'explicit'] as const
const RESOLVED_MODELS = ['inherit_global', 'explicit', 'conversation_frozen'] as const
const APPROVAL_SOURCES = ['inherit_global', 'override'] as const

function oneOf(value: unknown, allowed: readonly string[]): boolean {
  return typeof value === 'string' && allowed.includes(value)
}

export function isProfileConflict(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error)
  return message.includes('没有覆盖')
}

export function profileEditRequest(
  assistantId: string,
  expectedRevision: number,
  patch: Omit<BotEditRequest, 'assistantId' | 'expectedRevision'>,
): BotEditRequest {
  return { assistantId, expectedRevision, ...patch }
}

function textOrNull(value: unknown): boolean {
  return value == null || typeof value === 'string'
}

export function decodeProfileDetail(value: unknown): BotProfileDetail | null {
  const detail = record(value)
  const profile = record(detail?.profile)
  const capability = record(profile?.capability)
  const model = record(profile?.model)
  const resolved = record(detail?.resolved)
  const approval = record(profile?.approval)
  if (!detail || !profile || !capability || !model || !resolved || !approval) return null
  const revision = revisionOf(profile.revision)
  const version = whole(profile.version)
  if (revision == null || version == null || version < 0 || whole(profile.createdAt) == null || whole(profile.updatedAt) == null
    || typeof profile.assistantId !== 'string' || typeof profile.primary !== 'boolean'
    || !oneOf(capability.source, CAPABILITY_SOURCES) || typeof capability.explanation !== 'string'
    || (capability.mcpServerIds != null && !ids(capability.mcpServerIds))
    || (capability.skillIds != null && !ids(capability.skillIds))
    || !oneOf(model.source, MODEL_SOURCES) || typeof model.explanation !== 'string'
    || !oneOf(approval.source, APPROVAL_SOURCES) || typeof approval.explanation !== 'string'
    || !textOrNull(approval.policy) || !notes(profile.sharedMemory)
    || typeof detail.assistantId !== 'string' || typeof detail.name !== 'string'
    || typeof detail.description !== 'string' || typeof detail.icon !== 'string' || typeof detail.color !== 'string'
    || typeof detail.systemPrompt !== 'string' || typeof detail.conversationId !== 'string'
    || typeof detail.persistedProfile !== 'boolean' || typeof detail.providerId !== 'string'
    || typeof detail.model !== 'string' || typeof detail.resolvedProviderId !== 'string'
    || typeof detail.resolvedModel !== 'string' || !oneOf(detail.modelSource, RESOLVED_MODELS)
    || typeof detail.modelExplanation !== 'string'
    || !oneOf(resolved.source, CAPABILITY_SOURCES) || typeof resolved.explanation !== 'string'
    || !ids(resolved.mcpServerIds) || !ids(resolved.skillIds)
    || !ids(resolved.droppedMcpServerIds) || !ids(resolved.droppedSkillIds)
    || typeof resolved.rewriteSnapshot !== 'boolean') return null
  return detail as BotProfileDetail
}

export function decodeMemory(value: unknown): BotMemoryView | null {
  const view = record(value)
  if (!view || typeof view.assistantId !== 'string' || typeof view.globalInjection !== 'string'
    || !textOrNull(view.l1) || !textOrNull(view.l2) || !Array.isArray(view.sharedMemory)) return null
  return view as BotMemoryView
}

export function decodeBotList(value: unknown): BotListItem[] | null {
  if (!Array.isArray(value)) return null
  const items: BotListItem[] = []
  for (const entry of value) {
    const item = record(entry)
    if (!item || typeof item.assistantId !== 'string' || typeof item.name !== 'string'
      || typeof item.conversationId !== 'string' || typeof item.primary !== 'boolean'
      || typeof item.persistedProfile !== 'boolean' || typeof item.capabilityExplanation !== 'string'
      || !textOrNull(item.readError) || !textOrNull(item.capabilitySource)
      || !textOrNull(item.modelSource) || !textOrNull(item.approvalSource)) return null
    items.push(item as BotListItem)
  }
  return items
}

export function assistantFromProfile(detail: BotProfileDetail): ChatAssistant {
  const now = Math.floor(Date.now() / 1000)
  return {
    id: detail.assistantId,
    name: detail.name,
    description: detail.description,
    icon: detail.icon || 'bot',
    color: detail.color || '#6A8FBD',
    source: 'user',
    system_prompt: detail.systemPrompt,
    provider_id: detail.providerId,
    model: detail.model,
    enabled: true,
    installed: true,
    archived: false,
    built_in: false,
    created_at: now,
    updated_at: now,
  }
}

async function detail(command: string, args: Record<string, unknown>) {
  const decoded = decodeProfileDetail(await invoke<unknown>(command, args))
  if (!decoded) throw new Error('Bot 配置无法读取')
  return decoded
}

async function memory(command: string, args: Record<string, unknown>) {
  const decoded = decodeMemory(await invoke<unknown>(command, args))
  if (!decoded) throw new Error('Bot 记忆无法读取')
  return decoded
}

function capabilityWrite(source: 'granted_defaults' | 'explicit', mcpServerIds: string[], skillIds: string[]): BotCapabilityWrite {
  return source === 'granted_defaults' ? { source } : { source, mcpServerIds, skillIds }
}

function modelWrite(source: ModelSource, providerId: string, modelId: string): BotModelWrite {
  return source === 'explicit' ? { source, providerId, model: modelId } : { source }
}

function approvalWrite(source: ApprovalSource, policy: string): BotApprovalWrite {
  return source === 'override' ? { source, policy } : { source }
}

export const profileApi = {
  create: (persona: BotCreateRequest) => detail('chat_create_bot', { request: persona }),
  updatePersona: (assistantId: string, persona: BotPersonaWrite, expectedRevision: number) => detail('chat_update_bot', {
    request: profileEditRequest(assistantId, expectedRevision, { ...persona }),
  }),
  updateCapability: (assistantId: string, expectedRevision: number, source: 'granted_defaults' | 'explicit', mcpServerIds: string[] = [], skillIds: string[] = []) =>
    detail('chat_update_bot', {
      request: profileEditRequest(assistantId, expectedRevision, {
        capability: capabilityWrite(source, mcpServerIds, skillIds),
      }),
    }),
  updateModel: (assistantId: string, expectedRevision: number, source: ModelSource, providerId = '', modelId = '') =>
    detail('chat_update_bot', {
      request: profileEditRequest(assistantId, expectedRevision, {
        model: modelWrite(source, providerId, modelId),
      }),
    }),
  updateApproval: (assistantId: string, expectedRevision: number, source: ApprovalSource, policy = '') =>
    detail('chat_update_bot', {
      request: profileEditRequest(assistantId, expectedRevision, {
        approval: approvalWrite(source, policy),
      }),
    }),
  get: (assistantId: string) => detail('chat_get_bot_profile', { assistantId }),
  list: async () => {
    const decoded = decodeBotList(await invoke<unknown>('chat_list_bots'))
    if (!decoded) throw new Error('Bot 列表无法读取')
    return decoded
  },
  setPrimary: (assistantId: string, primary: boolean) =>
    invoke<unknown>('chat_set_primary_bot', { assistantId, primary }),
  memory: (assistantId: string) => memory('chat_get_bot_memory', { assistantId }),
  saveMemory: (assistantId: string, layer: MemoryLayer, content: string) =>
    memory('chat_save_bot_memory', { assistantId, layer, content }),
  clearMemory: (assistantId: string, layer: MemoryLayer) =>
    memory('chat_clear_bot_memory', { assistantId, layer }),
  addShared: async (assistantId: string, content: string) => {
    await invoke<unknown>('chat_add_bot_shared_memory', { assistantId, content })
  },
  deleteShared: async (assistantId: string, noteId: string) => {
    await invoke<unknown>('chat_delete_bot_shared_memory', { assistantId, noteId })
  },
  onChanged: (listener: () => void) => {
    if (!isTauriRuntime()) return Promise.resolve(() => {})
    return listen<unknown>('bot-profile-changed', () => listener())
  },
}
