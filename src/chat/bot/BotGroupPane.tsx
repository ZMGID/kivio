import { useEffect, useMemo, useRef, useState } from 'react'
import { ArrowLeft } from 'lucide-react'
import { Button, IconButton } from '../../components/Button'
import { confirmDialog } from '../../components/dialogQueue'
import { useLang, useT } from '../../components/i18n'
import { Input, Select, TextArea } from '../../settings/public/controls'
import { syncChatProtocol } from '../../api/chatProtocol'
import type { MemberAttention } from '../runInteractionInbox'
import { BotPendingCards, BotPrivateRunCards } from './BotPendingCards'
import { collaborationApi, decodeRoom, isDedicatedMemberConversation } from './collaboration'
import { refreshCollaboration, useBotCollaboration } from './collaborationStore'
import { requestPeerExchange } from './exchangeRequest'
import { roomPost, sendMentions } from './mentionPlan'
import { deliveryStatusLabel, roomStatusLabel } from './presence'
import { profileApi } from './profile'

function formatTime(at: number, lang: 'zh' | 'en') {
  return new Date(at * 1000).toLocaleString(lang === 'zh' ? 'zh-CN' : 'en-US', {
    month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hour12: false,
  })
}

function errorText(err: unknown) {
  return err instanceof Error ? err.message : String(err)
}

interface RoomComposer {
  body: string
  selected: string[]
  titleDraft: string
  renaming: boolean
  acceptedIds: string[]
  addId: string
}

const EMPTY_COMPOSER: RoomComposer = {
  body: '', selected: [], titleDraft: '', renaming: false, acceptedIds: [], addId: '',
}

export function BotGroupPane({ roomId, attention = [], onClose, onOpenMember }: {
  roomId: string
  attention?: readonly MemberAttention[]
  onClose: () => void
  onOpenMember?: (conversationId: string) => void
}) {
  const t = useT()
  const lang = useLang()
  const { snapshot, error: snapshotError } = useBotCollaboration()
  const room = snapshot.rooms.find((item) => item.id === roomId) ?? null
  const [contacts, setContacts] = useState<{ id: string; name: string }[]>([])
  const [contactsError, setContactsError] = useState('')
  const [directoryTick, setDirectoryTick] = useState(0)
  const [drafts, setDrafts] = useState<Record<string, RoomComposer>>({})
  const [roomErrors, setRoomErrors] = useState<Record<string, string>>({})
  const [busyRooms, setBusyRooms] = useState<ReadonlySet<string>>(new Set())
  const flights = useRef(new Map<string, number>())
  const seq = useRef(0)
  const seenRoom = useRef(roomId)
  const roomEpoch = useRef(0)
  if (seenRoom.current !== roomId) {
    seenRoom.current = roomId
    roomEpoch.current += 1
  }
  const draft = drafts[roomId] ?? EMPTY_COMPOSER
  const error = roomErrors[roomId] || contactsError
  const roomBusy = busyRooms.has(roomId)
  useEffect(() => {
    let current = true
    let stop = () => {}
    void profileApi.onChanged(() => { if (current) setDirectoryTick((value) => value + 1) }).then((unlisten) => {
      if (!current) unlisten()
      else stop = unlisten
    })
    return () => { current = false; stop() }
  }, [])
  useEffect(() => {
    let current = true
    void profileApi.list().then((entries) => {
      if (!current) return
      setContacts(entries
        .filter((entry) => !isDedicatedMemberConversation(entry.conversationId))
        .map((entry) => ({ id: entry.assistantId, name: entry.name })))
      setContactsError('')
    }).catch((err) => {
      if (current) setContactsError(errorText(err))
    })
    return () => { current = false }
  }, [directoryTick])
  const deliveries = useMemo(() => new Map(snapshot.deliveries.map((delivery) => [delivery.id, delivery])), [snapshot.deliveries])
  const pending = snapshot.pending.filter((action) => action.roomId === roomId && action.status === 'awaiting_user')
  const activeMembers = room?.members.filter((member) => member.state === 'active') ?? []
  const memberSyncKey = activeMembers
    .map((member) => member.conversationId)
    .filter((id) => isDedicatedMemberConversation(id))
    .join('\n')
  useEffect(() => {
    if (!memberSyncKey) return
    for (const id of memberSyncKey.split('\n')) void syncChatProtocol(id).catch(() => {})
  }, [memberSyncKey])
  const directory = useMemo(() => new Map(contacts.map((contact) => [contact.id, contact.name])), [contacts])
  const liveName = (member: { botId: string; name: string }) => directory.get(member.botId) || member.name
  const namedActive = activeMembers.map((member) => ({ botId: member.botId, name: liveName(member), conversationId: member.conversationId }))
  const candidates = contacts.filter((contact) => !room?.members.some((member) => member.botId === contact.id))
  const updateDraft = (targetId: string, patch: (current: RoomComposer) => RoomComposer) => {
    setDrafts((prev) => {
      const current = prev[targetId] ?? EMPTY_COMPOSER
      const next = patch(current)
      return next === current ? prev : { ...prev, [targetId]: next }
    })
  }
  const begin = (targetId: string) => {
    const ticket = ++seq.current
    flights.current.set(targetId, ticket)
    setBusyRooms((prev) => {
      if (prev.has(targetId)) return prev
      const next = new Set(prev)
      next.add(targetId)
      return next
    })
    setRoomErrors((prev) => prev[targetId] ? { ...prev, [targetId]: '' } : prev)
    return ticket
  }
  const finish = (targetId: string, ticket: number) => {
    if (flights.current.get(targetId) !== ticket) return
    flights.current.delete(targetId)
    setBusyRooms((prev) => {
      if (!prev.has(targetId)) return prev
      const next = new Set(prev)
      next.delete(targetId)
      return next
    })
  }
  const fail = (targetId: string, err: unknown) => {
    setRoomErrors((prev) => ({ ...prev, [targetId]: errorText(err) }))
  }
  const run = (targetId: string, action: () => Promise<void>) => {
    if (flights.current.has(targetId)) return
    const ticket = begin(targetId)
    void action().then(() => refreshCollaboration()).catch((err) => fail(targetId, err)).finally(() => finish(targetId, ticket))
  }
  const send = () => {
    if (!room || flights.current.has(room.id)) return
    const targetId = room.id
    const members = namedActive.map((member) => ({ botId: member.botId, name: member.name }))
    const names = new Map(members.map((member) => [member.botId, member.name]))
    const post = roomPost(members, draft.selected, draft.body)
    const pendingIds = post.mode === 'mention' ? post.ids.filter((id) => !draft.acceptedIds.includes(id)) : []
    if (post.mode === 'mention' && pendingIds.length === 0) return
    const text = draft.body.trim()
    const sentBody = draft.body
    const ticket = begin(targetId)
    const task = post.mode === 'broadcast'
      ? collaborationApi.broadcast(targetId, text).then(async () => {
          await refreshCollaboration()
          updateDraft(targetId, (current) => current.body === sentBody
            ? { ...current, body: '', selected: [], acceptedIds: [] }
            : current)
        })
      : sendMentions(pendingIds, (botId) => collaborationApi.mention(targetId, botId, text)).then(async (result) => {
          await refreshCollaboration()
          updateDraft(targetId, (current) => {
            if (current.body !== sentBody) return { ...current, acceptedIds: [] }
            if (result.error) {
              const acceptedIds = [...current.acceptedIds]
              for (const id of result.sent) if (!acceptedIds.includes(id)) acceptedIds.push(id)
              return { ...current, acceptedIds, selected: current.selected.filter((id) => !result.sent.includes(id)) }
            }
            return { ...current, body: '', selected: [], acceptedIds: [] }
          })
          if (result.error) {
            const sentNames = result.sent.map((id) => names.get(id) ?? id).join(', ')
            const message = errorText(result.error)
            setRoomErrors((prev) => ({
              ...prev,
              [targetId]: result.sent.length ? t.chatBotPartialSend.replace('{sent}', sentNames).replace('{error}', message) : message,
            }))
          }
        })
    void task.catch((err) => fail(targetId, err)).finally(() => finish(targetId, ticket))
  }
  const planned = room ? roomPost(namedActive, draft.selected, draft.body) : null
  const pendingMention = planned?.mode === 'mention' ? planned.ids.filter((id) => !draft.acceptedIds.includes(id)) : []
  const addressed = planned?.mode === 'mention'
  const openPrivate = (botId: string) => {
    if (!room || !onOpenMember || flights.current.has(room.id)) return
    const targetId = room.id
    const epoch = roomEpoch.current
    const ticket = begin(targetId)
    void collaborationApi.memberSession(targetId, botId).then((session) => {
      if (roomEpoch.current !== epoch) return
      onOpenMember(session.conversationId)
    }).catch((err) => fail(targetId, err)).finally(() => finish(targetId, ticket))
  }
  const removeMember = (botId: string, name: string) => {
    if (!room) return
    const target = room
    void confirmDialog({ message: t.chatBotRemoveMemberConfirm.replace('{name}', name), confirmLabel: t.chatBotRemoveMember, danger: true }).then((yes) => {
      if (!yes) return
      const next = target.members.filter((member) => member.botId !== botId).map((member) => member.botId)
      if (next.length === 0) {
        setRoomErrors((prev) => ({ ...prev, [target.id]: t.chatBotGroupNeedMember }))
        return
      }
      run(target.id, () => collaborationApi.setMembers(target.id, next).then(() => undefined))
    })
  }
  return (
    <section className="flex min-h-0 min-w-0 flex-1 flex-col" aria-label={room?.title ?? t.chatBotGroups}>
      <header className="flex h-14 shrink-0 items-center gap-2 px-4">
        <IconButton label={t.chatBotExchangeClose} onClick={onClose}><ArrowLeft size={16} /></IconButton>
        <div className="min-w-0 flex-1">
          {draft.renaming ? (
            <form className="flex items-center gap-2" onSubmit={(event) => {
              event.preventDefault()
              if (!room || flights.current.has(room.id)) return
              const targetId = room.id
              const titleDraft = draft.titleDraft
              const ticket = begin(targetId)
              void collaborationApi.renameRoom(targetId, titleDraft.trim()).then(async (raw) => {
                if (!decodeRoom(raw)) throw new Error(t.chatBotExchangeFailed)
                await refreshCollaboration()
                updateDraft(targetId, (current) => current.titleDraft === titleDraft ? { ...current, renaming: false } : current)
              }).catch((err) => fail(targetId, err)).finally(() => finish(targetId, ticket))
            }}>
              <Input value={draft.titleDraft} disabled={roomBusy} aria-label={t.chatBotGroupTitle} onChange={(value) => updateDraft(roomId, (current) => ({ ...current, titleDraft: value }))} />
              <Button size="sm" type="submit" variant="primary" disabled={roomBusy || !draft.titleDraft.trim()}>{t.save}</Button>
            </form>
          ) : (
            <h2 className="truncate text-[15px] font-semibold text-neutral-900">{room?.title ?? t.chatBotGroups}</h2>
          )}
          {room && <p className="text-[12px] text-neutral-500">{roomStatusLabel(room.status, t)}</p>}
        </div>
        {room && !draft.renaming && <Button size="sm" disabled={roomBusy} onClick={() => updateDraft(room.id, (current) => ({ ...current, titleDraft: room.title, renaming: true }))}>{t.chatBotRenameGroup}</Button>}
        {room?.status === 'active' && <Button size="sm" disabled={roomBusy} onClick={() => void run(room.id, () => collaborationApi.pauseRoom(room.id).then(() => undefined))}>{t.chatBotPause}</Button>}
        {room?.status === 'paused' && <Button size="sm" disabled={roomBusy} onClick={() => void run(room.id, () => collaborationApi.continueRoom(room.id).then(() => undefined))}>{t.chatBotContinue}</Button>}
        {room?.status === 'stopped' && <Button size="sm" disabled={roomBusy} onClick={() => void run(room.id, () => collaborationApi.continueRoom(room.id).then(() => undefined))}>{t.chatBotGroupResume}</Button>}
        {room && room.status !== 'stopped' && <Button size="sm" variant="danger" disabled={roomBusy} onClick={() => {
          const target = room
          void confirmDialog({ message: t.chatBotStopGroupConfirm, confirmLabel: t.chatBotStopGroup, danger: true }).then((yes) => {
            if (yes) run(target.id, () => collaborationApi.stopRoom(target.id).then(() => undefined))
          })
        }}>{t.chatBotStopGroup}</Button>}
      </header>
      <div className="custom-scrollbar min-h-0 flex-1 overflow-y-auto px-6 pb-4">
        {(error || snapshotError) && <p className="mb-3 text-[12px] text-red-600" role="alert">{error || snapshotError}</p>}
        {!room && <p className="text-[13px] text-neutral-500">{t.chatBotExchangeFailed}</p>}
        {room?.status === 'stopped' && <p className="mb-3 text-[12px] leading-5 text-neutral-500">{t.chatBotGroupResumeHint}</p>}
        {room && (
          <section className="mb-4" aria-label={t.chatBotGroupMembers}>
            <div className="mb-2 flex flex-wrap items-center gap-2">
              {room.members.map((member) => (
                <span key={member.botId} className="inline-flex items-center gap-1 rounded-full bg-neutral-900/[0.05] px-2 py-1 text-[12px]">
                  {liveName(member)}
                  {member.state === 'pending_removal' && <span className="text-neutral-500">{t.chatBotMemberLeaving}</span>}
                  {onOpenMember && (
                    <Button size="sm" variant={attention.some((item) => item.conversationId === member.conversationId) ? 'primary' : 'ghost'} disabled={roomBusy} onClick={() => openPrivate(member.botId)}>
                      {attention.some((item) => item.conversationId === member.conversationId) ? t.chatBotNeedsYou : t.chatBotOpenMemberThread}
                    </Button>
                  )}
                  <Button size="sm" variant="ghost" disabled={roomBusy} onClick={() => removeMember(member.botId, liveName(member))}>{t.chatBotRemoveMember}</Button>
                </span>
              ))}
            </div>
            {candidates.length > 0 && room.members.length < 8 && (
              <div className="flex items-center gap-2">
                <Select ariaLabel={t.chatBotAddMember} value={draft.addId} onChange={(addId) => updateDraft(room.id, (current) => ({ ...current, addId }))} options={[{ value: '', label: t.chatBotAddMember }, ...candidates.map((contact) => ({ value: contact.id, label: contact.name }))]} />
                <Button size="sm" disabled={roomBusy || !draft.addId} onClick={() => {
                  const addId = draft.addId
                  if (!addId) return
                  const next = [...room.members.map((member) => member.botId), addId]
                  updateDraft(room.id, (current) => ({ ...current, addId: '' }))
                  run(room.id, () => collaborationApi.setMembers(room.id, next).then(() => undefined))
                }}>{t.chatBotAddMember}</Button>
              </div>
            )}
          </section>
        )}
        <BotPrivateRunCards members={namedActive} attention={attention} disabled={roomBusy} onOpen={openPrivate} />
        <BotPendingCards actions={pending} />
        {room?.log.map((entry) => {
          const delivery = entry.deliveryId ? deliveries.get(entry.deliveryId) : undefined
          const authorId = entry.actor.kind === 'bot' ? entry.actor.id : null
          const authorName = entry.actor.kind === 'bot' ? entry.actor.name : ''
          const canReply = authorId != null && activeMembers.some((member) => member.botId === authorId)
          return (
            <article key={entry.id} className="border-b border-neutral-200/70 py-3">
              <p className="text-[12px] text-neutral-500">
                {entry.actor.kind === 'user' ? t.chatBotYou : entry.actor.name}
                {' · '}
                {formatTime(entry.at, lang)}
                {delivery && ` · ${deliveryStatusLabel(delivery.status, t)}`}
              </p>
              <p className="mt-1 whitespace-pre-wrap text-[14px] leading-6 text-neutral-900">{entry.body}</p>
              {delivery && delivery.attachmentIds && delivery.attachmentIds.length > 0 && (
                <p className="mt-1 text-[12px] text-neutral-500">{t.chatBotAttachmentCount.replace('{count}', String(delivery.attachmentIds.length))}</p>
              )}
              {(delivery || canReply) && (
                <div className="mt-2 flex gap-2">
                  {delivery && <Button size="sm" onClick={() => requestPeerExchange(delivery.exchangeId)}>{t.chatBotOpenExchange}</Button>}
                  {canReply && authorId && (
                    <Button size="sm" onClick={() => updateDraft(roomId, (current) => ({ ...current, selected: [authorId], acceptedIds: [] }))}>{t.chatBotReplyTo.replace('{name}', authorName)}</Button>
                  )}
                </div>
              )}
            </article>
          )
        })}
      </div>
      {room && (
        <form className="shrink-0 border-t border-neutral-200/70 px-6 py-3" onSubmit={(event) => { event.preventDefault(); send() }}>
          {room.status === 'paused' && <p className="mb-2 text-[12px] text-neutral-500">{t.chatBotPausedSendHint}</p>}
          <fieldset className="mb-2 flex flex-wrap gap-2" disabled={room.status === 'stopped' || roomBusy}>
            <legend className="sr-only">{t.chatBotGroupMembers}</legend>
            {activeMembers.map((member) => (
              <label key={member.botId} className="inline-flex items-center gap-1 text-[12px] text-neutral-700">
                <input type="checkbox" checked={draft.selected.includes(member.botId)} onChange={(event) => {
                  updateDraft(roomId, (current) => ({
                    ...current,
                    acceptedIds: [],
                    selected: event.target.checked ? [...current.selected, member.botId] : current.selected.filter((id) => id !== member.botId),
                  }))
                }} />
                {liveName(member)}
              </label>
            ))}
          </fieldset>
          <TextArea rows={3} value={draft.body} disabled={room.status === 'stopped' || roomBusy} placeholder={t.chatBotGroupComposer} onChange={(value) => updateDraft(roomId, (current) => current.body === value ? current : { ...current, body: value, acceptedIds: [] })} />
          <div className="mt-2 flex justify-end">
            <Button type="submit" variant="primary" disabled={roomBusy || room.status === 'stopped' || !draft.body.trim() || (addressed && pendingMention.length === 0)}>{addressed ? t.chatBotKindMention : t.chatBotBroadcast}</Button>
          </div>
        </form>
      )}
    </section>
  )
}
