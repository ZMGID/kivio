// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Conversation, StudyConversationImport, StudyMessageSource } from './types'

const mocks = vi.hoisted(() => ({ tauri: false, invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke, convertFileSrc: (path: string) => path }))
vi.mock('./utils', async original => ({ ...await original<typeof import('./utils')>(), isTauriRuntime: () => mocks.tauri }))
import { chatApi } from './api'

const materialId = 'a'.repeat(64)
const context = { materialId, page: 2 }
const source: StudyMessageSource = { page: 2, region: null, mode: 'read', attempt: '' }
const imported: StudyConversationImport = { version: 1, fingerprint: 'b'.repeat(64), messages: [
  { id: 'legacy-user', role: 'user', content: 'Read this graph', timestamp: 123, studySource: source },
  { id: 'legacy-assistant', role: 'assistant', content: '', timestamp: 124, streamOutcome: 'error', providerId: 'old-provider', model: 'old-model', error: 'Stopped before an answer' },
] }
const conversation = (): Conversation => ({ id: `conv_study_${materialId}_2`, revision: 1, title: 'Study', provider_id: 'provider', model: 'vision', created_at: 1, updated_at: 1, messages: [], study_context: { ...context, legacyImport: { version: 1, fingerprint: imported.fingerprint } } })

beforeEach(() => { mocks.tauri = false; mocks.invoke.mockReset(); localStorage.clear() })
afterEach(() => { localStorage.clear() })

describe('Study uses ordinary Chat API contracts', () => {
  it('passes source and immutable import through existing conversation creation IPC', async () => {
    mocks.tauri = true
    mocks.invoke.mockResolvedValue({ success: true, conversation: conversation() })
    await chatApi.createConversation('provider', 'vision', undefined, null, null, null, context, imported)
    expect(mocks.invoke).toHaveBeenCalledWith('chat_create_conversation', {
      providerId: 'provider', model: 'vision', folder: undefined, projectId: null, setId: null, assistantId: null,
      studyContext: context, studyImport: imported,
    })
  })

  it('passes source alongside normal attachment paths and user message identity', async () => {
    mocks.tauri = true
    mocks.invoke.mockResolvedValue({ success: true, conversation: conversation() })
    await chatApi.sendMessage(conversation().id, 'Read this graph', [{ id: 'image', type: 'image', name: 'page-2.png', path: '/tmp/page-2.png' }], null, undefined, 'user-new', source)
    expect(mocks.invoke).toHaveBeenCalledWith('chat_send_message', {
      conversationId: conversation().id, content: 'Read this graph', attachments: ['/tmp/page-2.png'], textAttachments: [],
      activeSkillId: null, planMessageId: undefined, userMessageId: 'user-new', studySource: source,
    })
  })

  it('keeps ordinary Chat invocation arguments unchanged', async () => {
    mocks.tauri = true
    mocks.invoke.mockResolvedValue({ success: true, conversation: conversation() })
    await chatApi.createConversation('provider', 'model')
    expect(mocks.invoke).toHaveBeenLastCalledWith('chat_create_conversation', { providerId: 'provider', model: 'model', folder: undefined, projectId: undefined, setId: undefined, assistantId: undefined })
    await chatApi.sendMessage('ordinary', 'hello')
    expect(mocks.invoke).toHaveBeenLastCalledWith('chat_send_message', { conversationId: 'ordinary', content: 'hello', attachments: [], textAttachments: [], activeSkillId: undefined, planMessageId: undefined, userMessageId: undefined })
  })

  it('browser preview preserves imported metadata and refuses a legacy retry without the original image', async () => {
    const saved = await chatApi.createConversation('provider', 'vision', undefined, null, null, null, context, imported)
    expect(saved.id).toBe(conversation().id)
    expect(saved.messages[0]).toMatchObject({ id: 'legacy-user', study_source: source, timestamp: 123 })
    expect(saved.messages[1]).toMatchObject({ content: '', stream_outcome: 'error', study_legacy_error: 'Stopped before an answer', provider_id: 'old-provider', model: 'old-model', timestamp: 124 })
    await expect(chatApi.regenerateMessage(saved.id, 'legacy-assistant')).rejects.toThrow('no saved page image')
    expect((await chatApi.getConversation(saved.id)).messages).toEqual(saved.messages)
    const again = await chatApi.createConversation('other', 'new-model', undefined, null, null, null, context, imported)
    expect(again).toEqual(saved)
    await expect(chatApi.createConversation('provider', 'vision', undefined, null, null, null, context, { ...imported, fingerprint: 'c'.repeat(64) })).rejects.toThrow('different saved source')
  })

  it('browser preview persists actual new source attachments through shared send and regeneration', async () => {
    const saved = await chatApi.createConversation('provider', 'vision', undefined, null, null, null, context, imported)
    const sent = await chatApi.sendMessage(saved.id, 'New graph question', [{ id: 'page', type: 'image', name: 'page-2.png', path: '/tmp/page-2.png' }], null, undefined, 'new-question', source)
    const lastUser = sent.messages[sent.messages.length - 2]
    expect(lastUser.study_source).toEqual(source)
    expect(lastUser.attachments?.[0].path).toBe('/tmp/page-2.png')
    const retried = await chatApi.regenerateMessage(saved.id, sent.messages[sent.messages.length - 1].id)
    expect(retried.messages[retried.messages.length - 2]).toEqual(lastUser)
  })
})
