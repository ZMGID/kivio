import { useEffect, useState } from 'react'
import { api } from '../../api/tauri'
import { chatApi } from '../api'
import type { ChatAssistant } from '../types'
import { useTauriEvent } from './useTauriEvent'

/** The current contact profile feeds both the title and composer mascot. */
export function useCurrentBotAppearance(assistantId: string | null): ChatAssistant | undefined {
  const [profile, setProfile] = useState<{ id: string; assistant?: ChatAssistant }>()
  const [revision, refresh] = useState(0)
  useTauriEvent(api.onChatAssistantsChanged, () => refresh(n => n + 1), [])
  useEffect(() => {
    let current = true
    if (assistantId) void chatApi.getAssistants().then(profiles => {
      if (current) setProfile({ id: assistantId, assistant: profiles.find(profile => profile.id === assistantId) })
    }).catch(() => { /* Keep a confirmed appearance through a transient metadata failure. */ })
    return () => { current = false }
  }, [assistantId, revision])
  // Never show the prior contact while the new contact's request is pending.
  return profile?.id === assistantId ? profile.assistant : undefined
}
