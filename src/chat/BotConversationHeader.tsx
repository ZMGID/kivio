import { useEffect, useState } from 'react'
import { ChevronDown } from 'lucide-react'
import { useT } from '../components/i18n'
import { api } from '../api/tauri'
import { chatApi } from './api'
import { BotAvatar } from './BotAvatar'
import { botContactId } from './botIdentity'
import { useTauriEvent } from './hooks/useTauriEvent'
import type { Conversation } from './types'

/** One contact identity; conversation controls disclose without changing their owners. */
export function BotConversationHeader({ conversation, onEdit, detailsOpen }: {
  conversation: Conversation | null
  onEdit: () => void
  detailsOpen: boolean
}) {
  const t = useT()
  const assistant = conversation?.assistant_snapshot ?? conversation?.assistantSnapshot
  const assistantId = botContactId(conversation?.id)
  const [color, setColor] = useState<string>()
  const [revision, refresh] = useState(0)
  useTauriEvent(api.onChatAssistantsChanged, () => refresh(n => n + 1), [])
  useEffect(() => {
    let current = true
    setColor(undefined)
    if (assistantId) void chatApi.getAssistants().then(profiles => {
      if (current) setColor(profiles.find(profile => profile.id === assistantId)?.color)
    }).catch(() => { /* Identity remains readable when profile metadata is unavailable. */ })
    return () => { current = false }
  }, [assistantId, revision])
  if (!assistantId || !assistant) return null
  return <div className="relative flex min-w-0 flex-1 items-center justify-center" data-tauri-drag-region>
    <button type="button" className="flex max-w-full items-center gap-2 rounded-lg px-2 py-1 text-[14px] font-semibold text-neutral-900 transition-colors hover:bg-neutral-900/[0.04]"
      aria-label={t.chatBotDetails} aria-expanded={detailsOpen} aria-controls="bot-settings-sidebar" onClick={onEdit} data-tauri-drag-region="false">
      <BotAvatar color={color} size={28} /><span className="truncate">{assistant.name}</span><ChevronDown size={12} className="shrink-0 text-neutral-500" />
    </button>
  </div>
}
