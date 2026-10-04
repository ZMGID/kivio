import { createWindowStore } from '../utils/windowStore'
import { chatApi } from './api'
import type { ChatAssistant } from './types'

export type AssistantSaveStatus = 'idle' | 'saving' | 'saved' | 'error'

/**
 * Explicit assistant edit session. There is no autosave timer: only save()
 * writes, and it keeps typing that arrives before the flight finishes.
 */
export interface AssistantDraftState {
  draft: ChatAssistant | null
  revision: number
  acknowledgedRevision: number
  persisted: boolean
  status: AssistantSaveStatus
  error: string
  editing: boolean
  session: number
}

const EMPTY: AssistantDraftState = {
  draft: null,
  revision: 0,
  acknowledgedRevision: 0,
  persisted: false,
  status: 'idle',
  error: '',
  editing: false,
  session: 0,
}

export const assistantDraftStore = createWindowStore<AssistantDraftState>(EMPTY)

const blockedIds = new Set<string>()
const landedIds = new Set<string>()

export function assistantSaveLanded(id: string) {
  return landedIds.has(id)
}

function live(id: string, session: number) {
  const state = assistantDraftStore.getSnapshot()
  return state.editing && state.session === session && state.draft?.id === id && !blockedIds.has(id)
}

export function resetAssistantDraftStore() {
  blockedIds.clear()
  landedIds.clear()
  assistantDraftStore.setState((prev) => ({ ...EMPTY, session: prev.session + 1 }))
}

export function normalizeAssistantForDraft(assistant: ChatAssistant): ChatAssistant {
  return {
    ...assistant,
    description: assistant.description ?? '',
    icon: assistant.icon ?? 'bot',
    color: assistant.color ?? '#6A8FBD',
    source: assistant.source ?? (assistant.built_in ?? assistant.builtIn ? 'builtin' : 'user'),
    system_prompt: assistant.system_prompt ?? assistant.systemPrompt ?? '',
    provider_id: assistant.provider_id ?? assistant.providerId ?? '',
    model: assistant.model ?? '',
    mcp_server_ids: assistant.mcp_server_ids ?? assistant.mcpServerIds ?? [],
    skill_ids: assistant.skill_ids ?? assistant.skillIds ?? [],
    enabled: assistant.enabled ?? true,
    installed: assistant.installed ?? true,
    archived: assistant.archived ?? false,
    built_in: assistant.built_in ?? assistant.builtIn ?? false,
    created_at: assistant.created_at ?? assistant.createdAt ?? Math.floor(Date.now() / 1000),
    updated_at: assistant.updated_at ?? assistant.updatedAt ?? Math.floor(Date.now() / 1000),
  }
}

function normalizeStringList(values?: string[], limit = 64): string[] {
  const out: string[] = []
  for (const value of values ?? []) {
    const item = value.trim()
    if (!item || out.includes(item)) continue
    out.push(item)
    if (out.length >= limit) break
  }
  return out
}

function draftPayload(draft: ChatAssistant): ChatAssistant {
  return {
    ...draft,
    name: draft.name.trim(),
    description: draft.description?.trim() ?? '',
    icon: draft.icon?.trim() || 'bot',
    color: draft.color?.trim() || '#6A8FBD',
    source: draft.source || (draft.built_in ?? draft.builtIn ? 'builtin' : 'user'),
    system_prompt: (draft.system_prompt ?? draft.systemPrompt ?? '').trim(),
    provider_id: (draft.provider_id ?? draft.providerId ?? '').trim(),
    model: draft.provider_id ? (draft.model ?? '').trim() : '',
    mcp_server_ids: normalizeStringList(draft.mcp_server_ids ?? draft.mcpServerIds),
    skill_ids: normalizeStringList(draft.skill_ids ?? draft.skillIds),
    enabled: draft.enabled ?? true,
    installed: draft.installed ?? true,
    archived: false,
    built_in: draft.built_in ?? draft.builtIn ?? false,
    created_at: draft.created_at,
    updated_at: Math.floor(Date.now() / 1000),
  }
}

export function beginAssistantEdit(assistant: ChatAssistant, persisted: boolean) {
  const current = assistantDraftStore.getSnapshot()
  if (current.editing && current.draft?.id === assistant.id) return
  const draft = normalizeAssistantForDraft(assistant)
  assistantDraftStore.setState((prev) => ({
    draft,
    revision: 1,
    acknowledgedRevision: 1,
    persisted,
    status: 'idle',
    error: '',
    editing: true,
    session: prev.session + 1,
  }))
}

/** Back / successful leave. Does not write. */
export function abandonAssistantEdit() {
  assistantDraftStore.setState((prev) => ({ ...EMPTY, session: prev.session + 1 }))
}

export function updateAssistantDraft(recipe: (draft: ChatAssistant) => ChatAssistant) {
  assistantDraftStore.setState((prev) => {
    if (!prev.editing || !prev.draft || blockedIds.has(prev.draft.id)) return prev
    return {
      ...prev,
      draft: recipe(prev.draft),
      revision: prev.revision + 1,
    }
  })
}

/** Stop a flight from landing on this id. Release if the delete does not succeed. */
export function blockAssistantSave(id: string) {
  blockedIds.add(id)
  return () => {
    blockedIds.delete(id)
    assistantDraftStore.setState((prev) => (
      prev.draft?.id === id && prev.status === 'saving' ? { ...prev, status: 'idle' } : prev
    ))
  }
}

export function forgetAssistantLanding(id: string) {
  landedIds.delete(id)
}

export function joinAssistantSave(id: string) {
  return assistantDraftStore.run(`assistant:${id}`, async () => {})
}

async function writeAssistantSession(id: string, session: number, nameRequired: string, force: boolean) {
  if (!live(id, session)) return
  let persisted = assistantDraftStore.getSnapshot().persisted
  let mustWrite = force
  while (live(id, session)) {
    const snap = assistantDraftStore.getSnapshot()
    if (!snap.draft) return
    if (!mustWrite && persisted && snap.revision === snap.acknowledgedRevision) {
      assistantDraftStore.setState((prev) => {
        if (!live(id, session) || prev.revision !== prev.acknowledgedRevision) return prev
        if (prev.status === 'saved' && prev.error === '') return prev
        return { ...prev, status: 'saved', error: '', persisted: true }
      })
      if (!live(id, session)) return
      const after = assistantDraftStore.getSnapshot()
      if (after.revision !== after.acknowledgedRevision) continue
      return
    }
    mustWrite = false
    const revision = snap.revision
    const body = draftPayload(snap.draft)
    if (!body.name) {
      assistantDraftStore.setState((prev) => (
        live(id, session) ? { ...prev, status: 'error', error: nameRequired } : prev
      ))
      return
    }
    assistantDraftStore.setState((prev) => (
      live(id, session) ? { ...prev, status: 'saving', error: '' } : prev
    ))
    if (!live(id, session)) return
    try {
      const saved = persisted
        ? await chatApi.updateAssistant(body)
        : await chatApi.createAssistant(body)
      persisted = true
      landedIds.add(saved.id || id)
      if (!live(id, session)) return
      const normalized = normalizeAssistantForDraft(saved)
      assistantDraftStore.setState((prev) => {
        if (!live(id, session) || !prev.draft) return prev
        if (prev.revision !== revision) {
          return {
            ...prev,
            persisted: true,
            acknowledgedRevision: revision,
            status: 'saving',
          }
        }
        return {
          ...prev,
          draft: normalized,
          revision,
          acknowledgedRevision: revision,
          persisted: true,
          status: 'saved',
          error: '',
        }
      })
    } catch (err) {
      if (!live(id, session)) return
      const message = err instanceof Error ? err.message : String(err)
      assistantDraftStore.setState((prev) => (
        live(id, session) ? { ...prev, status: 'error', error: message } : prev
      ))
      return
    }
  }
}

async function drainAssistant(id: string, session: number, nameRequired: string): Promise<ChatAssistant | null> {
  for (;;) {
    await assistantDraftStore.run(`assistant:${id}`, () => writeAssistantSession(id, session, nameRequired, true))
    const after = assistantDraftStore.getSnapshot()
    if (!live(id, session)) return null
    if (after.status === 'error') return null
    if (after.persisted && after.revision === after.acknowledgedRevision && after.draft) return after.draft
  }
}

export function saveAssistantDraft(nameRequired: string): Promise<ChatAssistant | null> {
  const start = assistantDraftStore.getSnapshot()
  if (!start.editing || !start.draft) return Promise.resolve(null)
  if (blockedIds.has(start.draft.id)) return Promise.resolve(null)
  return drainAssistant(start.draft.id, start.session, nameRequired)
}
