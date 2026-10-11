import { useState } from 'react'
import { Button } from '../../components/Button'
import { useT } from '../../components/i18n'
import type { MemberAttention } from '../runInteractionInbox'
import { collaborationApi, type PendingAction } from './collaboration'
import { refreshCollaboration, useBotCollaboration } from './collaborationStore'
import { privateRunCards } from './privateRunAttention'

function label(action: PendingAction, t: ReturnType<typeof useT>) {
  if (action.kind === 'handoff') return t.chatBotPendingHandoff
  if (action.kind === 'remove_member') return t.chatBotPendingRemove
  return t.chatBotPendingAdd
}

/** Membership and handoff only. Ask and tool approval stay on the private member thread. */
export function BotPendingCards({ actions }: { actions: readonly PendingAction[] }) {
  const t = useT()
  const [resolving, setResolving] = useState<string | null>(null)
  const [error, setError] = useState('')
  if (actions.length === 0) return null
  const resolve = (action: PendingAction, approve: boolean) => {
    if (resolving) return
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
        <article key={action.id} className="rounded-lg border border-neutral-200/80 px-3 py-2">
          <p className="text-[13px] font-medium text-neutral-900">{label(action, t)}</p>
          <p className="mt-0.5 text-[12px] text-neutral-600">{action.botName}</p>
          <div className="mt-2 flex gap-2">
            <Button size="sm" variant="primary" disabled={resolving === action.id} onClick={() => resolve(action, true)}>{t.chatBotApprove}</Button>
            <Button size="sm" disabled={resolving === action.id} onClick={() => resolve(action, false)}>{t.chatBotReject}</Button>
          </div>
        </article>
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
        <article key={card.conversationId} className="rounded-lg border border-neutral-200/80 px-3 py-2">
          <p className="text-[13px] font-medium text-neutral-900">{card.name}</p>
          <p className="mt-0.5 text-[12px] text-neutral-600">
            {card.kind === 'question' ? t.chatBotPrivateQuestion : t.chatBotPrivateApproval}
          </p>
          <div className="mt-2">
            <Button size="sm" variant="primary" disabled={disabled} onClick={() => onOpen(card.botId)}>{t.chatBotOpenPending}</Button>
          </div>
        </article>
      ))}
    </div>
  )
}
