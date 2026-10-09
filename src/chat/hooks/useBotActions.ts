import { useCallback, useRef, useState } from 'react'
import { api } from '../../api/tauri'
import { i18n, type Lang } from '../../components/i18n'
import { chatApi } from '../api'
import { abandonAssistantEdit, beginAssistantEdit, createBlankAssistant } from '../assistantDraftStore'
import { botAssistantId } from '../botIdentity'
import { getRememberedSidebarView } from '../persistence'
import { setCoarse } from '../streamingStore'
import type { ChatAssistant } from '../types'
import type { UseAssistantActionsOptions } from './useAssistantActions'
import { useTauriEvent } from './useTauriEvent'

type Options = Omit<UseAssistantActionsOptions, 'identity'> & {
  lang: Lang
  runAfterLeavingEditors: (action: () => void) => void
  openAssistantCenter: () => void
  startAssistantChat: (assistant: ChatAssistant) => Promise<void>
}

/** Bot UI intents reuse the existing draft, navigation and conversation owners. */
export function useBotActions({ lang, navigation, currentConversationRef, refreshSidebar, refreshContextStats,
  applyConversationIfCurrent, setStreamErrorForConversation, setAssistantStreamStatsByMessageId,
  runAfterLeavingEditors, openAssistantCenter, startAssistantChat }: Options) {
  const [dialogOpen, setDialogOpen] = useState(false)
  const refreshSequence = useRef(0)
  const beginNewBot = useCallback(() => {
    beginAssistantEdit(createBlankAssistant(i18n[lang].chatBotNew), false)
    setDialogOpen(true)
  }, [lang])
  const editBot = useCallback((assistant: ChatAssistant) => {
    runAfterLeavingEditors(() => { beginAssistantEdit(assistant, true); setDialogOpen(true) })
  }, [runAfterLeavingEditors])
  const openBot = useCallback(async (assistant: ChatAssistant) => {
    const creation = navigation.beginConversationCreation()
    try {
      const conversation = await chatApi.openBot(assistant.id)
      refreshSidebar()
      if (navigation.commitCreatedConversation(creation, conversation)) {
        setAssistantStreamStatsByMessageId({})
        setCoarse({ streamError: '' })
      }
    } catch (err) {
      if (navigation.isConversationCreationCurrent(creation)) setCoarse({ streamError: String(err) })
      throw err
    }
  }, [navigation, refreshSidebar, setAssistantStreamStatsByMessageId])
  const refreshCurrentBot = useCallback(async (assistantId: string) => {
    const current = currentConversationRef.current
    if (!current || botAssistantId(current.id) !== assistantId) return
    const sequence = ++refreshSequence.current
    const updated = await chatApi.getConversationWindow(current.id)
    const latest = currentConversationRef.current
    if (latest?.id !== current.id || sequence !== refreshSequence.current) return
    // Profile edits must not replace loaded older history or in-flight messages.
    applyConversationIfCurrent(current.id, { ...latest, title: updated.title,
      revision: Math.max(latest.revision ?? 0, updated.revision ?? 0),
      assistant_snapshot: updated.assistant_snapshot, assistantSnapshot: updated.assistantSnapshot,
      provider_id: updated.provider_id, model: updated.model })
    void refreshContextStats(current.id)
  }, [applyConversationIfCurrent, currentConversationRef, refreshContextStats])
  useTauriEvent(api.onChatAssistantsChanged, assistantId => {
    refreshSidebar()
    void refreshCurrentBot(assistantId).catch(err => {
      const current = currentConversationRef.current
      if (current && botAssistantId(current.id) === assistantId) setStreamErrorForConversation(current.id, String(err))
    })
  }, [currentConversationRef, refreshCurrentBot, refreshSidebar, setStreamErrorForConversation])
  const finishEdit = useCallback(async (assistant: ChatAssistant, created: boolean) => {
    refreshSidebar()
    if (created) await openBot(assistant)
    else await refreshCurrentBot(assistant.id)
  }, [openBot, refreshCurrentBot, refreshSidebar])
  const closeDialog = useCallback(() => { setDialogOpen(false); abandonAssistantEdit() }, [])
  const openAdvanced = useCallback(() => { setDialogOpen(false); openAssistantCenter() }, [openAssistantCenter])
  const editCurrentBot = useCallback(() => {
    const current = currentConversationRef.current
    const assistantId = botAssistantId(current?.id)
    if (!assistantId || !current) return
    void chatApi.getAssistants().then(assistants => {
      if (currentConversationRef.current?.id !== current.id) return
      const assistant = assistants.find(item => item.id === assistantId)
      if (assistant) editBot(assistant)
    }).catch(err => setStreamErrorForConversation(current.id, String(err)))
  }, [currentConversationRef, editBot, setStreamErrorForConversation])
  const startAssistant = useCallback((assistant: ChatAssistant) => {
    if (getRememberedSidebarView() === 'bot') void openBot(assistant).catch(() => { /* openBot exposes the failure in chat. */ })
    else void startAssistantChat(assistant)
  }, [openBot, startAssistantChat])
  return { dialogOpen, beginNewBot, editBot, finishEdit, closeDialog, openAdvanced, editCurrentBot, startAssistant }
}
