import { IDBFactory, IDBObjectStore } from 'fake-indexeddb'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Conversation, StudyConversationContext, StudyConversationImport, StudyMessageSource } from '../types'
import type { StudyTurn } from './studyStorage'
import type { StudyReaderContext } from './studyMaterial'

const api = vi.hoisted(() => ({ createConversation: vi.fn(), getConversation: vi.fn() }))
const attachments = vi.hoisted(() => ({ chatSavePastedImage: vi.fn() }))
vi.mock('../../api/tauri', () => ({ api: attachments }))
vi.mock('../api', () => ({ chatApi: api }))
let bridge: typeof import('./studyChatBridge')
let storage: typeof import('./studyStorage')
let composer: typeof import('../composerDraft')
let conversations: Map<string, Conversation>

beforeEach(async () => {
  vi.resetModules()
  vi.stubGlobal('indexedDB', new IDBFactory())
  storage = await import('./studyStorage')
  composer = await import('../composerDraft')
  bridge = await import('./studyChatBridge')
  conversations = new Map()
  attachments.chatSavePastedImage.mockReset().mockResolvedValue({ success: true, path: '/tmp/study-page.png', name: 'study-page.png' })
  api.createConversation.mockReset().mockImplementation(async (
    provider: string | undefined, model: string | undefined,
    _folder: unknown, _project: unknown, _assistant: unknown, _set: unknown,
    context: StudyConversationContext, imported: StudyConversationImport,
  ) => {
    const id = `conv_study_${context.materialId}_${context.page}`
    const existing = conversations.get(id)
    if (existing) {
      if (existing.study_context?.legacyImport?.fingerprint !== imported.fingerprint) throw new Error('Source mismatch')
      return existing
    }
    const conversation: Conversation = {
      id, revision: 1, title: 'Study', provider_id: provider ?? 'provider', model: model ?? 'vision',
      created_at: 1, updated_at: 1,
      study_context: { ...context, legacyImport: { version: 1, fingerprint: imported.fingerprint } },
      messages: imported.messages.map(message => ({
        id: message.id, role: message.role, content: message.content, timestamp: message.timestamp,
        study_source: message.studySource, stream_outcome: message.streamOutcome,
        provider_id: message.providerId, model: message.model, study_legacy_error: message.error,
      })),
    }
    conversations.set(id, conversation)
    return conversation
  })
  api.getConversation.mockReset().mockImplementation(async (id: string) => {
    const result = conversations.get(id)
    if (!result) throw new Error('Conversation not found')
    return result
  })
})

afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals() })

function turn(overrides: Partial<StudyTurn> = {}): StudyTurn {
  return { id: 'legacy-turn', page: 1, mode: 'hint', question: 'Why?', attempt: 'My attempt',
    sourceText: 'EXTRACTED_PRIVATE_SOURCE', answer: 'A hint', status: 'complete', createdAt: 123456,
    providerId: 'old-provider', model: 'old-vision-model', region: null, ...overrides }
}
async function seed(history: StudyTurn[] = [turn()]) {
  const { document } = await storage.importStudyDocument({ name: 'paper.pdf', kind: 'pdf', pageCount: 2, blob: new Blob(['original PDF']) })
  document.lastPage = 1
  document.pages['1'] = { ...storage.createEmptyStudyPage(), question: 'Unsent question', attempt: 'Unsent attempt', notes: 'Keep notes', correctedText: 'MANUAL_LEGACY_SOURCE', history }
  document.revision = await storage.saveStudyDocument(document)
  return document
}

async function rawRecord(store: string, key: string): Promise<unknown> {
  const opening = indexedDB.open(storage.STUDY_DATABASE_NAME, 1)
  const db = await new Promise<IDBDatabase>((resolve, reject) => { opening.onsuccess = () => resolve(opening.result); opening.onerror = () => reject(opening.error) })
  try {
    return await new Promise((resolve, reject) => {
      const request = db.transaction(store).objectStore(store).get(key)
      request.onsuccess = () => resolve(request.result)
      request.onerror = () => reject(request.error)
    })
  } finally { db.close() }
}

describe('Study history migration into Chat', () => {
  it('imports all visible turns, timestamps and outcomes while preserving the exact legacy documents and material bytes', async () => {
    const legacyAnswer = JSON.stringify({ version: 1, mode: 'hint', hint: 'Visible hint', withheldSolution: 'SECRET_SOLUTION' })
    const document = await seed([
      turn({ answer: legacyAnswer }),
      turn({ id: 'empty-error', status: 'error', answer: '', error: 'Connection failed' }),
      turn({ id: 'cancelled', status: 'cancelled', answer: 'Partial answer' }),
      turn({ id: 'streaming', status: 'streaming', answer: 'Interrupted answer' }),
    ])
    const before = await rawRecord('documents', document.id)
    const workspaceBefore = await rawRecord('meta', 'workspace')
    const conversation = await bridge.ensureStudyConversation({ documentId: document.id, page: 1 })
    expect(conversation.messages).toHaveLength(8)
    expect(conversation.messages[0]).toMatchObject({ content: 'Why?', timestamp: 123, study_source: { page: 1, mode: 'hint', attempt: 'My attempt', region: null } })
    expect(conversation.messages[1]).toMatchObject({ content: 'Visible hint', stream_outcome: 'completed', provider_id: 'old-provider', model: 'old-vision-model' })
    expect(conversation.messages[3]).toMatchObject({ content: '', stream_outcome: 'error', study_legacy_error: 'Connection failed' })
    expect(conversation.messages[5]).toMatchObject({ content: 'Partial answer', stream_outcome: 'cancelled' })
    expect(conversation.messages[7]).toMatchObject({ content: 'Interrupted answer', stream_outcome: 'interrupted' })
    expect(JSON.stringify(api.createConversation.mock.calls)).not.toMatch(/SECRET_SOLUTION|EXTRACTED_PRIVATE_SOURCE|MANUAL_LEGACY_SOURCE|Unsent question/)
    expect(await rawRecord('documents', document.id)).toEqual(before)
    expect(await rawRecord('meta', 'workspace')).toEqual(workspaceBefore)
    expect(await (await storage.readStudyDocumentBlob(document.id)).text()).toBe('original PDF')
    expect((await storage.readStudyChatSource(document.id, 1)).receipt?.conversationId).toBe(conversation.id)
  })

  it('does not create duplicates under concurrent page opening or overwrite Chat continued after migration', async () => {
    const document = await seed()
    const options = { documentId: document.id, page: 1 }
    const [first, second] = await Promise.all([bridge.ensureStudyConversation(options), bridge.ensureStudyConversation(options)])
    expect(first.id).toBe(second.id)
    expect(conversations.size).toBe(1)
    first.messages.push({ id: 'continued', role: 'user', content: 'New question', timestamp: 200 })
    const count = api.createConversation.mock.calls.length
    expect((await bridge.ensureStudyConversation(options)).messages.slice(-1)[0]?.id).toBe('continued')
    expect(api.createConversation).toHaveBeenCalledTimes(count)
  })

  it('recovers a durable create whose response was lost, without another conversation or lost source records', async () => {
    const document = await seed()
    const create = api.createConversation.getMockImplementation()!
    api.createConversation.mockImplementationOnce(async (...args) => { await create(...args); throw new Error('Response lost') })
    const options = { documentId: document.id, page: 1 }
    await expect(bridge.ensureStudyConversation(options)).rejects.toThrow('Response lost')
    expect((await storage.readStudyChatSource(document.id, 1)).receipt).toBeNull()
    const restored = await bridge.ensureStudyConversation(options)
    expect(conversations.size).toBe(1)
    expect(restored.messages).toHaveLength(2)
  })

  it('retries only the receipt after its write fails, preserving the already-created Chat', async () => {
    const document = await seed()
    const put = IDBObjectStore.prototype.put
    const fail = vi.spyOn(IDBObjectStore.prototype, 'put').mockImplementation(function (this: IDBObjectStore, value, key) {
      if (this.name === 'meta' && String(value.key).startsWith('chat:')) throw new DOMException('Full', 'QuotaExceededError')
      return put.call(this, value, key)
    })
    const options = { documentId: document.id, page: 1 }
    await expect(bridge.ensureStudyConversation(options)).rejects.toMatchObject({ code: 'quota' })
    expect(conversations.size).toBe(1)
    fail.mockRestore()
    await bridge.ensureStudyConversation(options)
    expect(conversations.size).toBe(1)
    expect((await storage.readStudyChatSource(document.id, 1)).receipt).not.toBeNull()
  })

  it('does not resurrect a linked conversation after deletion', async () => {
    const document = await seed()
    const options = { documentId: document.id, page: 1 }
    const conversation = await bridge.ensureStudyConversation(options)
    conversations.delete(conversation.id)
    api.createConversation.mockClear()
    await expect(bridge.ensureStudyConversation(options)).rejects.toThrow('will not be restored automatically')
    expect(api.createConversation).not.toHaveBeenCalled()
    expect((await storage.readStudyChatSource(document.id, 1)).document.pages['1'].history).toHaveLength(1)
  })

  it('detects a legacy writer after import rather than replacing resumed Chat history', async () => {
    const document = await seed()
    const options = { documentId: document.id, page: 1 }
    const conversation = await bridge.ensureStudyConversation(options)
    document.pages['1'].history.push(turn({ id: 'older-window-late-turn' }))
    await storage.saveStudyDocument(document)
    await expect(bridge.ensureStudyConversation(options)).rejects.toMatchObject({ code: 'conflict' })
    expect(conversation.messages).toHaveLength(2)
    expect((await storage.readStudyChatSource(document.id, 1)).document.pages['1'].history).toHaveLength(2)
  })

  it('detects legacy history changed between creation and the receipt commit', async () => {
    const document = await seed()
    const create = api.createConversation.getMockImplementation()!
    api.createConversation.mockImplementationOnce(async (...args) => {
      const conversation = await create(...args)
      document.pages['1'].history.push(turn({ id: 'changed-during-migration' }))
      await storage.saveStudyDocument(document)
      return conversation
    })
    await expect(bridge.ensureStudyConversation({ documentId: document.id, page: 1 })).rejects.toMatchObject({ code: 'conflict' })
    expect((await storage.readStudyChatSource(document.id, 1)).receipt).toBeNull()
    expect(conversations.size).toBe(1)
  })

  async function reimport(documentId: string) {
    await storage.deleteStudyDocument(documentId)
    return (await storage.importStudyDocument({ name: 'paper-reimported.pdf', kind: 'pdf', pageCount: 2, blob: new Blob(['original PDF']) })).document
  }

  it.each([true, false])('reconnects an explicitly removed/reimported material without replacing Chat (legacy history: %s)', async withLegacy => {
    const document = await seed(withLegacy ? [turn()] : [])
    const options = { documentId: document.id, page: 1 }
    const conversation = await bridge.ensureStudyConversation(options)
    conversation.messages.push({ id: 'after-migration', role: 'user', content: 'Continued in Chat', timestamp: 200 })
    const originalMessages = JSON.stringify(conversation.messages)
    const originalFingerprint = conversation.study_context!.legacyImport!.fingerprint
    const restored = await reimport(document.id)
    expect(restored.id).toBe(document.id)
    expect(restored.pages).toEqual({})
    api.createConversation.mockClear()
    const reopened = await bridge.ensureStudyConversation(options)
    expect(reopened).toBe(conversation)
    expect(JSON.stringify(reopened.messages)).toBe(originalMessages)
    expect(api.createConversation).not.toHaveBeenCalled()
    const firstReceipt = (await storage.readStudyChatSource(document.id, 1)).receipt!
    expect(firstReceipt.fingerprint).toBe(originalFingerprint)
    expect(firstReceipt.sourceFingerprint).toBe((await bridge.studyHistoryImport([])).fingerprint)
    expect(firstReceipt.sourceGeneration).toBeTruthy()
    expect((await bridge.ensureStudyConversation(options)).id).toBe(conversation.id)
    // Each further explicit removal creates a new independently verifiable recovery generation.
    await reimport(document.id)
    await bridge.ensureStudyConversation(options)
    const secondReceipt = (await storage.readStudyChatSource(document.id, 1)).receipt!
    expect(secondReceipt.fingerprint).toBe(originalFingerprint)
    expect(secondReceipt.sourceGeneration).not.toBe(firstReceipt.sourceGeneration)
    expect(conversations.size).toBe(1)
  })

  it('does not treat unexpectedly emptied history as reimported material without a removal marker', async () => {
    const document = await seed()
    const options = { documentId: document.id, page: 1 }
    await bridge.ensureStudyConversation(options)
    document.pages['1'].history = []
    await storage.saveStudyDocument(document)
    await expect(bridge.ensureStudyConversation(options)).rejects.toMatchObject({ code: 'conflict' })
    expect((await storage.readStudyChatSource(document.id, 1)).restoration).toBeNull()
  })

  it.each(['before reconnect', 'after reconnect'])('rejects changed reimported history %s without overwriting Chat', async when => {
    const document = await seed()
    const options = { documentId: document.id, page: 1 }
    const conversation = await bridge.ensureStudyConversation(options)
    const restored = await reimport(document.id)
    if (when === 'after reconnect') await bridge.ensureStudyConversation(options)
    restored.pages['1'] = { ...storage.createEmptyStudyPage(), history: [turn()] }
    await storage.saveStudyDocument(restored)
    await expect(bridge.ensureStudyConversation(options)).rejects.toMatchObject({ code: 'conflict' })
    expect(conversation.messages).toHaveLength(2)
    expect(conversations.size).toBe(1)
  })

  it('rejects a history write during reconnect instead of acknowledging a stale source baseline', async () => {
    const document = await seed()
    const options = { documentId: document.id, page: 1 }
    await bridge.ensureStudyConversation(options)
    const originalReceipt = (await storage.readStudyChatSource(document.id, 1)).receipt
    const restored = await reimport(document.id)
    const get = api.getConversation.getMockImplementation()!
    api.getConversation.mockImplementationOnce(async (...args) => {
      const conversation = await get(...args)
      restored.pages['1'] = { ...storage.createEmptyStudyPage(), history: [turn({ id: 'late-reimport-writer' })] }
      await storage.saveStudyDocument(restored)
      return conversation
    })
    await expect(bridge.ensureStudyConversation(options)).rejects.toMatchObject({ code: 'conflict' })
    expect((await storage.readStudyChatSource(document.id, 1)).receipt).toEqual(originalReceipt)
  })

  it('rejects a second remove/reimport during reconnect instead of claiming its newer generation', async () => {
    const document = await seed()
    const options = { documentId: document.id, page: 1 }
    await bridge.ensureStudyConversation(options)
    const originalReceipt = (await storage.readStudyChatSource(document.id, 1)).receipt
    await reimport(document.id)
    const get = api.getConversation.getMockImplementation()!
    api.getConversation.mockImplementationOnce(async (...args) => {
      const conversation = await get(...args)
      await reimport(document.id)
      return conversation
    })
    await expect(bridge.ensureStudyConversation(options)).rejects.toMatchObject({ code: 'conflict' })
    expect((await storage.readStudyChatSource(document.id, 1)).receipt).toEqual(originalReceipt)
    expect((await bridge.ensureStudyConversation(options)).id).toBe(originalReceipt!.conversationId)
  })

  it('does not resurrect a deleted Chat when its source material is removed and reimported', async () => {
    const document = await seed()
    const options = { documentId: document.id, page: 1 }
    const conversation = await bridge.ensureStudyConversation(options)
    const originalReceipt = (await storage.readStudyChatSource(document.id, 1)).receipt
    conversations.delete(conversation.id)
    await reimport(document.id)
    api.createConversation.mockClear()
    await expect(bridge.ensureStudyConversation(options)).rejects.toThrow('will not be restored automatically')
    expect(api.createConversation).not.toHaveBeenCalled()
    expect((await storage.readStudyChatSource(document.id, 1)).receipt).toEqual(originalReceipt)
    expect(conversations.size).toBe(0)
  })

  it('rolls back the material removal when its explicit recovery marker cannot commit', async () => {
    const document = await seed()
    await bridge.ensureStudyConversation({ documentId: document.id, page: 1 })
    const put = IDBObjectStore.prototype.put
    const failure = vi.spyOn(IDBObjectStore.prototype, 'put').mockImplementation(function (this: IDBObjectStore, value, key) {
      if (this.name === 'meta' && String(value.key).startsWith('chat-material:')) throw new DOMException('Full', 'QuotaExceededError')
      return put.call(this, value, key)
    })
    await expect(storage.deleteStudyDocument(document.id)).rejects.toMatchObject({ code: 'quota' })
    failure.mockRestore()
    expect(await rawRecord('meta', `chat-material:${document.id}`)).toBeUndefined()
    expect((await storage.readStudyChatSource(document.id, 1)).document.pages['1'].history).toHaveLength(1)
    expect(await (await storage.readStudyDocumentBlob(document.id)).text()).toBe('original PDF')
  })

  it('creates a page with no history and isolates distinct pages of the same material', async () => {
    const document = await seed([])
    const first = await bridge.ensureStudyConversation({ documentId: document.id, page: 1 })
    const second = await bridge.ensureStudyConversation({ documentId: document.id, page: 2 })
    expect(first.id).not.toBe(second.id)
    expect(first.messages).toEqual([])
    expect(second.messages).toEqual([])
  })
})

describe('Study adaptation of the shared composer draft', () => {
  it('persists scoped typing, accepted clear and rejected-send restore without changing another page draft', () => {
    let savedA = 'Saved A'
    let savedB = 'Saved B'
    const offA = bridge.bindStudyComposerDraft({ conversationId: 'a', read: () => savedA, write: input => { savedA = input } })
    const offB = bridge.bindStudyComposerDraft({ conversationId: 'b', read: () => savedB, write: input => { savedB = input } })
    expect(composer.getComposerDraft('a')?.input).toBe('Saved A')
    composer.setComposerDraft('a', { input: 'New A', quotes: [], attachments: [] })
    expect(savedA).toBe('New A')
    composer.setComposerDraft('a', { input: '', quotes: [], attachments: [] })
    expect(savedA).toBe('')
    composer.setComposerDraft('a', { input: 'Restored A', quotes: [], attachments: [] })
    expect(savedA).toBe('Restored A')
    expect(savedB).toBe('Saved B')
    offA(); offB()
  })

  it('publishes confirmed saved-version recovery to the mounted composer and removes stale quotes and attachments', () => {
    let saved = 'Initially saved'
    const write = vi.fn((input: string) => { saved = input })
    const off = bridge.bindStudyComposerDraft({ conversationId: 'a', read: () => saved, write })
    composer.setComposerDraft('a', { input: 'Conflicting unsaved question', quotes: ['Old quote'], attachments: [{ id: 'old', type: 'image', name: 'old.png', path: '/tmp/old.png' }], attachmentError: 'Old failure' })
    composer.setComposerDraft('b', { input: 'Another page', quotes: [], attachments: [] })
    const displayed = vi.fn()
    const stopDisplay = composer.subscribeComposerDraft((key, draft) => { if (key === 'a') displayed(draft) })
    // Mirrors the source owner's successful, user-confirmed reload before the bridge is invoked.
    saved = 'Freshly loaded saved question'
    write.mockClear()
    bridge.restoreStudyComposerDraft('a', saved)
    expect(displayed).toHaveBeenLastCalledWith({ input: saved, quotes: [], attachments: [] })
    expect(composer.getComposerDraft('a')).toEqual({ input: saved, quotes: [], attachments: [] })
    expect(composer.getComposerDraft('b')?.input).toBe('Another page')
    expect(write).not.toHaveBeenCalled()
    off(); stopDisplay()
  })

  it('never hydrates over newer composer input and detaches old page subscriptions', () => {
    composer.setComposerDraft('a', { input: 'Newer local input', quotes: ['Quote'], attachments: [] })
    let saved = 'Older durable input'
    const off = bridge.bindStudyComposerDraft({ conversationId: 'a', read: () => saved, write: input => { saved = input } })
    expect(saved).toBe('Newer local input')
    expect(composer.getComposerDraft('a')?.quotes).toEqual(['Quote'])
    off()
    composer.setComposerDraft('a', { input: 'After detaching', quotes: [], attachments: [] })
    expect(saved).toBe('Newer local input')
    expect(composer.getComposerDraft('__new__')).toBeUndefined()
  })
})


describe('Study source uses Chat attachment preparation', () => {
  function options() {
    const documentId = 'a'.repeat(64)
    const region = { x: 0.1, y: 0.2, width: 0.3, height: 0.4 }
    const context: StudyReaderContext = { page: 2, pageCount: 5, region: { ...region }, status: 'ready', imageDataUrl: 'data:image/png;base64,aW1hZ2U=' }
    const source: StudyMessageSource = { page: 2, region, mode: 'check', attempt: 'Original attempt' }
    const conversation: Conversation = { id: `conv_study_${documentId}_2`, revision: 1, title: 'Study', provider_id: 'provider', model: 'vision', messages: [], created_at: 1, updated_at: 1, study_context: { materialId: documentId, page: 2 } }
    return { documentId, page: 2, context, source, conversation }
  }

  it('saves the rendered PNG with the existing attachment API and returns shared send inputs only', async () => {
    const input = options()
    const result = await bridge.prepareStudySourceAttachment(input)
    expect(attachments.chatSavePastedImage).toHaveBeenCalledWith('study-page.png', 'image/png', 'aW1hZ2U=')
    expect(result.attachment).toEqual({ id: expect.stringMatching(/^pending-att-/), type: 'image', name: 'study-page.png', path: '/tmp/study-page.png' })
    expect(result.source).toEqual(input.source)
    expect(result.source).not.toBe(input.source)
    expect(result.source.region).not.toBe(input.source.region)
    expect(api.createConversation).not.toHaveBeenCalled()
    expect(api.getConversation).not.toHaveBeenCalled()
  })

  it('owns page, crop, mode, attempt and image before I/O while the user navigates and edits', async () => {
    let finish!: (value: { success: boolean; path: string; name: string }) => void
    attachments.chatSavePastedImage.mockImplementationOnce(() => new Promise(resolve => { finish = resolve }))
    const input = options()
    const originalSource = JSON.parse(JSON.stringify(input.source))
    const saving = bridge.prepareStudySourceAttachment(input)
    input.page = 3
    input.documentId = 'b'.repeat(64)
    input.context.page = 3
    input.context.region!.x = 0.5
    input.context.imageDataUrl = 'data:image/png;base64,bmV3LWltYWdl'
    input.source.page = 3
    input.source.region!.x = 0.6
    input.source.mode = 'read'
    input.source.attempt = 'Later attempt'
    input.conversation.study_context!.page = 3
    finish({ success: true, path: '/tmp/original-page.png', name: 'study-page.png' })
    const result = await saving
    expect(result.source).toEqual(originalSource)
    expect(result.attachment.path).toBe('/tmp/original-page.png')
    expect(attachments.chatSavePastedImage).toHaveBeenCalledWith('study-page.png', 'image/png', 'aW1hZ2U=')
  })

  it.each([
    ['document', (input: ReturnType<typeof options>) => { input.documentId = 'b'.repeat(64) }],
    ['conversation page', (input: ReturnType<typeof options>) => { input.conversation.study_context!.page = 3 }],
    ['source page', (input: ReturnType<typeof options>) => { input.source.page = 3 }],
    ['reader page', (input: ReturnType<typeof options>) => { input.context.page = 3 }],
    ['crop', (input: ReturnType<typeof options>) => { input.context.region = null }],
    ['invalid crop', (input: ReturnType<typeof options>) => { input.source.region!.width = 1 }],
    ['loading', (input: ReturnType<typeof options>) => { input.context.status = 'loading' }],
    ['missing image', (input: ReturnType<typeof options>) => { input.context.imageDataUrl = undefined }],
    ['empty attempt', (input: ReturnType<typeof options>) => { input.source.attempt = '  ' }],
  ] as const)('rejects a %s mismatch before saving any attachment', async (_name, change) => {
    const input = options()
    change(input)
    await expect(bridge.prepareStudySourceAttachment(input)).rejects.toThrow()
    expect(attachments.chatSavePastedImage).not.toHaveBeenCalled()
  })

  it('leaves model eligibility to shared Chat and permits ordinary reading without an attempt', async () => {
    const input = options()
    input.source = { page: 2, mode: 'read', attempt: '', region: null }
    input.context.region = null
    input.conversation.model = ''
    input.conversation.provider_id = ''
    expect((await bridge.prepareStudySourceAttachment(input)).source).toEqual(input.source)
  })

  it.each([
    [{ success: false, error: 'Disk full' }, 'Disk full'],
    [{ success: true, name: 'study-page.png' }, 'Could not save'],
  ])('surfaces unsuccessful or incomplete saves without replacing the question or source', async (response, expected) => {
    attachments.chatSavePastedImage.mockResolvedValueOnce(response)
    const input = options()
    const before = JSON.stringify(input)
    await expect(bridge.prepareStudySourceAttachment(input)).rejects.toThrow(expected as string)
    expect(JSON.stringify(input)).toBe(before)
    expect(api.createConversation).not.toHaveBeenCalled()
  })

  it('propagates attachment transport failure without starting a model request', async () => {
    attachments.chatSavePastedImage.mockRejectedValueOnce(new Error('Connection unavailable'))
    await expect(bridge.prepareStudySourceAttachment(options())).rejects.toThrow('Connection unavailable')
    expect(api.createConversation).not.toHaveBeenCalled()
  })
})
