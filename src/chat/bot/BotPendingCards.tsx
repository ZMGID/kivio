import { useState } from 'react'
import { Button } from '../../components/Button'
import { useT } from '../../components/i18n'
import type { MemberAttention } from '../runInteractionInbox'
import { collaborationApi, type PendingAction } from './collaboration'
import { refreshCollaboration, useBotCollaboration } from './collaborationStore'
import { privateRunCards } from './privateRunAttention'

function label(action: PendingAction, t: ReturnType<typeof useT>) {
  const template = action.kind === 'handoff' ? t.chatBotPendingHandoffRequest
    : action.kind === 'remove_member' ? t.chatBotPendingRemoveRequest : t.chatBotPendingAddRequest
  return template.replace('{name}', action.botName)
}

/** Membership and handoff only. Ask and tool approval stay on the private member thread. */
export function BotPendingCards({ actions, disabled = false }: { actions: readonly PendingAction[]; disabled?: boolean }) {
  const t = useT()
  const [resolving, setResolving] = useState<string | null>(null)
  const [error, setError] = useState('')
  if (actions.length === 0) return null
  const resolve = (action: PendingAction, approve: boolean) => {
    if (resolving || disabled) return
    setResolving(action.id)
    setError('')
    void collaborationApi.resolve(action.id, approve).then(() => refreshCollaboration()).catch((err) => {
      setError(err instanceof Error ? err.message : String(err))
    }).finally(() => setResolving(null))
  }
  return (
    <div className="mb-3 flex flex-col gap-2">
      {error && <p className="text-[12px] text-red-600" role="alert">{error}</p>}
      {actions.map((action) => (
        <div key={action.id} className="flex flex-wrap items-center gap-x-3 gap-y-2 py-2">
          <p className="min-w-0 flex-1 text-[13px] leading-5 text-[var(--text)]">{label(action, t)}</p>
          <div className="flex gap-2" aria-busy={resolving === action.id}>
            <Button size="sm" variant="primary" disabled={disabled || resolving !== null} onClick={() => resolve(action, true)}>{t.chatBotApprove}</Button>
            <Button size="sm" variant="ghost" disabled={disabled || resolving !== null} onClick={() => resolve(action, false)}>{t.chatBotReject}</Button>
          </div>
        </div>
      ))}
    </div>
  )
}

export function BotConversationPending({ botId }: { botId: string }) {
  const { snapshot } = useBotCollaboration()
  const actions = snapshot.pending.filter((action) => action.status === 'awaiting_user' && action.botId === botId)
  if (!botId || actions.length === 0) return null
  return <div className="px-4 pt-3"><BotPendingCards actions={actions} /></div>
}

/** Opens the member thread's real chat pending card. The prompt and tool arguments stay there. */
export function BotPrivateRunCards({
  members,
  attention,
  disabled = false,
  onOpen,
}: {
  members: readonly { botId: string; name: string; conversationId: string }[]
  attention: readonly MemberAttention[]
  disabled?: boolean
  onOpen: (botId: string) => void
}) {
  const t = useT()
  const cards = privateRunCards(members, attention)
  if (cards.length === 0) return null
  return (
    <div className="mb-3 flex flex-col gap-2">
      {cards.map((card) => (
        <div key={card.conversationId} className="flex flex-wrap items-center gap-x-3 gap-y-1 py-1">
          <p className="min-w-0 flex-1 text-[13px] leading-5 text-[var(--text-muted)]">
            {(card.kind === 'question' ? t.chatBotPrivateQuestionWaiting : t.chatBotPrivateApprovalWaiting).replace('{name}', card.name)}
          </p>
          <Button size="sm" variant="ghost" disabled={disabled} onClick={() => onOpen(card.botId)}>{t.chatBotOpenPending}</Button>
        </div>
      ))}
    </div>
  )
}
