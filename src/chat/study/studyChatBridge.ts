import { api as desktopApi } from '../../api/tauri'
import { chatApi } from '../api'
import { getComposerDraft, setComposerDraft, subscribeComposerDraft } from '../composerDraft'
import type { Conversation, PendingAttachment, StudyConversationImport, StudyMessageSource } from '../types'
import { readLegacyStudyAnswer } from './studyLegacyAnswer'
import type { StudyReaderContext } from './studyMaterial'
import { readStudyChatSource, saveStudyChatReceipt, StudyStorageError, type StudyTurn } from './studyStorage'

async function sha256(value: string): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(value))
  return Array.from(new Uint8Array(digest), byte => byte.toString(16).padStart(2, '0')).join('')
}

/** Legacy records remain untouched. Only visible conversation text enters the shared Chat history. */
export async function studyHistoryImport(history: StudyTurn[]): Promise<StudyConversationImport> {
  // Take ownership before the first await; caller edits must not change the imported snapshot.
  const snapshot = JSON.parse(JSON.stringify(history)) as StudyTurn[]
  const fingerprint = await sha256(JSON.stringify(snapshot))
  const messages: StudyConversationImport['messages'] = []
  for (const turn of snapshot) {
    const id = `msg_study_${await sha256(turn.id)}`
    const timestamp = Math.floor(turn.createdAt / 1000)
    const studySource: StudyMessageSource = { page: turn.page, region: turn.region ?? null, mode: turn.mode, attempt: turn.attempt }
    messages.push({ id: `${id}_user`, role: 'user', content: turn.question, timestamp, studySource })
    messages.push({
      id: `${id}_assistant`, role: 'assistant',
      content: readLegacyStudyAnswer(turn.answer)?.visibleText ?? turn.answer,
      timestamp,
      streamOutcome: turn.status === 'complete' ? 'completed' : turn.status === 'streaming' ? 'interrupted' : turn.status,
      providerId: turn.providerId, model: turn.model,
      ...(turn.error ? { error: turn.error } : {}),
    })
  }
  return { version: 1, fingerprint, messages }
}

/**
 * One lazy, idempotent migration per material/page. Chat owns subsequent history.
 * A receipt prevents a deleted conversation from being silently recreated from its old backup.
 */
export async function ensureStudyConversation(options: {
  documentId: string
  page: number
  providerId?: string
  model?: string
}): Promise<Conversation> {
  const { documentId: materialId, page, providerId, model } = options
  const { document, receipt, restoration } = await readStudyChatSource(materialId, page)
  const history = document.pages[String(page)]?.history ?? []
  const imported = await studyHistoryImport(history)
  // A fresh empty source is expected only when deletion and same-hash reimport left an explicit
  // durable generation marker. Its Chat import identity remains unchanged across that recovery.
  const reimported = Boolean(receipt && restoration && receipt.sourceGeneration !== restoration.generation)
  const expectedSource = receipt?.sourceFingerprint ?? receipt?.fingerprint
  if (receipt && (reimported ? history.length !== 0 : expectedSource !== imported.fingerprint)) {
    throw new StudyStorageError('conflict', 'Legacy Study history changed after migration. Its saved backup and your Chat conversation were both preserved; reload before continuing.')
  }
  let conversation: Conversation
  if (receipt) {
    try { conversation = await chatApi.getConversation(receipt.conversationId) }
    catch (error) {
      throw new StudyStorageError('storage', 'Could not reopen the saved Study conversation. Retry if it is temporarily unavailable. A removed conversation will not be restored automatically; your original material and legacy history remain saved.', error)
    }
  } else {
    conversation = await chatApi.createConversation(providerId, model, undefined, null, null, null, { materialId, page }, imported)
  }
  if (conversation.id !== `conv_study_${materialId}_${page}`
    || conversation.study_context?.materialId !== materialId
    || conversation.study_context?.page !== page
    || conversation.study_context?.legacyImport?.version !== 1
    || conversation.study_context?.legacyImport?.fingerprint !== (receipt?.fingerprint ?? imported.fingerprint)) {
    throw new StudyStorageError('conflict', 'The returned conversation does not match this saved Study page. Existing work was preserved.')
  }
  if (!receipt || reimported) await saveStudyChatReceipt({
    version: 1, materialId, page, conversationId: conversation.id, fingerprint: receipt?.fingerprint ?? imported.fingerprint,
    ...(restoration ? { sourceFingerprint: imported.fingerprint, sourceGeneration: restoration.generation } : {}),
  }, history)
  return conversation
}

/**
 * Adapt Study's restart-persistent question to the existing scoped composer.
 * `write` goes through the material owner's ordinary CAS save; this bridge owns no second draft state.
 * Register before mounting InputBar, or its subscription will receive the initial hydration.
 */
export function bindStudyComposerDraft(options: {
  conversationId: string
  read: () => string
  write: (input: string) => void
}): () => void {
  const { conversationId, read, write } = options
  const existing = getComposerDraft(conversationId)
  const initial = read()
  if (!existing) setComposerDraft(conversationId, { input: initial, quotes: [], attachments: [] })
  else if (existing.input !== initial) write(existing.input)
  return subscribeComposerDraft((key, draft) => {
    if (key === conversationId && draft.input !== read()) write(draft.input)
  })
}


/** Snapshot and save the source image using Chat's existing attachment adapter; never starts a model request. */
export async function prepareStudySourceAttachment(options: {
  documentId: string
  page: number
  context: StudyReaderContext
  source: StudyMessageSource
  conversation: Conversation
}): Promise<{ attachment: PendingAttachment; source: StudyMessageSource }> {
  const { documentId, page, context, conversation } = options
  const source: StudyMessageSource = {
    page: options.source.page, mode: options.source.mode, attempt: options.source.attempt,
    region: options.source.region ? { ...options.source.region } : null,
  }
  const region = source.region
  const imageDataUrl = context.imageDataUrl
  const binding = conversation.study_context
  // Every identity check and owned snapshot precedes attachment I/O. Later navigation or crop edits
  // must not change the captured image/source that the caller sends with its conversation override.
  if (!binding || binding.materialId !== documentId || binding.page !== page
    || source.page !== page || !Number.isSafeInteger(page) || page < 1
    || context.page !== page || page > context.pageCount) {
    throw new Error('The source page does not match this Study conversation. Reopen the page before sending.')
  }
  if (region && ([region.x, region.y, region.width, region.height].some(value => !Number.isFinite(value))
    || region.x < 0 || region.y < 0 || region.width <= 0 || region.height <= 0
    || region.x + region.width > 1.000001 || region.y + region.height > 1.000001)) {
    throw new Error('Select a valid region inside the source page.')
  }
  const currentRegion = context.region
  const sameRegion = (!region && !currentRegion) || Boolean(region && currentRegion
    && region.x === currentRegion.x && region.y === currentRegion.y
    && region.width === currentRegion.width && region.height === currentRegion.height)
  if (context.status !== 'ready' || !sameRegion) {
    throw new Error('Wait for the selected page or region to finish loading before sending.')
  }
  if (source.mode === 'check' && !source.attempt.trim()) {
    throw new Error('Write your attempt before checking it.')
  }
  const payload = /^data:image\/png;base64,([A-Za-z0-9+/]+={0,2})$/.exec(imageDataUrl ?? '')?.[1]
  if (!payload) throw new Error('The original page image is not ready. Select the page or region again.')
  const result = await desktopApi.chatSavePastedImage('study-page.png', 'image/png', payload)
  if (!result.success || !result.path || !result.name) {
    throw new Error(result.error || 'Could not save the Study page image. Your question is unchanged; retry before sending.')
  }
  return {
    attachment: { id: `pending-att-${crypto.randomUUID()}`, type: 'image', name: result.name, path: result.path },
    source,
  }
}

/** Only for an explicitly confirmed saved-version reload, after the source owner has restored its durable snapshot. */
export function restoreStudyComposerDraft(conversationId: string, input: string): void {
  setComposerDraft(conversationId, { input, quotes: [], attachments: [] })
}
