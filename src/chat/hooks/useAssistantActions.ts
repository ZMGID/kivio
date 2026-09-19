import { useCallback, type Dispatch, type MutableRefObject, type SetStateAction } from 'react'
import { chatApi } from '../api'
import type { createChatNavigationController } from '../chatNavigationController'
import type { AssistantStreamStats } from '../MessageList'
import { setCoarse as setStreamCoarse } from '../streamingStore'
import type { ChatAssistant, Conversation } from '../types'

type ChatNavigationController = ReturnType<typeof createChatNavigationController>

export interface AssistantActionIdentity {
  activeProviderId: string
  activeModel: string
  projectId: string | null | undefined
  projectName: string | undefined
  setId: string | null | undefined
}

export interface UseAssistantActionsOptions {
  currentConversationRef: MutableRefObject<Conversation | null>
  navigation: Pick<
    ChatNavigationController,
    'beginConversationCreation' | 'isConversationCreationCurrent' | 'commitCreatedConversation'
  >
  identity: AssistantActionIdentity
  refreshSidebar: () => void
  refreshContextStats: (conversationId: string) => Promise<void>
  applyConversationIfCurrent: (expectedId: string, conversation: Conversation) => boolean
  setStreamErrorForConversation: (conversationId: string, message: string) => void
  setAssistantStreamStatsByMessageId: Dispatch<SetStateAction<Record<string, AssistantStreamStats>>>
}

function errorMessage(err: unknown, fallback: string): string {
  return typeof err === 'string' ? err : (err as Error).message || fallback
}

function setStreamError(error: string): void {
  setStreamCoarse({ streamError: error })
}

/** 助手 / 搭建对话的创建与会话级助手切换。 */
export function useAssistantActions({
  currentConversationRef,
  navigation,
  identity,
  refreshSidebar,
  refreshContextStats,
  applyConversationIfCurrent,
  setStreamErrorForConversation,
  setAssistantStreamStatsByMessageId,
}: UseAssistantActionsOptions) {
  const startAssistantChat = useCallback(async (assistant: ChatAssistant) => {
    const creation = navigation.beginConversationCreation()
    setAssistantStreamStatsByMessageId({})
    try {
      const assistantProviderId = assistant.provider_id ?? assistant.providerId ?? ''
      const assistantModel = assistant.model ?? ''
      const conv = await chatApi.createConversation(
        assistantProviderId || identity.activeProviderId || undefined,
        assistantModel || identity.activeModel || undefined,
        identity.projectName,
        identity.projectId ?? null,
        assistant.id,
        identity.setId ?? null,
      )
      refreshSidebar()
      if (navigation.commitCreatedConversation(creation, conv)) {
        setStreamError('')
      }
    } catch (err) {
      console.error('Failed to start assistant conversation:', err)
      if (navigation.isConversationCreationCurrent(creation)) {
        setStreamError(errorMessage(err, '创建助手对话失败'))
      }
    }
  }, [identity, navigation, refreshSidebar, setAssistantStreamStatsByMessageId])

  const startBuilderChat = useCallback(async () => {
    const creation = navigation.beginConversationCreation()
    setAssistantStreamStatsByMessageId({})
    try {
      const conv = await chatApi.createBuilderConversation(
        identity.activeProviderId || undefined,
        identity.activeModel || undefined,
        identity.projectId ?? null,
      )
      refreshSidebar()
      if (navigation.commitCreatedConversation(creation, conv)) {
        setStreamError('')
      }
    } catch (err) {
      console.error('Failed to start builder conversation:', err)
      if (navigation.isConversationCreationCurrent(creation)) {
        setStreamError(errorMessage(err, '创建搭建对话失败'))
      }
    }
  }, [identity, navigation, refreshSidebar, setAssistantStreamStatsByMessageId])

  const applyAssistant = useCallback(async (assistantId: string | null) => {
    const conversation = currentConversationRef.current
    if (!conversation) return
    const conversationId = conversation.id
    try {
      const updated = await chatApi.updateConversation(conversationId, {
        assistantId: assistantId ?? '',
      })
      applyConversationIfCurrent(conversationId, updated)
      refreshSidebar()
      if (assistantId) void refreshContextStats(updated.id)
    } catch (err) {
      console.error('Failed to update conversation assistant:', err)
      setStreamErrorForConversation(conversationId, errorMessage(err, '助手切换失败'))
    }
  }, [
    applyConversationIfCurrent, currentConversationRef, refreshContextStats,
    refreshSidebar, setStreamErrorForConversation,
  ])

  // 底栏弹层选择专家：有会话则切换该会话专家，无会话则以该专家开新对话；null=清除。
  const selectAssistant = useCallback(async (assistant: ChatAssistant | null) => {
    if (!assistant) {
      await applyAssistant(null)
      return
    }
    if (currentConversationRef.current) await applyAssistant(assistant.id)
    else await startAssistantChat(assistant)
  }, [applyAssistant, currentConversationRef, startAssistantChat])

  return { startAssistantChat, startBuilderChat, applyAssistant, selectAssistant }
}
