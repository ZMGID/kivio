import { useCallback, useRef, useState } from 'react'
import { api } from '../../api/tauri'
import { i18n, type Lang } from '../../components/i18n'
import { chatApi } from '../api'
import { abandonAssistantEdit, beginAssistantEdit, createBlankAssistant } from '../assistantDraftStore'
import { botAssistantId, botContactId } from '../botIdentity'
import { draftKey, updateComposerDraft } from '../composerDraft'
import { isBotWorkspace } from '../workspace'
import { setCoarse } from '../streamingStore'
import type { ChatAssistant } from '../types'
import type { UseAssistantActionsOptions } from './useAssistantActions'
import { useTauriEvent } from './useTauriEvent'

type Options = Omit<UseAssistantActionsOptions, 'identity'> & {
  lang: Lang
  runAfterLeavingEditors: (action: () => void) => void
  startAssistantChat: (assistant: ChatAssistant) => Promise<void>
}

/** Bot UI intents reuse the existing draft, navigation and conversation owners. */
export function useBotActions({ lang, navigation, currentConversationRef, refreshSidebar, refreshContextStats,
  applyConversationIfCurrent, setStreamErrorForConversation, setAssistantStreamStatsByMessageId,
  runAfterLeavingEditors, startAssistantChat }: Options) {
  const [editorMode, setEditorMode] = useState<'create' | 'edit' | null>(null)
  const [editedAssistantId, setEditedAssistantId] = useState<string | null>(null)
  const refreshSequence = useRef(0)
  const pendingTaskPrompt = useRef<string | null>(null)
  const beginNewBot = useCallback(() => {
    beginAssistantEdit(createBlankAssistant(i18n[lang].chatBotNew), false)
    setEditedAssistantId(null)
    setEditorMode('create')
  }, [lang])
  const editBot = useCallback((assistant: ChatAssistant) => {
    runAfterLeavingEditors(() => { beginAssistantEdit(assistant, true); setEditedAssistantId(assistant.id); setEditorMode('edit') })
  }, [runAfterLeavingEditors])
  const openBot = useCallback(async (assistant: ChatAssistant) => {
    const creation = navigation.beginConversationCreation()
    try {
      const conversation = await chatApi.openBot(assistant.id)
      refreshSidebar()
      if (navigation.commitCreatedConversation(creation, conversation)) {
        setAssistantStreamStatsByMessageId({})
        setCoarse({ streamError: '' })
        if (pendingTaskPrompt.current !== null) {
          const prompt = pendingTaskPrompt.current
          updateComposerDraft(draftKey(conversation.id), draft => ({ ...draft, input: prompt }))
          pendingTaskPrompt.current = null
        }
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
    let updated = await chatApi.getConversationWindow(current.id)
    let latest = currentConversationRef.current
    if (latest?.id !== current.id || sequence !== refreshSequence.current) return
    if (updated.revision < latest.revision) {
      // A model edit or completed reply can overtake this read. Revalidate once;
      // continuous changes must not keep the editor waiting or promote stale metadata.
      updated = await chatApi.getConversationWindow(current.id)
      latest = currentConversationRef.current
      if (latest?.id !== current.id || sequence !== refreshSequence.current) return
    }
    if (updated.revision < latest.revision) return
    // Profile edits must not replace loaded older history or in-flight messages.
    applyConversationIfCurrent(current.id, { ...latest, title: updated.title,
      revision: updated.revision,
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
  const closeDialog = useCallback(() => {
    setEditorMode(null)
    pendingTaskPrompt.current = null
    abandonAssistantEdit()
  }, [])
  const startTaskChat = useCallback(async (prompt: string) => {
    const current = currentConversationRef.current
    if (current && botAssistantId(current.id)) {
      const creation = navigation.beginConversationCreation()
      if (!navigation.commitCreatedConversation(creation, current)) return
      updateComposerDraft(draftKey(current.id), draft => ({ ...draft, input: prompt }))
    } else {
      beginNewBot()
      pendingTaskPrompt.current = prompt
    }
  }, [beginNewBot, currentConversationRef, navigation])
  const editCurrentBot = useCallback(() => {
    const current = currentConversationRef.current
    const assistantId = botContactId(current?.id)
    if (!assistantId || !current) return
    void chatApi.getAssistants().then(assistants => {
      if (currentConversationRef.current?.id !== current.id) return
      const assistant = assistants.find(item => item.id === assistantId)
      if (assistant) editBot(assistant)
    }).catch(err => setStreamErrorForConversation(current.id, String(err)))
  }, [currentConversationRef, editBot, setStreamErrorForConversation])
  const startAssistant = useCallback((assistant: ChatAssistant) => {
    if (isBotWorkspace()) void openBot(assistant).catch(() => { /* openBot exposes the failure in chat. */ })
    else void startAssistantChat(assistant)
  }, [openBot, startAssistantChat])
  return { editorMode, editedAssistantId, beginNewBot, editBot, finishEdit, closeDialog, editCurrentBot, startAssistant, startTaskChat }
}
