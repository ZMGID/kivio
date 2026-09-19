import { chatApi } from './api'
import { createChatExecutionOwner, type ExecutionLease, type PreparedRunOutcome } from './chatExecutionOwner'
import { prepareConversationForSend, type SendPreparationIntent } from './prepareConversationForSend'
import { createStreamPreviewOwner } from './streamPreviewOwner'
import type { Conversation, PendingAttachment } from './types'

type ExecutionOwner = ReturnType<typeof createChatExecutionOwner>
type PreviewOwner = ReturnType<typeof createStreamPreviewOwner>
type SettlementPorts = Parameters<ExecutionOwner['finish']>[2]
type Persistence = Pick<typeof chatApi, 'createConversation' | 'setAgentRuntime' | 'updateConversation' | 'sendMessage'>

export type SendResult =
  | { kind: 'persisted'; conversation: Conversation; composerAccepted: true }
  | { kind: 'persisted_error'; conversation: Conversation; error: Error; composerAccepted: true }
  | { kind: 'not_committed'; error: Error; partialConversation?: Conversation; composerAccepted: false }

/** The view owns route commitment; the send transaction only carries this
 * capability after it has successfully reserved the send. */
export interface SendCreationCommit {
  isCurrent: () => boolean
  commit: (conversation: Conversation) => boolean
}

export type SendPresentationEvent =
  | { kind: 'created'; conversation: Conversation; startingConversationId: string | null; creation: SendCreationCommit | null }
  | { kind: 'updated'; conversation: Conversation }
  | { kind: 'rejected'; error: Error; conversationId: string | null; startingConversationId: string | null; creation: SendCreationCommit | null }
  | { kind: 'started'; conversation: Conversation; content: string; attachments: PendingAttachment[]; fanOut: boolean }
  | { kind: 'outcome'; conversationId: string; outcome: PreparedRunOutcome }
  | { kind: 'settled' }

export interface SendIntent {
  content: string
  attachments: PendingAttachment[]
  preparation: SendPreparationIntent
  attachmentSkillId: string | null
  disabledReason: string
  planMessageId?: string
  onPartialConversation?: (conversation: Conversation) => void
  onAccepted?: () => void
}

interface SendPresentation {
  currentConversationId: () => string | null
  beginCreation?: () => SendCreationCommit
  present: (event: SendPresentationEvent) => void
}

interface SendDependencies {
  executionOwner: ExecutionOwner
  previewOwner: PreviewOwner
  persistence: Persistence
  settlementPorts: SettlementPorts
  presentation: SendPresentation
  now?: () => number
}

function withComposerResult(outcome: PreparedRunOutcome): SendResult {
  return outcome.kind === 'not_committed'
    ? { ...outcome, composerAccepted: false }
    : { ...outcome, composerAccepted: true }
}

/** One send transaction, from reservation through canonical prepare, invoke,
 * presentation and settlement. The view supplies a snapshot and receives
 * semantic projections; it cannot settle or release an execution itself. */
export function createChatSendController({
  executionOwner, previewOwner, persistence, settlementPorts, presentation, now = Date.now,
}: SendDependencies) {
  return {
    async send(intent: SendIntent): Promise<SendResult> {
      const content = intent.content.trim()
      const attachments = intent.attachments
      const startingConversationId = presentation.currentConversationId()
      let creation: SendCreationCommit | null = null
      const reject = (error: Error, conversationId: string | null, partialConversation?: Conversation): SendResult => {
        presentation.present({ kind: 'rejected', error, conversationId, startingConversationId, creation })
        return partialConversation
          ? { kind: 'not_committed', error, partialConversation, composerAccepted: false }
          : { kind: 'not_committed', error, composerAccepted: false }
      }
      if (!content && attachments.length === 0) {
        return { kind: 'not_committed', error: new Error('消息为空'), composerAccepted: false }
      }
      const preparation = intent.preparation
      const sendTarget = preparation.override && preparation.conversation
        ? preparation.conversation.id
        : preparation.forceNew ? null : startingConversationId
      if (!preparation.forceNew && intent.disabledReason) {
        return reject(new Error(intent.disabledReason), sendTarget)
      }
      const claim = executionOwner.claimSend(sendTarget)
      if (!claim) return reject(new Error('该对话正在发送中，请稍后再试'), sendTarget)

      let lease: ExecutionLease | null = null
      try {
        const prepared = await prepareConversationForSend(preparation, persistence, (phase, conversation) => {
          if (phase === 'created') {
            if (!executionOwner.bindSend(claim, conversation.id)) return false
            intent.onPartialConversation?.(conversation)
            presentation.present({ kind: 'created', conversation, startingConversationId, creation })
          } else {
            intent.onPartialConversation?.(conversation)
            presentation.present({ kind: 'updated', conversation })
          }
        }, () => { creation = presentation.beginCreation?.() ?? null })
        if (!prepared.ok) {
          console.error(`Failed to prepare conversation before send (${prepared.stage}):`, prepared.error)
          return reject(prepared.error, prepared.conversation?.id ?? null, prepared.conversation ?? undefined)
        }
        const conversation = prepared.conversation
        if (!executionOwner.bindSend(claim, conversation.id)) {
          return reject(new Error('该对话正在发送中，请稍后再试'), conversation.id)
        }
        if (executionOwner.snapshot(conversation.id).inFlight) {
          return reject(new Error('该对话正在生成中，请稍后再试'), conversation.id)
        }

        const replyArms = conversation.reply_models ?? conversation.replyModels ?? []
        const planMode = conversation.agent_plan_state?.mode ?? conversation.agentPlanState?.mode ?? 'act'
        const fanOut = replyArms.length >= 2 && planMode === 'act'
        const startedAt = now()
        lease = executionOwner.begin({
          conversationId: conversation.id, kind: 'send', startedAt, claim,
          optimistic: { content, attachments, stored: conversation.messages },
          group: fanOut ? {
            groupId: `grp-local-${startedAt}`,
            arms: replyArms.map((ref) => ({ providerId: ref.provider_id, model: ref.model })),
          } : undefined,
        })
        if (!lease) return reject(new Error('该对话正在生成中，请稍后再试'), conversation.id)

        if (presentation.currentConversationId() === conversation.id) previewOwner.activate(conversation.id)
        previewOwner.begin(conversation.id, startedAt, fanOut ? 'group' : 'single')
        presentation.present({ kind: 'started', conversation, content, attachments, fanOut })
        intent.onAccepted?.()

        const outcome = await executionOwner.submitPreparedRun({
          lease, content, attachments,
          attachmentSkillId: intent.attachmentSkillId,
          planMessageId: intent.planMessageId,
        }, {
          ...settlementPorts,
          onOutcome: (outcome) => presentation.present({ kind: 'outcome', conversationId: conversation.id, outcome }),
        })
        return withComposerResult(outcome)
      } finally {
        if (lease) await executionOwner.finish(lease, null, settlementPorts)
        executionOwner.abandonSend(claim)
        presentation.present({ kind: 'settled' })
      }
    },
  }
}
