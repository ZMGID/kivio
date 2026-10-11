import { useCallback, useEffect, useId, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { Check, Palette, X } from 'lucide-react'
import { Button, IconButton } from '../components/Button'
import { confirmDialog } from '../components/dialogQueue'
import { useT } from '../components/i18n'
import { Input, Label, TextArea } from '../settings/public/controls'
import { useWindowStore } from '../utils/windowStore'
import { abandonAssistantEdit, adoptFetchedAssistant, assistantDraftStore, beginAssistantEdit, noteAssistantPersisted, saveAssistantDraft, updateAssistantDraft } from './assistantDraftStore'
import type { ChatAssistant } from './types'
import { assistantColors } from './assistantColors'
import { BotAvatar } from './BotAvatar'
import { BotAvatarPicker } from './BotAvatarPicker'
import { BotRuntimePanel, type BotThinkingSettings } from './bot/BotRuntimePanel'
import { BotWorkDetails } from './bot/BotWorkDetails'
import { assistantFromProfile, isProfileConflict, profileApi, type BotPersonaWrite } from './bot/profile'
import { isTauriRuntime } from './utils'

function personaFromAssistant(assistant: ChatAssistant): BotPersonaWrite {
  return {
    name: assistant.name.trim(),
    description: (assistant.description ?? '').trim(),
    systemPrompt: (assistant.system_prompt ?? assistant.systemPrompt ?? '').trim(),
    icon: assistant.icon?.trim() || 'bot',
    color: assistant.color?.trim() || '#6A8FBD',
  }
}

type HeldProfile = {
  revision: number
  persona: BotPersonaWrite
  session: number
  /** Draft revision when this profile snapshot was applied. Later edits stay on this token. */
  draftRevision: number
}

/** Creation uses a modal; an existing Bot is edited alongside its continuing chat. */
export function BotProfileEditor({ onSaved, onClose, thinkingSettings, registerLeaveGuard, conversationId, onOpenArtifacts }: {
  onSaved: (assistant: ChatAssistant, created: boolean) => Promise<void>
  onClose: () => void
  thinkingSettings?: BotThinkingSettings
  registerLeaveGuard?: (guard: (() => Promise<boolean>) | null) => void
  conversationId?: string | null
  onOpenArtifacts?: (conversationId: string) => void
}) {
  const t = useT()
  const tabsId = useId()
  const [state] = useWindowStore(assistantDraftStore)
  const created = useRef(!state.persisted).current
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [colorsOpen, setColorsOpen] = useState(false)
  const [tab, setTab] = useState<'details' | 'artifacts' | 'settings'>('details')
  const editing = tab === 'settings'
  const [settingsPage, setSettingsPage] = useState<'profile' | 'runtime' | 'tools' | 'memory'>('profile')
  const [makePrimary, setMakePrimary] = useState(false)
  const [runtimeBusy, setRuntimeBusy] = useState(false)
  const createdId = useRef<string | null>(null)
  const heldProfile = useRef<HeldProfile | null>(null)
  const revisionLoad = useRef<Promise<unknown> | null>(null)
  const pendingOpen = useRef<{ assistant: ChatAssistant; session: number; revision: number; profileRevision: number } | null>(null)
  const runtimeGuard = useRef({ dirty: false, busy: false })
  const [conflict, setConflict] = useState(false)
  const savedProfile = useRef(state.draft)
  const alive = useRef(true)
  const dialog = useRef<HTMLDialogElement>(null)
  const panel = useRef<HTMLElement>(null)
  const dirty = state.revision !== state.acknowledgedRevision
  const syncRuntime = useCallback((activity: { dirty: boolean; busy: boolean }) => {
    runtimeGuard.current = activity
    setRuntimeBusy((current) => current === activity.busy ? current : activity.busy)
  }, [])
  const requestClose = useCallback(async () => {
    const blocked = () => busy || state.status === 'saving' || runtimeGuard.current.busy
    if (blocked()) return false
    if ((dirty || runtimeGuard.current.dirty) && !await confirmDialog({ message: t.chatBotDiscardConfirm, confirmLabel: t.discardAndClose })) return false
    if (blocked()) return false
    onClose()
    return true
  }, [busy, dirty, onClose, state.status, t])
  useEffect(() => {
    registerLeaveGuard?.(requestClose)
    return () => registerLeaveGuard?.(null)
  }, [registerLeaveGuard, requestClose])
  useEffect(() => {
    alive.current = true
    if (created) {
      dialog.current?.showModal()
      dialog.current?.querySelector<HTMLInputElement>('input')?.select()
    } else panel.current?.focus()
    return () => { alive.current = false }
  }, [created])
  const openedId = state.draft?.id
  useEffect(() => {
    if (created || !isTauriRuntime() || !openedId) return
    let current = true
    heldProfile.current = null
    const snap = assistantDraftStore.getSnapshot()
    const fence = { session: snap.session, revision: snap.revision }
    const pending = profileApi.get(openedId).then((detail) => {
      if (!current) return null
      const live = assistantDraftStore.getSnapshot()
      if (live.session !== fence.session) return null
      const assistant = assistantFromProfile(detail)
      if (live.revision !== fence.revision || !adoptFetchedAssistant(assistant, fence)) {
        setConflict(true)
        return null
      }
      savedProfile.current = assistant
      const persona = personaFromAssistant(assistant)
      heldProfile.current = {
        revision: detail.profile.revision,
        persona,
        session: fence.session,
        draftRevision: fence.revision,
      }
      return detail.profile.revision
    })
    revisionLoad.current = pending
    pending.catch((err) => {
      if (current) setError(err instanceof Error ? err.message : String(err))
    })
    return () => { current = false }
  }, [created, openedId])
  const switchTab = async (nextTab: typeof tab) => {
    if (nextTab === tab) return true
    if (!editing) { setTab(nextTab); return true }
    if (busy || state.status === 'saving' || runtimeGuard.current.busy) return false
    if ((dirty || runtimeGuard.current.dirty) && !await confirmDialog({ message: t.chatBotDiscardConfirm, confirmLabel: t.discardAndClose })) return false
    if (!alive.current || assistantDraftStore.getSnapshot().status === 'saving' || runtimeGuard.current.busy) return false
    if (savedProfile.current) {
      abandonAssistantEdit()
      beginAssistantEdit(savedProfile.current, true)
      const next = assistantDraftStore.getSnapshot()
      const revision = heldProfile.current?.revision
      if (revision != null) {
        heldProfile.current = {
          revision,
          persona: personaFromAssistant(savedProfile.current),
          session: next.session,
          draftRevision: next.revision,
        }
      }
    }
    pendingOpen.current = null
    setError('')
    setColorsOpen(false)
    setTab(nextTab)
    return true
  }
  const draft = state.draft
  if (!draft) return null
  const finishNavigation = async (saved: ChatAssistant, profileRevision: number) => {
    await onSaved(saved, created)
    if (!alive.current) return
    pendingOpen.current = null
    setConflict(false)
    abandonAssistantEdit()
    if (created) {
      onClose()
      return
    }
    beginAssistantEdit(saved, true)
    const reopened = assistantDraftStore.getSnapshot()
    heldProfile.current = {
      revision: profileRevision,
      persona: personaFromAssistant(saved),
      session: reopened.session,
      draftRevision: reopened.revision,
    }
    setColorsOpen(false)
  }
  const save = async () => {
    if (busy || runtimeGuard.current.busy) return
    setBusy(true)
    setError('')
    try {
      if (isTauriRuntime()) {
        const snap = assistantDraftStore.getSnapshot()
        if (!snap.draft) return
        const pending = pendingOpen.current
        if (pending && snap.session === pending.session && snap.revision === pending.revision) {
          await finishNavigation(pending.assistant, pending.profileRevision)
          return
        }
        const writtenRevision = snap.revision
        let persona = personaFromAssistant(snap.draft)
        const updatingId = created ? createdId.current : snap.draft.id
        let detail
        if (!updatingId) {
          if (!persona.name) throw new Error(t.chatAssistantNameRequired)
          detail = await profileApi.create({ ...persona, primary: makePrimary })
        } else {
          if (!heldProfile.current && revisionLoad.current) await revisionLoad.current
          const again = assistantDraftStore.getSnapshot()
          const held = heldProfile.current
          if (!again.draft || !held || again.session !== held.session || again.revision !== writtenRevision) {
            throw new Error(t.chatBotRevisionPending)
          }
          persona = again.revision === held.draftRevision ? held.persona : personaFromAssistant(again.draft)
          if (!persona.name) throw new Error(t.chatAssistantNameRequired)
          detail = await profileApi.updatePersona(updatingId, persona, held.revision)
        }
        if (created) createdId.current = detail.assistantId
        const saved = assistantFromProfile(detail)
        noteAssistantPersisted(saved, writtenRevision)
        const after = assistantDraftStore.getSnapshot()
        heldProfile.current = {
          revision: detail.profile.revision,
          persona: personaFromAssistant(saved),
          session: after.session,
          draftRevision: writtenRevision,
        }
        savedProfile.current = saved
        setConflict(false)
        pendingOpen.current = after.revision === writtenRevision
          ? { assistant: saved, session: after.session, revision: after.revision, profileRevision: detail.profile.revision }
          : null
        if (!alive.current) return
        await finishNavigation(saved, detail.profile.revision)
        return
      }
      const saved = await saveAssistantDraft(t.chatAssistantNameRequired)
      if (!saved || !alive.current) return
      savedProfile.current = saved
      await onSaved(saved, created)
      if (alive.current) {
        if (created) onClose()
        else setColorsOpen(false)
      }
    } catch (err) {
      if (!alive.current) return
      const message = err instanceof Error ? err.message : String(err)
      setConflict(isProfileConflict(err))
      setError(message)
    } finally {
      if (alive.current) setBusy(false)
    }
  }
  const reloadOpened = async () => {
    if (busy || runtimeGuard.current.busy) return
    const id = createdId.current ?? draft.id
    if (!id || !await confirmDialog({ message: t.chatBotReloadConfirm, confirmLabel: t.chatBotReloadProfile })) return
    setBusy(true)
    setError('')
    try {
      const detail = await profileApi.get(id)
      if (!alive.current) return
      const saved = assistantFromProfile(detail)
      savedProfile.current = saved
      pendingOpen.current = null
      abandonAssistantEdit()
      beginAssistantEdit(saved, true)
      const next = assistantDraftStore.getSnapshot()
      heldProfile.current = {
        revision: detail.profile.revision,
        persona: personaFromAssistant(saved),
        session: next.session,
        draftRevision: next.revision,
      }
      setConflict(false)
      setColorsOpen(false)
    } catch (err) {
      if (alive.current) setError(err instanceof Error ? err.message : String(err))
    } finally {
      if (alive.current) setBusy(false)
    }
  }
  const form = <form className="flex min-h-0 flex-1 flex-col" onSubmit={event => { event.preventDefault(); void save() }}>
    {created && <header className="flex shrink-0 items-center justify-between gap-3 pb-5">
      <h2 className="text-[14px] font-semibold text-neutral-900">{t.chatBotNew}</h2>
      <IconButton size="sm" variant="ghost" label={t.chatWinClose} disabled={busy} onClick={() => void requestClose()}><X size={16} /></IconButton>
    </header>}
    <div className={`custom-scrollbar min-h-0 flex-1 overflow-y-auto ${created ? 'pb-5' : 'px-5 py-5'}`}>
      <div className="flex flex-col gap-4">
        <div hidden={!created && settingsPage !== 'profile'}>
        <div className="flex flex-col gap-4">
        {(created || colorsOpen) && <div className="flex flex-col gap-3">
          <BotAvatarPicker icon={draft.icon} color={draft.color} disabled={busy || runtimeBusy} onChange={patch => updateAssistantDraft(d => ({ ...d, ...patch }))} />
          {(created || colorsOpen) && <div className="flex flex-wrap justify-center gap-2" role="group" aria-label={t.chatAssistantColor}>
            {assistantColors.map((color, index) => <button key={color} type="button" disabled={busy}
              aria-label={`${t.chatAssistantColor} ${index + 1}`} aria-pressed={draft.color === color}
              className="flex h-6 w-6 items-center justify-center rounded-full transition-opacity hover:opacity-80 disabled:opacity-50"
              style={{ backgroundColor: color }} onClick={() => updateAssistantDraft(d => ({ ...d, color }))}>
              {draft.color === color && <Check size={13} strokeWidth={2.5} color="white" />}
            </button>)}
          </div>}
        </div>}
        <label className="flex flex-col gap-1.5">
          <Label>{t.chatBotName}</Label>
          <Input autoFocus={created} value={draft.name} disabled={busy} onChange={name => updateAssistantDraft(d => ({ ...d, name }))} />
        </label>
        <label className="flex flex-col gap-1.5">
          <Label>{t.chatBotDescription}</Label>
          <Input value={draft.description ?? ''} disabled={busy} onChange={description => updateAssistantDraft(d => ({ ...d, description }))} />
        </label>
        <label className="flex flex-col gap-1.5">
            <Label>{t.chatBotRole}</Label>
            <TextArea rows={created ? 6 : 4} value={draft.system_prompt ?? draft.systemPrompt ?? ''} disabled={busy}
              placeholder={t.chatBotRoleHint} onChange={system_prompt => updateAssistantDraft(d => ({ ...d, system_prompt }))} />
        </label>
        </div>
        </div>
        {!created && <BotRuntimePanel section={settingsPage} assistantId={draft.id} onActivity={syncRuntime} thinkingSettings={thinkingSettings} />}
        {created && isTauriRuntime() && <label className="flex items-start gap-2 text-[13px] text-neutral-800">
          <input type="checkbox" className="mt-0.5" checked={makePrimary} disabled={busy} onChange={(event) => setMakePrimary(event.target.checked)} />
          <span className="font-medium">{t.chatBotSetPrimary}</span>
        </label>}
      </div>
    </div>
    {(created || dirty || busy || error || state.error || conflict) && <footer className={`flex shrink-0 flex-col gap-3 border-t border-neutral-200/60 ${created ? 'pt-4' : 'px-5 py-3'}`}>
      {(error || state.error) && <p className="text-xs text-red-600" role="alert">{error || state.error}</p>}
      {conflict && <Button size="sm" disabled={busy} onClick={() => void reloadOpened()}>{t.chatBotReloadProfile}</Button>}
      <div className="flex items-center justify-between gap-3">
        <span className="text-[12px] text-neutral-500" role="status">{!created && (busy ? t.chatLoading : dirty ? t.unsavedChanges : state.status === 'saved' ? t.saved : '')}</span>
        <div className="flex items-center gap-2">
          {created && <Button disabled={busy} onClick={() => void requestClose()}>{t.cancel}</Button>}
          <Button type="submit" variant="primary" disabled={busy || runtimeBusy || !draft.name.trim() || (!created && !dirty && !error && !state.error)}>{busy ? t.chatLoading : created ? t.chatBotCreate : t.chatBotSaveProfile}</Button>
        </div>
      </div>
    </footer>}
  </form>
  if (created) return createPortal(<dialog ref={dialog} className="kv-modal kv-modal--wide custom-scrollbar" aria-label={t.chatBotNew}
    onCancel={event => { event.preventDefault(); void requestClose() }}>{form}</dialog>, document.body)
  const tabs = ['details', 'artifacts', 'settings'] as const
  const tabLabels = { details: t.chatBotTabDetails, artifacts: t.chatBotArtifacts, settings: t.chatBotTabSettings }
  return <aside id="bot-settings-sidebar" ref={panel} tabIndex={-1} aria-label={t.chatBotDetails}
    className="chat-bot-settings-sidebar flex min-h-0 shrink-0 flex-col border-l border-neutral-200/60"
    onKeyDown={event => {
      if (event.key === 'Escape' && panel.current?.contains(event.target as Node) && !event.defaultPrevented
        && !panel.current.querySelector('[aria-haspopup="menu"][aria-expanded="true"]')) {
        event.stopPropagation(); void requestClose()
      }
    }}>
    <header className="chat-titlebar-row flex shrink-0 items-center justify-between px-5">
      <h2 className="text-[14px] font-semibold text-neutral-900">{t.chatBotDetails}</h2>
      <IconButton variant="ghost" label={t.chatWinClose} disabled={busy || runtimeBusy} onClick={() => void requestClose()}><X size={16} /></IconButton>
    </header>
    <div className="flex shrink-0 items-start gap-3 px-5 pb-4 pt-3">
      <BotAvatar icon={draft.icon} color={draft.color} size={40} />
      <div className="min-w-0 flex-1">
        <h3 className="break-words text-[15px] font-semibold text-neutral-900">{draft.name}</h3>
        {draft.description && <p className="mt-1 break-words text-[12px] leading-5 text-neutral-500">{draft.description}</p>}
      </div>
      {editing && settingsPage === 'profile' && <IconButton variant="ghost" label={t.chatBotAvatarEdit} aria-expanded={colorsOpen} disabled={busy || runtimeBusy}
        onClick={() => setColorsOpen(open => !open)}><Palette size={15} /></IconButton>}
    </div>
    <div className="shrink-0 border-b border-neutral-200/60 px-5 pb-3">
      <div className="kv-seg" role="tablist" aria-label={t.chatBotDetails}>
        {tabs.map(value => <button key={value} id={`${tabsId}-tab-${value}`} type="button" role="tab"
          className={tab === value ? 'active' : ''}
          aria-selected={tab === value} aria-controls={`${tabsId}-panel`} tabIndex={tab === value ? 0 : -1}
          disabled={busy || runtimeBusy} onClick={() => void switchTab(value)}
          onKeyDown={event => {
            if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return
            event.preventDefault()
            const index = tabs.indexOf(value)
            const next = event.key === 'Home' ? tabs[0] : event.key === 'End' ? tabs[2]
              : tabs[(index + (event.key === 'ArrowRight' ? 1 : 2)) % tabs.length]
            void switchTab(next).then(changed => { if (changed) document.getElementById(`${tabsId}-tab-${next}`)?.focus() })
          }}>{tabLabels[value]}</button>)}
      </div>
    </div>
    {editing && <nav className="flex shrink-0 gap-1 px-5 py-3" aria-label={t.chatBotTabSettings}>
      {(['profile', 'runtime', 'tools', 'memory'] as const).map(value => <Button key={value} size="sm"
        variant={settingsPage === value ? 'default' : 'ghost'} aria-current={settingsPage === value ? 'page' : undefined}
        disabled={busy || runtimeBusy} onClick={() => setSettingsPage(value)}>
        {{profile: t.chatBotSettingsProfile, runtime: t.chatBotSettingsRuntime, tools: t.chatBotSettingsTools, memory: t.chatBotSettingsMemory}[value]}
      </Button>)}
    </nav>}
    <div id={`${tabsId}-panel`} role="tabpanel" tabIndex={0} aria-labelledby={`${tabsId}-tab-${tab}`} className="flex min-h-0 flex-1 flex-col">
      {editing ? form : <div className="custom-scrollbar min-h-0 flex-1 overflow-y-auto px-5 py-5">
        {(error || state.error) && <p className="mb-3 text-xs text-red-600" role="alert">{error || state.error}</p>}
        {conversationId && onOpenArtifacts && <BotWorkDetails conversationId={conversationId} onOpenArtifacts={onOpenArtifacts}
          section={tab === 'artifacts' ? 'artifacts' : 'tasks'} />}
      </div>}
    </div>
  </aside>
}
