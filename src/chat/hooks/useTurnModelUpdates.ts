import type { Dispatch, SetStateAction } from 'react'
import { api } from '../../api/tauri'
import type { createChatExecutionOwner } from '../chatExecutionOwner'
import type { Conversation } from '../types'
import { useTauriEvent } from './useTauriEvent'

export function useTurnModelUpdates(
  execution: ReturnType<typeof createChatExecutionOwner>,
  setConversation: Dispatch<SetStateAction<Conversation | null>>,
) {
  useTauriEvent(api.onTurnModelUpdated, event => {
    execution.updateTurnModel(event.conversationId, event.messageId, event.turnModel)
    setConversation(current => {
      if (current?.id !== event.conversationId || current.revision > event.revision) return current
      const index = current.messages.findIndex(message => message.id === event.messageId)
      if (index < 0) return current
      const messages = current.messages.slice()
      messages[index] = { ...messages[index], turn_model: event.turnModel }
      return { ...current, messages }
    })
  }, [execution, setConversation])
}
