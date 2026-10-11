import { useEffect, useRef, useState, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { Users, Bot, Eye, EyeOff, Pin, Plus, Settings } from 'lucide-react'
import { Button, IconButton } from '../components/Button'
import { i18n } from '../components/i18n'
import { Input, Label } from '../settings/public/controls'
import { collaborationApi, decodeRoom } from './bot/collaboration'
import { refreshCollaboration, useBotCollaboration } from './bot/collaborationStore'
import { contactAvatarMood, contactSignal, deliveryStatusLabel, roomStatusLabel } from './bot/presence'
import { profileApi, type BotListItem } from './bot/profile'
import { isTauriRuntime } from './utils'
import { AgentIcon, ExtensionsIcon, KnowledgeIcon, MediaIcon, NotesIcon, PluginIcon, PortfolioIcon, TasksIcon } from '../settings/public/icons'
import { api } from '../api/tauri'
import { chatApi } from './api'
import { botAssistantId, isDedicatedMemberConversation } from './botIdentity'
import { BotAvatar } from './BotAvatar'
import { useTauriEvent } from './hooks/useTauriEvent'
import { useClampedMenuPosition } from './useClampedMenuPosition'
import type { SidebarProps, ExtensionsNavItem } from './Sidebar'
import type { ChatAssistant, ConversationListItem } from './types'

type Props = Pick<SidebarProps, 'lang' | 'currentConversationId' | 'generatingConversationIds' | 'attentionConversationIds' | 'botExecutionFeedback' | 'optimisticConversations' | 'refreshKey' | 'searchOpen' | 'onSelectConversation' | 'onOpenSettings' | 'onOpenExtensionsItem'> & {
  footer: ReactNode
  onNewBot: () => void
  onEditBot?: (assistant: ChatAssistant) => void
  onOpenRoom?: (roomId: string) => void
  activeRoomId?: string | null
}

export function BotRoster({ lang, currentConversationId, generatingConversationIds, optimisticConversations = [], refreshKey, searchOpen, onSelectConversation, onOpenSettings, onOpenExtensionsItem, onEditBot, footer, onNewBot, onOpenRoom, activeRoomId = null, attentionConversationIds, botExecutionFeedback }: Props) {
  const t = i18n[lang]
  const collaboration = useBotCollaboration()
  const [items, setItems] = useState<ConversationListItem[]>([])
  const [assistants, setAssistants] = useState<ChatAssistant[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState('')
  const [revision, refresh] = useState(0)
  const [query, setQuery] = useState('')
  const [matches, setMatches] = useState<Set<string>>(new Set())
  const [showHidden, setShowHidden] = useState(false)
  const [menu, setMenu] = useState<{ left: number; top: number; item?: ConversationListItem } | null>(null)
  const menuRef = useRef<HTMLDivElement>(null)
  const moreRef = useRef<HTMLButtonElement>(null)
  const menuPosition = useClampedMenuPosition(menuRef, menu ?? { left: 0, top: 0 })
  const pendingMutations = useRef(new Set<string>())
  const [savingIds, setSavingIds] = useState<ReadonlySet<string>>(new Set())
  const [creatingGroup, setCreatingGroup] = useState(false)
  const [groupTitle, setGroupTitle] = useState('')
  const [groupMembers, setGroupMembers] = useState<string[]>([])
  const [groupError, setGroupError] = useState('')
  const [groupBusy, setGroupBusy] = useState(false)
  const [botProfiles, setBotProfiles] = useState<BotListItem[]>([])
  const [directoryReady, setDirectoryReady] = useState(false)
  const [profileError, setProfileError] = useState('')
  const groupDialog = useRef<HTMLDialogElement>(null)
  useTauriEvent(api.onChatAssistantsChanged, () => refresh(n => n + 1), [])
  useEffect(() => {
    let stop = () => {}
    let current = true
    void profileApi.onChanged(() => { if (current) refresh(n => n + 1) }).then((unlisten) => {
      if (!current) unlisten()
      else stop = unlisten
    })
    return () => { current = false; stop() }
  }, [])
  useEffect(() => {
    if (!isTauriRuntime()) return
    let current = true
    void profileApi.list().then((entries) => {
      if (!current) return
      setBotProfiles(entries.filter((entry) => !isDedicatedMemberConversation(entry.conversationId)))
      setProfileError(entries.find((entry) => entry.readError)?.readError ?? '')
      setDirectoryReady(true)
    }).catch((err) => {
      if (!current) return
      setProfileError(err instanceof Error ? err.message : String(err))
      setDirectoryReady(false)
    })
    return () => { current = false }
  }, [refreshKey, revision])
  useEffect(() => {
    let current = true
    void Promise.all([chatApi.getBotConversations(), chatApi.getAssistants()]).then(([conversations, profiles]) => {
      if (!current) return
      setItems(conversations.filter(item => Boolean(botAssistantId(item.id))))
      setAssistants(profiles)
      setError('')
    }).catch(err => { if (current) setError(String(err)) }).finally(() => { if (current) setLoading(false) })
    return () => { current = false }
  }, [refreshKey, revision])
  const search = searchOpen ? query.trim().toLowerCase() : ''
  useEffect(() => {
    let current = true
    setMatches(new Set())
    if (!search) return
    const timer = window.setTimeout(() => {
      void chatApi.searchConversations(search, 100, true).then(hits => {
        if (current) setMatches(new Set(hits.filter(hit => botAssistantId(hit.id)).map(hit => hit.id)))
      }).catch(err => { if (current) setError(String(err)) })
    }, 180)
    return () => { current = false; window.clearTimeout(timer) }
  }, [search, revision, refreshKey])
  useEffect(() => {
    if (!menu) return
    const close = (event: MouseEvent) => { if (!menuRef.current?.contains(event.target as Node) && !moreRef.current?.contains(event.target as Node)) setMenu(null) }
    const key = (event: KeyboardEvent) => { if (event.key === 'Escape') setMenu(null) }
    window.addEventListener('mousedown', close); window.addEventListener('keydown', key)
    return () => { window.removeEventListener('mousedown', close); window.removeEventListener('keydown', key) }
  }, [menu])
  const update = async (item: ConversationListItem, changes: { archived?: boolean; pinned?: boolean }) => {
    if (pendingMutations.current.has(item.id)) return
    pendingMutations.current.add(item.id); setSavingIds(new Set(pendingMutations.current)); setMenu(null)
    try {
      await chatApi.updateConversation(item.id, changes)
      setItems(previous => previous.map(row => row.id === item.id ? { ...row, ...changes } : row))
      refresh(n => n + 1)
    } catch (err) { setError(String(err)) }
    finally { pendingMutations.current.delete(item.id); setSavingIds(new Set(pendingMutations.current)) }
  }
  useEffect(() => {
    if (!creatingGroup) return
    groupDialog.current?.showModal()
  }, [creatingGroup])
  const createGroup = async () => {
    if (groupBusy) return
    if (groupMembers.length === 0) { setGroupError(t.chatBotGroupNeedMember); return }
    setGroupBusy(true)
    setGroupError('')
    try {
      const raw = await collaborationApi.createRoom(groupTitle, groupMembers)
      const room = decodeRoom(raw)
      if (!room) throw new Error(t.chatBotExchangeFailed)
      await refreshCollaboration()
      setCreatingGroup(false)
      setGroupTitle('')
      setGroupMembers([])
      onOpenRoom?.(room.id)
    } catch (err) {
      setGroupError(err instanceof Error ? err.message : String(err))
    } finally {
      setGroupBusy(false)
    }
  }
  const profiles = new Map(assistants.map(assistant => [assistant.id, assistant]))
  const directoryNames = new Map(botProfiles.map(entry => [entry.assistantId, entry.name]))
  const memberName = (member: { botId: string; name: string }) => directoryNames.get(member.botId) || member.name
  const rooms = collaboration.snapshot.rooms.filter(room => {
    if (!search) return true
    return [room.title, ...room.members.map(memberName)].some(text => text.toLowerCase().includes(search))
  }).sort((a, b) => b.updatedAt - a.updatedAt || a.id.localeCompare(b.id))
  const contacts = directoryReady
    ? botProfiles.map(entry => {
        const match = items.find(item => item.id === entry.conversationId || botAssistantId(item.id) === entry.assistantId)
        return match
          ? { ...match, assistant_id: entry.assistantId, assistant_name: entry.name }
          : { id: entry.conversationId, title: entry.name, preview: '', provider_id: '', model: '', message_count: 0, created_at: 0, updated_at: 0, pinned: false, archived: false, assistant_id: entry.assistantId, assistant_name: entry.name }
      })
    : items
  const live = new Map(optimisticConversations.map(item => [item.id, item]))
  const rows = contacts.map(item => ({ ...item, ...(live.get(item.id) ? { preview: live.get(item.id)!.preview, updated_at: live.get(item.id)!.updated_at } : {}) }))
    .filter(item => Boolean(item.archived) === showHidden)
    .filter(item => {
      const assistant = profiles.get(botAssistantId(item.id) ?? '')
      return !search || matches.has(item.id) || [assistant?.name, assistant?.description, item.title, item.preview].some(text => text?.toLowerCase().includes(search))
    })
    .sort((a, b) => Number(Boolean(b.pinned)) - Number(Boolean(a.pinned)) || b.updated_at - a.updated_at || a.id.localeCompare(b.id))
  const entries = [
    ...rows.map(item => ({ kind: 'bot' as const, item, id: item.id, updatedAt: item.updated_at, pinned: Boolean(item.pinned) })),
    ...(!showHidden ? rooms : []).map(room => ({ kind: 'room' as const, room, id: room.id, updatedAt: room.updatedAt, pinned: false })),
  ].sort((a, b) => Number(b.pinned) - Number(a.pinned) || b.updatedAt - a.updatedAt || a.id.localeCompare(b.id))
  const openMenu = (left: number, top: number, item?: ConversationListItem) => setMenu({ left: Math.max(8, Math.min(left, window.innerWidth - 240)), top: item ? Math.max(8, Math.min(top, window.innerHeight - 160)) : top, item })
  const action = (callback: () => void) => { setMenu(null); callback() }
  const extension = (item: ExtensionsNavItem) => action(() => onOpenExtensionsItem(item))
  const menuItem = menu?.item
  const menuAssistant = menuItem ? profiles.get(botAssistantId(menuItem.id) ?? '') : undefined
  return <div className="flex min-h-0 flex-1 flex-col" data-tauri-drag-region="false">
    {searchOpen && <div className="px-3 pb-3 pt-1"><Input autoFocus aria-label={t.chatBotSearch} placeholder={t.chatBotSearch} value={query} onChange={setQuery} /></div>}
    {showHidden && <p className="px-4 pb-2 text-xs text-neutral-500">{t.chatBotHidden}</p>}
    {(error || collaboration.error || profileError) && <div className="px-4 py-2 text-xs" role="alert"><p className="text-red-600">{error || collaboration.error || profileError}</p><Button size="sm" onClick={() => { refresh(n => n + 1); void refreshCollaboration() }}>{t.chatRetry}</Button></div>}
    <div className="custom-scrollbar min-h-0 flex-1 overflow-y-auto px-2 pb-3 pt-1">
      {loading ? <div className="space-y-1" aria-busy="true" aria-label={t.chatLoading}>{[0, 1, 2].map(i => <div key={i} className="kv-skeleton h-[60px] rounded-lg" />)}</div>
        : entries.length ? entries.map(entry => {
          if (entry.kind === 'room') {
            const room = entry.room
            const signal = contactSignal({
              generating: room.members.some(member => generatingConversationIds?.has(member.conversationId)),
              pending: collaboration.snapshot.pending.some(action => action.roomId === room.id && action.status === 'awaiting_user'),
              deliveries: collaboration.snapshot.deliveries.filter(delivery => delivery.roomId === room.id),
              attention: Boolean(attentionConversationIds && room.members.some(member => attentionConversationIds.has(member.conversationId))),
            })
            const latest = collaboration.snapshot.deliveries.filter(delivery => delivery.roomId === room.id)
              .sort((a, b) => b.updatedAt - a.updatedAt || b.id.localeCompare(a.id))[0]
            return <button type="button" key={`room:${room.id}`} aria-current={activeRoomId === room.id ? 'page' : undefined}
              onClick={() => onOpenRoom?.(room.id)}
              className={`mb-0.5 flex min-h-[60px] w-full items-center gap-2.5 rounded-lg px-2.5 py-2 text-left ${activeRoomId === room.id ? 'bg-neutral-900/[0.06]' : 'hover:bg-neutral-900/[0.04]'}`}>
              <span className="relative shrink-0"><Users size={28} className="m-0.5 text-neutral-500" aria-hidden="true" />{signal === 'working' && <span className="absolute bottom-0 right-0 h-2.5 w-2.5 rounded-full border-2 border-neutral-100 bg-emerald-500" />}{signal === 'attention' && <span className="absolute bottom-0 right-0 h-2.5 w-2.5 rounded-full border-2 border-neutral-100 bg-amber-500" />}</span>
              <span className="min-w-0 flex-1">
                <span className="block truncate text-[13px] font-medium text-neutral-900">{room.title}</span>
                <span className="mt-0.5 block truncate text-[12px] leading-4 text-neutral-600">{signal === 'working' ? t.chatBotWorking : signal === 'attention' ? (collaboration.snapshot.pending.some(action => action.roomId === room.id && action.status === 'awaiting_user') || !latest ? t.chatBotNeedsYou : deliveryStatusLabel(latest.status, t)) : `${roomStatusLabel(room.status, t)} · ${room.members.map(memberName).join(', ')}`}</span>
              </span>
            </button>
          }
          const item = entry.item
          const assistant = profiles.get(botAssistantId(item.id) ?? '')
          const assistantId = botAssistantId(item.id) ?? ''
          const botDeliveries = collaboration.snapshot.deliveries.filter(delivery => delivery.toConversationId === item.id || delivery.toBotId === assistantId)
          const feedback = botExecutionFeedback?.[item.id]
          const signal = feedback?.status === 'error' ? 'attention' : contactSignal({
            generating: Boolean(generatingConversationIds?.has(item.id)),
            pending: collaboration.snapshot.pending.some(action => action.botId === assistantId && action.status === 'awaiting_user'),
            deliveries: botDeliveries,
            attention: Boolean(attentionConversationIds?.has(item.id)),
          })
          const avatarMood = contactAvatarMood({
            error: feedback?.status === 'error',
            generating: Boolean(generatingConversationIds?.has(item.id)),
            pending: collaboration.snapshot.pending.some(action => action.botId === assistantId && action.status === 'awaiting_user'),
            deliveries: botDeliveries, attention: Boolean(attentionConversationIds?.has(item.id)),
          })
          const latest = [...botDeliveries].sort((a, b) => b.updatedAt - a.updatedAt || b.id.localeCompare(a.id))[0]
          const latestSuccess = [...botDeliveries].filter(delivery => delivery.status === 'succeeded').sort((a, b) => b.updatedAt - a.updatedAt || b.id.localeCompare(a.id))[0]
          const working = signal === 'working'
          return <button type="button" key={item.id} disabled={savingIds.has(item.id)}
            aria-current={!activeRoomId && currentConversationId === item.id ? 'page' : undefined}
            onClick={() => { if (item.archived) void update(item, { archived: false }); else onSelectConversation(item.id, item, { project: null, set: null }) }}
            onContextMenu={event => { event.preventDefault(); openMenu(event.clientX, event.clientY, item) }}
            onKeyDown={event => { if (event.key === 'F10' && event.shiftKey) { event.preventDefault(); const rect = event.currentTarget.getBoundingClientRect(); openMenu(rect.right - 224, rect.bottom, item) } }}
            className={`group mb-0.5 flex select-none min-h-[60px] w-full min-w-0 items-center gap-2.5 rounded-lg px-2.5 py-2 text-left transition-colors disabled:opacity-50 ${!activeRoomId && currentConversationId === item.id ? 'bg-neutral-900/[0.06]' : 'hover:bg-neutral-900/[0.04]'}`}>
            <span className="relative shrink-0"><BotAvatar icon={assistant?.icon} color={assistant?.color} size={36} mood={avatarMood} label={t.chatBotAvatarMoods.split('|')[({ idle: 0, think: 1, search: 2, work: 3, speak: 4, wait: 5, error: 6, done: 7 })[avatarMood]]} successKey={feedback?.status === 'completed' || latestSuccess ? `${feedback?.status === 'completed' ? feedback.key : ''}|${latestSuccess?.id ?? ''}` : undefined} />{working && <span className="absolute bottom-0 right-0 h-2.5 w-2.5 rounded-full border-2 border-neutral-100 bg-emerald-500" />}{signal === 'attention' && <span className="absolute bottom-0 right-0 h-2.5 w-2.5 rounded-full border-2 border-neutral-100 bg-amber-500" />}</span>
            <span className="min-w-0 flex-1">
              <span className="flex items-center gap-1.5"><span className="truncate text-[13px] font-medium text-neutral-900">{assistant?.name ?? item.assistant_name ?? item.title}</span>{botProfiles.some(entry => entry.assistantId === assistantId && entry.primary) && <span className="shrink-0 text-[11px] text-neutral-500">{t.chatBotPrimary}</span>}{item.pinned && <Pin size={11} className="shrink-0 text-neutral-400" />}</span>
              <span className="mt-0.5 block truncate text-[12px] leading-4 text-neutral-600">{item.archived ? t.chatBotRestore : working ? t.chatBotWorking : avatarMood === 'error' ? t.chatBotAvatarMoods.split('|')[6] : signal === 'attention' ? (collaboration.snapshot.pending.some(action => action.botId === assistantId && action.status === 'awaiting_user') || !latest ? t.chatBotNeedsYou : deliveryStatusLabel(latest.status, t)) : item.preview || assistant?.description || t.chatBotNoMessages}</span>
            </span>
          </button>
        }) : <div className="px-3 py-10 text-center" role="status">
          {!search && !showHidden && <div className="mb-4 flex justify-center"><BotAvatar color="var(--text-muted)" size={46} /></div>}
          <p className="text-[13px] leading-relaxed text-neutral-500">{search ? t.chatNoMatchingConversations : showHidden ? t.chatBotNoHidden : t.chatBotEmpty}</p>
          {!search && !showHidden && <div className="mt-4"><Button onClick={onNewBot}><Plus size={14} />{t.chatBotNew}</Button></div>}
        </div>}
    </div>
    <div className="flex shrink-0 items-center gap-2 border-t border-neutral-200/60 p-1.5"><div className="min-w-0 flex-1">{footer}</div><IconButton ref={moreRef} size="sm" variant="ghost" label={t.chatBotWorkspace} aria-haspopup="menu" aria-expanded={Boolean(menu && !menu.item)} onClick={event => { if (menu && !menu.item) setMenu(null); else { const rect = event.currentTarget.getBoundingClientRect(); openMenu(rect.right - 224, rect.top) } }}><ExtensionsIcon /></IconButton></div>
    {menu && createPortal(<div ref={menuRef} className="kv-menu custom-scrollbar fixed z-[200] max-h-[calc(100vh-16px)] min-w-[224px] overflow-y-auto" style={menu?.item ? menuPosition : { left: menuPosition.left, bottom: window.innerHeight - menu.top + 8, maxHeight: Math.max(120, menu.top - 16) }} role="menu">
      {menuItem ? <>
        {menuAssistant && onEditBot && <button type="button" role="menuitem" className="kv-menu-item" onClick={() => action(() => onEditBot(menuAssistant))}><Bot />{t.chatBotDetails}</button>}
        {menuAssistant && isTauriRuntime() && <button type="button" role="menuitem" className="kv-menu-item" onClick={() => action(() => {
          const primary = botProfiles.some(entry => entry.assistantId === menuAssistant.id && entry.primary)
          void profileApi.setPrimary(menuAssistant.id, !primary).then(() => refresh(n => n + 1)).catch(err => setProfileError(err instanceof Error ? err.message : String(err)))
        })}>{botProfiles.some(entry => entry.assistantId === menuAssistant.id && entry.primary) ? t.chatBotClearPrimary : t.chatBotSetPrimary}</button>}
        <button type="button" role="menuitem" className="kv-menu-item" onClick={() => void update(menuItem, { pinned: !menuItem.pinned })}><Pin />{menuItem.pinned ? t.chatBotUnpin : t.chatBotPin}</button>
        <button type="button" role="menuitem" className="kv-menu-item" onClick={() => void update(menuItem, { archived: !menuItem.archived })}>{menuItem.archived ? <Eye /> : <EyeOff />}{menuItem.archived ? t.chatBotRestore : t.chatBotHide}</button>
      </> : <>
        <button type="button" role="menuitem" className="kv-menu-item" onClick={() => action(() => { setGroupError(''); setCreatingGroup(true) })}><Plus />{t.chatBotNewGroup}</button>
        <div className="my-1 border-t border-neutral-200/80" />
        <button type="button" role="menuitem" className="kv-menu-item" onClick={() => extension('plugins')}><PluginIcon />{t.chatNavPlugins}</button>
        <button type="button" role="menuitem" className="kv-menu-item" onClick={() => extension('tasks')}><TasksIcon />{t.chatNavTasks}</button>
        <button type="button" role="menuitem" className="kv-menu-item" onClick={() => extension('artifacts')}><PortfolioIcon />{t.chatNavArtifacts}</button>
        <div className="my-1 border-t border-neutral-200/80" />
        <button type="button" role="menuitem" className="kv-menu-item" onClick={() => extension('assistants')}><AgentIcon />{t.chatNavAssistants}</button>
        <button type="button" role="menuitem" className="kv-menu-item" onClick={() => extension('knowledge')}><KnowledgeIcon />{t.chatNavKnowledge}</button>
        <button type="button" role="menuitem" className="kv-menu-item" onClick={() => extension('media')}><MediaIcon />{t.chatNavMedia}</button>
        <button type="button" role="menuitem" className="kv-menu-item" onClick={() => extension('notes')}><NotesIcon />{t.chatNavNotes}</button>
        <button type="button" role="menuitem" className="kv-menu-item" onClick={() => action(onOpenSettings)}><Settings />{t.settings}</button>
        <div className="my-1 border-t border-neutral-200/80" />
        <button type="button" role="menuitem" className="kv-menu-item" onClick={() => action(() => setShowHidden(value => !value))}><EyeOff />{t.chatBotHidden}{showHidden ? ' ✓' : ''}</button>
      </>}
    </div>, document.body)}
    {creatingGroup && createPortal(<dialog ref={groupDialog} className="kv-modal" aria-label={t.chatBotNewGroup}
      onCancel={event => { event.preventDefault(); if (!groupBusy) setCreatingGroup(false) }}>
      <form className="flex flex-col gap-4" onSubmit={event => { event.preventDefault(); void createGroup() }}>
        <h2 className="text-[14px] font-semibold text-neutral-900">{t.chatBotNewGroup}</h2>
        <label className="flex flex-col gap-1.5"><Label>{t.chatBotGroupTitle}</Label><Input value={groupTitle} disabled={groupBusy} onChange={setGroupTitle} /></label>
        <fieldset className="flex max-h-48 flex-col gap-1 overflow-y-auto custom-scrollbar" disabled={groupBusy}>
          <legend className="mb-1 text-[12px] text-neutral-500">{t.chatBotGroupMembers}</legend>
          {botProfiles.length === 0 && <p className="text-[12px] text-neutral-500">{directoryReady ? t.chatBotEmpty : t.chatLoading}</p>}
          {botProfiles.map(entry => (
            <label key={entry.assistantId} className="flex items-center gap-2 text-[13px] text-neutral-800">
              <input type="checkbox" checked={groupMembers.includes(entry.assistantId)} onChange={event => {
                setGroupMembers(current => {
                  if (!event.target.checked) return current.filter(id => id !== entry.assistantId)
                  return current.length >= 8 ? current : [...current, entry.assistantId]
                })
              }} />
              {entry.name}
            </label>
          ))}
        </fieldset>
        {groupError && <p className="text-xs text-red-600" role="alert">{groupError}</p>}
        <div className="flex justify-end gap-2">
          <Button disabled={groupBusy} onClick={() => setCreatingGroup(false)}>{t.cancel}</Button>
          <Button type="submit" variant="primary" disabled={groupBusy}>{t.chatBotCreateGroup}</Button>
        </div>
      </form>
    </dialog>, document.body)}
  </div>
}
