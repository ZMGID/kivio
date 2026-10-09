import { useEffect, useRef, useState, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { ChevronDown, SlidersHorizontal } from 'lucide-react'
import { IconButton } from '../components/Button'
import { useT } from '../components/i18n'
import { api } from '../api/tauri'
import { chatApi } from './api'
import { BotAvatar } from './BotAvatar'
import { botAssistantId } from './botIdentity'
import { useTauriEvent } from './hooks/useTauriEvent'
import type { Conversation } from './types'

/** One contact identity; conversation controls disclose without changing their owners. */
export function BotConversationHeader({ conversation, onEdit, controls }: {
  conversation: Conversation | null
  onEdit: () => void
  controls: ReactNode
}) {
  const t = useT()
  const assistant = conversation?.assistant_snapshot ?? conversation?.assistantSnapshot
  const assistantId = botAssistantId(conversation?.id)
  const [color, setColor] = useState<string>()
  const [revision, refresh] = useState(0)
  const [anchor, setAnchor] = useState<{ top: number; right: number } | null>(null)
  const trigger = useRef<HTMLButtonElement>(null)
  const panel = useRef<HTMLDivElement>(null)
  useTauriEvent(api.onChatAssistantsChanged, () => refresh(n => n + 1), [])
  useEffect(() => {
    let current = true
    setColor(undefined)
    if (assistantId) void chatApi.getAssistants().then(profiles => {
      if (current) setColor(profiles.find(profile => profile.id === assistantId)?.color)
    }).catch(() => { /* Identity remains readable when profile metadata is unavailable. */ })
    return () => { current = false }
  }, [assistantId, revision])
  useEffect(() => { setAnchor(null) }, [conversation?.id])
  useEffect(() => {
    if (!anchor) return
    const close = (event: MouseEvent) => {
      // Model and permission pickers portal their own panels outside this disclosure.
      if (!(event.target instanceof Element)) return
      if (!panel.current?.contains(event.target) && !trigger.current?.contains(event.target)
        && !event.target.closest('.kv-menu, [role="dialog"], [role="listbox"]')) setAnchor(null)
    }
    const key = (event: KeyboardEvent) => { if (event.key === 'Escape') { setAnchor(null); trigger.current?.focus() } }
    window.addEventListener('mousedown', close)
    window.addEventListener('keydown', key)
    return () => { window.removeEventListener('mousedown', close); window.removeEventListener('keydown', key) }
  }, [anchor])
  if (!assistantId || !assistant) return null
  return <div className="relative flex min-w-0 flex-1 items-center justify-center" data-tauri-drag-region>
    <button type="button" className="flex max-w-[calc(100%-72px)] items-center gap-2 rounded-lg px-2 py-1 text-[14px] font-semibold text-neutral-900 transition-colors hover:bg-neutral-900/[0.04]"
      aria-label={t.chatBotDetails} onClick={onEdit} data-tauri-drag-region="false">
      <BotAvatar color={color} size={28} /><span className="truncate">{assistant.name}</span><ChevronDown size={12} className="shrink-0 text-neutral-500" />
    </button>
    <div className="absolute right-0" data-tauri-drag-region="false">
      <IconButton ref={trigger} size="md" label={t.chatBotConversationSettings} aria-expanded={Boolean(anchor)} aria-haspopup="dialog"
        onClick={event => { const rect = event.currentTarget.getBoundingClientRect(); setAnchor(anchor ? null : { top: rect.bottom + 8, right: Math.max(12, window.innerWidth - rect.right) }) }}>
        <SlidersHorizontal size={16} />
      </IconButton>
    </div>
    {anchor && createPortal(<div ref={panel} role="dialog" aria-label={t.chatBotConversationSettings}
      className="kv-menu fixed z-[100] flex w-[min(440px,calc(100vw-24px))] flex-wrap items-center gap-3 p-3" style={anchor}>
      {controls}
    </div>, document.body)}
  </div>
}
