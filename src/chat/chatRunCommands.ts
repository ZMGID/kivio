import { chatApi } from './api'
import { createChatExecutionOwner, type ExecutionLease } from './chatExecutionOwner'
import { assistantTurnSpan } from './messageGroups'
import { createStreamPreviewOwner } from './streamPreviewOwner'
import type { Conversation } from './types'

type ExecutionOwner = ReturnType<typeof createChatExecutionOwner>
type PreviewOwner = ReturnType<typeof createStreamPreviewOwner>
type SettlementPorts = Parameters<ExecutionOwner['finish']>[2]
type Persistence = Pick<typeof chatApi, 'regenerateMessage' | 'replyWithModel'>
type Command = 'regenerate' | 'replyWithModel'

export type RunCommandPresentationEvent =
  | { kind: 'truncated'; conversation: Conversation; removedMessageIds: string[] }
  | { kind: 'started'; conversationId: string; command: Command }
  | { kind: 'persisted'; conversationId: string; conversation: Conversation }
  | { kind: 'failed'; conversationId: string; command: Command; error: Error; clearPreview: boolean }
  | { kind: 'rejected'; conversationId: string; error: Error }
  | { kind: 'settled'; conversationId: string }

export type RunCommandResult =
  | { kind: 'persisted'; conversation: Conversation }
  | { kind: 'failed'; error: Error }
  | { kind: 'rejected'; reason: 'no_conversation' | 'busy' | 'invalid_target' | 'not_last_turn' }

interface RunCommandDependencies {
  executionOwner: ExecutionOwner
  previewOwner: PreviewOwner
  persistence: Persistence
  settlementPorts: SettlementPorts
  presentation: { present: (event: RunCommandPresentationEvent) => void }
  now?: () => number
  newGroupId?: () => string
}

function errorFrom(value: unknown, fallback: string): Error {
  return value instanceof Error
    ? value
    : new Error(typeof value === 'string'
      ? value
      : typeof (value as { message?: unknown } | null)?.message === 'string'
        ? (value as { message: string }).message
        : fallback)
}

/** Owns two follow-up commands and their execution lifetime. The view only
 * projects semantic events; it never has to infer when to release a run. */
export function createChatRunCommands({
  executionOwner, previewOwner, persistence, settlementPorts, presentation,
  now = Date.now, newGroupId = () => `grp_${crypto.randomUUID()}`,
}: RunCommandDependencies) {
  const rejected = (
    conversationId: string | null,
    reason: Extract<RunCommandResult, { kind: 'rejected' }>['reason'],
    message?: string,
  ): RunCommandResult => {
    if (conversationId && message) {
      presentation.present({ kind: 'rejected', conversationId, error: new Error(message) })
    }
    return { kind: 'rejected', reason }
  }

  const execute = async (
    command: Command,
    conversationId: string,
    lease: ExecutionLease,
    invoke: () => Promise<Conversation>,
  ): Promise<RunCommandResult> => {
    let persisted: Conversation | null = null
    try {
      const updated = await invoke()
      persisted = updated
      try {
        presentation.present({ kind: 'persisted', conversationId, conversation: updated })
      } catch (error) {
        console.error('Failed to present Chat run command result:', error)
      }
      return { kind: 'persisted', conversation: updated }
    } catch (value) {
      const error = errorFrom(value, command === 'regenerate' ? '重新生成失败' : '换模型回答失败')
      console.error(`Failed to ${command}:`, value)
      const clearPreview = command === 'regenerate' && !previewOwner.freeze(conversationId)
      try {
        presentation.present({ kind: 'failed', conversationId, command, error, clearPreview })
      } catch (presentationError) {
        console.error('Failed to present Chat run command error:', presentationError)
      }
      return { kind: 'failed', error }
    } finally {
      await executionOwner.finish(lease, persisted, settlementPorts)
      try {
        presentation.present({ kind: 'settled', conversationId })
      } catch (error) {
        console.error('Failed to present Chat run command settlement:', error)
      }
    }
  }

  const abortStart = async (
    command: Command, conversationId: string, lease: ExecutionLease, value: unknown,
  ): Promise<RunCommandResult> => {
    const error = errorFrom(value, command === 'regenerate' ? '重新生成失败' : '换模型回答失败')
    console.error('Failed to start Chat run command:', value)
    const clearPreview = command === 'regenerate' && !previewOwner.freeze(conversationId)
    try {
      presentation.present({ kind: 'failed', conversationId, command, error, clearPreview })
    } catch (presentationError) {
      console.error('Failed to present Chat run command error:', presentationError)
    } finally {
      await executionOwner.finish(lease, null, settlementPorts)
      try {
        presentation.present({ kind: 'settled', conversationId })
      } catch (settlementError) {
        console.error('Failed to present Chat run command settlement:', settlementError)
      }
    }
    return { kind: 'failed', error }
  }

  return {
    async regenerate({ conversation, messageId, newContent }: {
      conversation: Conversation | null
      messageId: string
      newContent?: string
    }): Promise<RunCommandResult> {
      if (!conversation) return rejected(null, 'no_conversation')
      const conversationId = conversation.id
      if (executionOwner.snapshot(conversationId).inFlight) {
        return rejected(conversationId, 'busy', '该对话正在生成中，请稍后再试')
      }
      const targetIndex = conversation.messages.findIndex((message) => message.id === messageId)
      if (targetIndex < 0) return rejected(conversationId, 'invalid_target')
      const keepTarget = conversation.messages[targetIndex].role === 'user'
      const cutFrom = keepTarget ? targetIndex + 1 : targetIndex
      const trimmedContent = newContent?.trim() || undefined
      const keptMessages = conversation.messages.slice(0, cutFrom)
      if (keepTarget && trimmedContent) {
        keptMessages[targetIndex] = { ...keptMessages[targetIndex], content: trimmedContent }
      }
      const removedMessageIds = conversation.messages.slice(cutFrom).map((message) => message.id)
      const startedAt = now()
      const lease = executionOwner.begin({ conversationId, kind: 'regenerate', startedAt })
      if (!lease) return rejected(conversationId, 'busy', '该对话正在生成中，请稍后再试')
      try {
        presentation.present({
          kind: 'truncated', conversation: { ...conversation, messages: keptMessages }, removedMessageIds,
        })
        previewOwner.begin(conversationId, startedAt)
        presentation.present({ kind: 'started', conversationId, command: 'regenerate' })
      } catch (error) {
        return abortStart('regenerate', conversationId, lease, error)
      }
      return execute('regenerate', conversationId, lease, () => (
        persistence.regenerateMessage(conversationId, messageId, trimmedContent)
      ))
    },
    async replyWithModel({ conversation, messageId, providerId, model }: {
      conversation: Conversation | null
      messageId: string
      providerId: string
      model: string
    }): Promise<RunCommandResult> {
      if (!conversation) return rejected(null, 'no_conversation')
      const conversationId = conversation.id
      if (executionOwner.snapshot(conversationId).inFlight) {
        return rejected(conversationId, 'busy', '该对话正在生成中，请稍后再试')
      }
      const span = assistantTurnSpan(conversation.messages, messageId)
      if (!span || span.end !== conversation.messages.length - 1) {
        return rejected(conversationId, 'not_last_turn', '只能对最后一轮回答换模型')
      }
      const groupId = span.groupId || newGroupId()
      const startedAt = now()
      const lease = executionOwner.begin({
        conversationId, kind: 'replyWithModel', startedAt,
        group: { groupId, arms: [
          ...span.siblings.map((message) => ({
            providerId: message.provider_id ?? message.providerId ?? conversation.provider_id ?? '',
            model: message.model ?? conversation.model ?? '',
            messageId: message.id,
            streaming: false,
            content: message.content,
            reasoning: message.reasoning,
            toolCalls: message.tool_calls ?? message.toolCalls ?? [],
            segments: message.segments ?? [],
          })),
          { providerId, model },
        ] },
      })
      if (!lease) return rejected(conversationId, 'busy', '该对话正在生成中，请稍后再试')
      try {
        previewOwner.begin(conversationId, startedAt, 'group')
        presentation.present({ kind: 'started', conversationId, command: 'replyWithModel' })
      } catch (error) {
        return abortStart('replyWithModel', conversationId, lease, error)
      }
      return execute('replyWithModel', conversationId, lease, () => (
        persistence.replyWithModel(conversationId, messageId, providerId, model, groupId)
      ))
    },
  }
}
