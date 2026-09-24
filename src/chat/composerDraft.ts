import type { PendingAttachment } from './types'

/**
 * 输入框草稿的内存暂存：切换对话/页面导致 InputBar 卸载重挂时，保住已打的字、引用与附件。
 * 按会话 id 存（新建对话用 NEW_CHAT_KEY）。仅进程内存活——app 关窗销毁即清，符合"本次使用期间"语义。
 * ponytail: 内存 Map 足够；要跨 app 重启保留再换 sessionStorage（附件 temp 路径届时可能已被 GC，需另处理）。
 */
export interface ComposerDraft {
  input: string
  quotes: string[]
  attachments: PendingAttachment[]
  attachmentError?: string
}

const NEW_CHAT_KEY = '__new__'
const drafts = new Map<string, ComposerDraft>()
const listeners = new Set<(key: string, draft: ComposerDraft) => void>()

export function subscribeComposerDraft(listener: (key: string, draft: ComposerDraft) => void) {
  listeners.add(listener)
  return () => { listeners.delete(listener) }
}

/** A pending operation follows only an explicitly committed draft migration. */
const operations = new Set<{ key: string }>()
export function registerComposerDraftScope(scope: { key: string }) {
  operations.add(scope)
  return () => { operations.delete(scope) }
}
export function beginComposerDraftOperation(key: string) {
  const scope = { key }
  return Object.assign(scope, { release: registerComposerDraftScope(scope) })
}

export function draftKey(conversationId: string | null | undefined): string {
  return conversationId || NEW_CHAT_KEY
}

export function getComposerDraft(key: string): ComposerDraft | undefined {
  return drafts.get(key)
}

export function setComposerDraft(key: string, draft: ComposerDraft): void {
  if (!draft.input && draft.quotes.length === 0 && draft.attachments.length === 0 && !draft.attachmentError) {
    drafts.delete(key)
  } else {
    drafts.set(key, draft)
  }
  for (const listener of listeners) listener(key, draft)
}

/** 异步附件结果只修改启动时的草稿，不覆盖期间输入的正文和引用。 */
export function updateComposerDraft(key: string, update: (draft: ComposerDraft) => ComposerDraft): void {
  setComposerDraft(key, update(getComposerDraft(key) ?? { input: '', quotes: [], attachments: [] }))
}

/**
 * 新建会话落库拿到真 id 时，把占位键上的草稿**搬**到真 id 下。
 *
 * 切 plan/orchestrate 模式（以及任何需要先建会话的操作）会让 conversationId 从 undefined
 * 变成真 id，草稿键随之从 NEW_CHAT_KEY 变成该 id。调用方的回填逻辑若把这当成「切到了另一条
 * 会话」，就会把用户刚打的字清掉，且切回原模式也不还原（真 id 不会变回 undefined）。
 *
 * 搬而不是拷：占位键必须腾空，否则下次新建会话又会捡到这条已归属别人的草稿。
 * 目标键已有草稿时不动（那是它自己的，优先），返回 false 让调用方走正常回填。
 */
export function migrateNewChatDraft(fromKey: string, toKey: string): boolean {
  if (fromKey !== NEW_CHAT_KEY || toKey === NEW_CHAT_KEY) return false
  if (drafts.has(toKey)) return false
  const draft = drafts.get(NEW_CHAT_KEY)
  let migrated = Boolean(draft)
  for (const scope of operations) {
    if (scope.key === fromKey) {
      scope.key = toKey
      migrated = true
    }
  }
  if (draft) setComposerDraft(toKey, draft)
  drafts.delete(NEW_CHAT_KEY)
  return migrated
}
