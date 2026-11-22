import type { AdditionalDirectory, Conversation } from './types'

/** 会话对象上 snake / camel 双写字段的读取口径，集中在这里以免页面各处各写一遍 `??`。 */

export function additionalDirectoriesOf(conversation: Conversation | null | undefined): AdditionalDirectory[] {
  return conversation?.additional_directories ?? conversation?.additionalDirectories ?? []
}

/** 没有消息也没有绑定助手的会话：顶栏模型 / 联网 / 多答模型等仍以输入栏草稿为准。 */
export function isPlainBlankConversation(conversation: Conversation | null): boolean {
  return Boolean(
    conversation
    && !hasEstablishedRuntime(conversation)
    && !(conversation.assistant_id ?? conversation.assistantId),
  )
}

/** An established conversation keeps its saved selection when an unrelated composer draft is sent. */
export function hasEstablishedRuntime(conversation: Conversation | null): boolean {
  if (!conversation) return false
  return Boolean(conversation.runtime_locked || conversation.forked_from || conversation.forkedFrom
    || (conversation.history_total ?? 0) > 0 || conversation.messages.length > 0)
}
