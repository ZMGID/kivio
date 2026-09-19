import { isPlaceholderTitle, optimisticConversationTitle } from './conversationTitle'
import type { Conversation, ConversationListItem } from './types'

/**
 * 侧栏乐观条目：发送一落地就把会话顶到侧栏最上方，不等真实列表 refetch。
 * 生命期是「发送 → settle（原地换成持久化标题）→ 下一次 refetch 接管」。
 */

export function optimisticConversationListItem(
  conversation: Conversation,
  content: string,
  attachmentNames: readonly string[] = [],
): ConversationListItem {
  const preview = content.replace(/\s+/g, ' ').trim()
  const title = isPlaceholderTitle(conversation.title)
    ? optimisticConversationTitle(content, attachmentNames)
    : conversation.title
  return {
    id: conversation.id,
    title,
    preview: preview.length > 100 ? `${preview.slice(0, 100)}...` : preview,
    provider_id: conversation.provider_id,
    model: conversation.model,
    message_count: Math.max(1, conversation.messages.length),
    created_at: conversation.created_at,
    updated_at: Math.floor(Date.now() / 1000),
    pinned: conversation.pinned,
    folder: conversation.folder,
    project_id: conversation.project_id ?? conversation.projectId ?? null,
    projectId: conversation.project_id ?? conversation.projectId ?? null,
    set_id: conversation.set_id ?? conversation.setId ?? null,
    setId: conversation.set_id ?? conversation.setId ?? null,
    assistant_id: conversation.assistant_id ?? conversation.assistantId ?? null,
    assistantId: conversation.assistant_id ?? conversation.assistantId ?? null,
    assistant_name:
      conversation.assistant_snapshot?.name
      ?? conversation.assistantSnapshot?.name
      ?? null,
    assistantName:
      conversation.assistant_snapshot?.name
      ?? conversation.assistantSnapshot?.name
      ?? null,
  }
}

/** 取会话最后一条 user/assistant 消息文本（侧栏 preview 口径，与 api.ts toListItem 一致）。 */
export function conversationLastMessageContent(conversation: Conversation): string {
  for (let i = conversation.messages.length - 1; i >= 0; i--) {
    const message = conversation.messages[i]
    if (message.role === 'user' || message.role === 'assistant') {
      return message.content?.trim() ?? ''
    }
  }
  return ''
}

/**
 * 用持久化后的真实会话替换侧栏乐观条目（同 id 原地替换，行实例不销毁，SwapTitle 才能
 * 感知标题从「截断第一句」变成「模型标题」并播放替换过渡）。
 * keptConversation 为空（发送彻底失败）时退回移除条目。
 */
export function settleOptimisticConversationListItems(
  items: ConversationListItem[],
  conversationId: string,
  keptConversation: Conversation | null,
): ConversationListItem[] {
  if (!keptConversation) return items.filter((item) => item.id !== conversationId)
  const firstUser = keptConversation.messages.find((message) => message.role === 'user')
  return items.map((item) =>
    item.id === conversationId
      ? optimisticConversationListItem(
          keptConversation,
          firstUser?.content ?? conversationLastMessageContent(keptConversation),
          firstUser?.attachments?.map((attachment) => attachment.name) ?? [],
        )
      : item,
  )
}

/** 真实列表落地后剪掉不再生成中的乐观条目；长跑 run 期间的保留（乐观标题不能被打回「新对话」）。 */
export function pruneSettledOptimisticItems(
  items: ConversationListItem[],
  generatingConversationIds: ReadonlySet<string>,
): ConversationListItem[] {
  const next = items.filter((item) => generatingConversationIds.has(item.id))
  return next.length === items.length ? items : next
}
