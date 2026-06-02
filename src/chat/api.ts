// Chat API 调用封装
import { invoke } from '@tauri-apps/api/core'
import type { Conversation, ConversationListItem } from './types'

export const chatApi = {
  // 获取对话列表
  async getConversations(
    offset = 0,
    limit = 50,
    folder?: string
  ): Promise<ConversationListItem[]> {
    const result = await invoke<{ success: boolean; conversations: ConversationListItem[] }>(
      'chat_get_conversations',
      { offset, limit, folder }
    )
    if (!result.success) {
      throw new Error('Failed to get conversations')
    }
    return result.conversations
  },

  // 获取对话详情
  async getConversation(conversationId: string): Promise<Conversation> {
    const result = await invoke<{ success: boolean; conversation: Conversation }>(
      'chat_get_conversation',
      { conversationId }
    )
    if (!result.success) {
      throw new Error('Failed to get conversation')
    }
    return result.conversation
  },

  // 创建新对话
  async createConversation(
    providerId?: string,
    model?: string,
    folder?: string
  ): Promise<Conversation> {
    const result = await invoke<{ success: boolean; conversation: Conversation }>(
      'chat_create_conversation',
      { providerId, model, folder }
    )
    if (!result.success) {
      throw new Error('Failed to create conversation')
    }
    return result.conversation
  },

  // 发送消息
  async sendMessage(
    conversationId: string,
    content: string,
    attachments: string[] = []
  ): Promise<Conversation> {
    const result = await invoke<{ success: boolean; conversation?: Conversation; error?: string }>(
      'chat_send_message',
      { conversationId, content, attachments }
    )
    if (!result.success || !result.conversation) {
      throw new Error(result.error || 'Failed to send message')
    }
    return result.conversation
  },

  // 删除对话
  async deleteConversation(conversationId: string): Promise<void> {
    const result = await invoke<{ success: boolean }>('chat_delete_conversation', {
      conversationId,
    })
    if (!result.success) {
      throw new Error('Failed to delete conversation')
    }
  },

  // 更新对话
  async updateConversation(
    conversationId: string,
    updates: {
      title?: string
      pinned?: boolean
      folder?: string
      providerId?: string
      model?: string
    }
  ): Promise<Conversation> {
    const result = await invoke<{ success: boolean; conversation: Conversation }>(
      'chat_update_conversation',
      {
        conversationId,
        title: updates.title,
        pinned: updates.pinned,
        folder: updates.folder,
        providerId: updates.providerId,
        model: updates.model,
      }
    )
    if (!result.success) {
      throw new Error('Failed to update conversation')
    }
    return result.conversation
  },
}
