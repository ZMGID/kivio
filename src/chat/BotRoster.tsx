import { useEffect, useRef, useState, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { Bot, Eye, EyeOff, MoreHorizontal, PanelsTopLeft, Pin, Plus, Settings } from 'lucide-react'
import { Button } from '../components/Button'
import { i18n } from '../components/i18n'
import { Input } from '../settings/public/controls'
import { AgentIcon, KnowledgeIcon, MediaIcon, NotesIcon, PluginIcon, PortfolioIcon, TasksIcon } from '../settings/public/icons'
import { api } from '../api/tauri'
import { chatApi } from './api'
import { botAssistantId } from './botIdentity'
import { BotAvatar } from './BotAvatar'
import { useTauriEvent } from './hooks/useTauriEvent'
import { useClampedMenuPosition } from './useClampedMenuPosition'
import type { SidebarProps, ExtensionsNavItem } from './Sidebar'
import type { ChatAssistant, ConversationListItem } from './types'
import type { SidebarView } from './persistence'

type Props = Pick<SidebarProps, 'lang' | 'currentConversationId' | 'generatingConversationIds' | 'optimisticConversations' | 'refreshKey' | 'searchOpen' | 'onSelectConversation' | 'onOpenSettings' | 'onOpenExtensionsItem'> & {
  footer: ReactNode
  onNewBot: () => void
  onEditBot?: (assistant: ChatAssistant) => void
  onSwitchView: (view: SidebarView) => void
}

export function BotRoster({ lang, currentConversationId, generatingConversationIds, optimisticConversations = [], refreshKey, searchOpen, onSelectConversation, onOpenSettings, onOpenExtensionsItem, onEditBot, onSwitchView, footer, onNewBot }: Props) {
  const t = i18n[lang]
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
  useTauriEvent(api.onChatAssistantsChanged, () => refresh(n => n + 1), [])
  useEffect(() => {
    let current = true
    void Promise.all([chatApi.getBotConversations(), chatApi.getAssistants()]).then(([conversations, profiles]) => {
      if (!current) return
      setItems(conversations); setAssistants(profiles); setError('')
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
  const profiles = new Map(assistants.map(assistant => [assistant.id, assistant]))
  const live = new Map(optimisticConversations.map(item => [item.id, item]))
  const rows = items.map(item => ({ ...item, ...(live.get(item.id) ? { preview: live.get(item.id)!.preview, updated_at: live.get(item.id)!.updated_at } : {}) }))
    .filter(item => Boolean(item.archived) === showHidden)
    .filter(item => {
      const assistant = profiles.get(botAssistantId(item.id) ?? '')
      return !search || matches.has(item.id) || [assistant?.name, assistant?.description, item.title, item.preview].some(text => text?.toLowerCase().includes(search))
    })
    .sort((a, b) => Number(Boolean(b.pinned)) - Number(Boolean(a.pinned)) || b.updated_at - a.updated_at || a.id.localeCompare(b.id))
  const openMenu = (left: number, top: number, item?: ConversationListItem) => setMenu({ left: Math.max(8, Math.min(left, window.innerWidth - 240)), top: item ? Math.max(8, Math.min(top, window.innerHeight - 160)) : top, item })
  const action = (callback: () => void) => { setMenu(null); callback() }
  const extension = (item: ExtensionsNavItem) => action(() => onOpenExtensionsItem(item))
  const menuItem = menu?.item
  const menuAssistant = menuItem ? profiles.get(botAssistantId(menuItem.id) ?? '') : undefined
  return <div className="flex min-h-0 flex-1 flex-col" data-tauri-drag-region="false">
    {searchOpen && <div className="px-3 pb-3 pt-1"><Input autoFocus aria-label={t.chatBotSearch} placeholder={t.chatBotSearch} value={query} onChange={setQuery} /></div>}
    {showHidden && <p className="px-4 pb-2 text-xs text-neutral-500">{t.chatBotHidden}</p>}
    {error && <div className="px-4 py-2 text-xs" role="alert"><p className="text-red-600">{error}</p><Button size="sm" onClick={() => refresh(n => n + 1)}>{t.chatRetry}</Button></div>}
    <div className="custom-scrollbar min-h-0 flex-1 overflow-y-auto px-2.5 pb-3 pt-2">
      {loading ? <div className="space-y-2" aria-busy="true" aria-label={t.chatLoading}>{[0, 1, 2].map(i => <div key={i} className="kv-skeleton h-16 rounded-xl" />)}</div>
        : rows.length ? rows.map(item => {
          const assistant = profiles.get(botAssistantId(item.id) ?? '')
          const working = generatingConversationIds?.has(item.id)
          return <button type="button" key={item.id} disabled={savingIds.has(item.id)}
            aria-current={currentConversationId === item.id ? 'page' : undefined}
            onClick={() => { if (item.archived) void update(item, { archived: false }); else onSelectConversation(item.id, item, { project: null, set: null }) }}
            onContextMenu={event => { event.preventDefault(); openMenu(event.clientX, event.clientY, item) }}
            onKeyDown={event => { if (event.key === 'F10' && event.shiftKey) { event.preventDefault(); const rect = event.currentTarget.getBoundingClientRect(); openMenu(rect.right - 224, rect.bottom, item) } }}
            className={`group mb-1 flex select-none min-h-[76px] w-full min-w-0 items-center gap-3 rounded-xl px-3 py-3 text-left transition-colors disabled:opacity-50 ${currentConversationId === item.id ? 'bg-neutral-900/[0.085]' : 'hover:bg-neutral-900/[0.04]'}`}>
            <span className="relative shrink-0"><BotAvatar color={assistant?.color} size={42} />{working && <span className="absolute bottom-0 right-0 h-2.5 w-2.5 rounded-full border-2 border-neutral-100 bg-emerald-500" />}</span>
            <span className="min-w-0 flex-1">
              <span className="flex items-center gap-1.5"><span className="truncate text-[14px] font-semibold text-neutral-900">{assistant?.name ?? item.assistant_name ?? item.title}</span>{item.pinned && <Pin size={11} className="shrink-0 text-neutral-400" />}</span>
              <span className="mt-1 block truncate text-[13px] leading-5 text-neutral-500">{item.archived ? t.chatBotRestore : working ? t.chatBotWorking : item.preview || assistant?.description || t.chatBotNoMessages}</span>
            </span>
          </button>
        }) : <div className="px-3 py-10 text-center" role="status">
          {!search && !showHidden && <div className="mb-4 flex justify-center"><BotAvatar color="var(--text-muted)" size={46} /></div>}
          <p className="text-[13px] leading-relaxed text-neutral-500">{search ? t.chatNoMatchingConversations : showHidden ? t.chatBotNoHidden : t.chatBotEmpty}</p>
          {!search && !showHidden && <div className="mt-4"><Button onClick={onNewBot}><Plus size={14} />{t.chatBotNew}</Button></div>}
        </div>}
    </div>
    <div className="flex shrink-0 items-center gap-2 px-3 pb-3 pt-2"><div className="min-w-0 flex-1">{footer}</div><Button ref={moreRef} aria-haspopup="menu" aria-expanded={Boolean(menu && !menu.item)} onClick={event => { if (menu && !menu.item) setMenu(null); else { const rect = event.currentTarget.getBoundingClientRect(); openMenu(rect.right - 224, rect.top) } }}><MoreHorizontal size={17} />{t.chatBotMore}</Button></div>
    {menu && createPortal(<div ref={menuRef} className="kv-menu custom-scrollbar fixed z-[200] max-h-[calc(100vh-16px)] min-w-[224px] overflow-y-auto" style={menu?.item ? menuPosition : { left: menuPosition.left, bottom: window.innerHeight - menu.top + 8, maxHeight: Math.max(120, menu.top - 16) }} role="menu">
      {menuItem ? <>
        {menuAssistant && onEditBot && <button type="button" role="menuitem" className="kv-menu-item" onClick={() => action(() => onEditBot(menuAssistant))}><Bot />{t.chatBotDetails}</button>}
        <button type="button" role="menuitem" className="kv-menu-item" onClick={() => void update(menuItem, { pinned: !menuItem.pinned })}><Pin />{menuItem.pinned ? t.chatBotUnpin : t.chatBotPin}</button>
        <button type="button" role="menuitem" className="kv-menu-item" onClick={() => void update(menuItem, { archived: !menuItem.archived })}>{menuItem.archived ? <Eye /> : <EyeOff />}{menuItem.archived ? t.chatBotRestore : t.chatBotHide}</button>
      </> : <>
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
        <button type="button" role="menuitem" className="kv-menu-item" onClick={() => action(() => onSwitchView('flat'))}><PanelsTopLeft />{t.chatSidebarUseFlat}</button>
        <button type="button" role="menuitem" className="kv-menu-item" onClick={() => action(() => onSwitchView('classic'))}><PanelsTopLeft />{t.chatSidebarUseClassic}</button>
      </>}
    </div>, document.body)}
  </div>
}
