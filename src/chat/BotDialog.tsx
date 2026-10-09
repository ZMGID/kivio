import { useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { Check, ChevronRight, SlidersHorizontal, X } from 'lucide-react'
import { Button, IconButton } from '../components/Button'
import { useT } from '../components/i18n'
import { Input, Label, TextArea } from '../settings/public/controls'
import { useWindowStore } from '../utils/windowStore'
import { assistantDraftStore, saveAssistantDraft, updateAssistantDraft } from './assistantDraftStore'
import type { ChatAssistant } from './types'
import { assistantColors } from './assistantColors'
import { BotAvatar } from './BotAvatar'

/** Uses the existing assistant edit owner; retries keep the same saved identity. */
export function BotDialog({ onSaved, onClose, onAdvanced }: {
  onSaved: (assistant: ChatAssistant, created: boolean) => Promise<void>
  onClose: () => void
  onAdvanced: () => void
}) {
  const t = useT()
  const [state] = useWindowStore(assistantDraftStore)
  const created = useRef(!state.persisted).current
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const alive = useRef(true)
  const dialog = useRef<HTMLDialogElement>(null)
  useEffect(() => {
    alive.current = true
    dialog.current?.showModal()
    const name = dialog.current?.querySelector<HTMLInputElement>('input')
    name?.focus()
    name?.select()
    return () => { alive.current = false }
  }, [])
  const draft = state.draft
  if (!draft) return null
  const save = async () => {
    if (busy) return
    setBusy(true)
    setError('')
    try {
      const saved = await saveAssistantDraft(t.chatAssistantNameRequired)
      if (!saved || !alive.current) return
      await onSaved(saved, created)
      if (alive.current) onClose()
    } catch (err) {
      if (alive.current) setError(err instanceof Error ? err.message : String(err))
    } finally {
      if (alive.current) setBusy(false)
    }
  }
  return createPortal(
    <dialog ref={dialog} className="kv-modal kv-modal--wide custom-scrollbar" aria-label={created ? t.chatBotNew : t.chatBotDetails}
      onCancel={event => { event.preventDefault(); if (!busy) onClose() }}>
      <form className="flex flex-col gap-5" onSubmit={event => { event.preventDefault(); void save() }}>
        <div className="flex items-start justify-between gap-3">
          <div>
            <h2 className="text-[18px] font-semibold leading-6">{created ? t.chatBotNew : t.chatBotDetails}</h2>
            <p className="mt-1 text-[13px] leading-5 text-neutral-500">{created ? t.chatBotSetupHint : t.chatBotEditHint}</p>
          </div>
          <IconButton size="xs" label={t.chatWinClose} disabled={busy} onClick={onClose}><X size={16} /></IconButton>
        </div>
        <div className="flex items-center gap-4">
          <BotAvatar color={draft.color} size={54} />
          <label className="flex min-w-0 flex-1 flex-col gap-1.5">
            <Label>{t.chatBotName}</Label>
            <Input autoFocus value={draft.name} disabled={busy} onChange={name => updateAssistantDraft(d => ({ ...d, name }))} />
          </label>
        </div>
        <div className="flex flex-wrap items-center gap-3" role="group" aria-label={t.chatAssistantColor}>
          <span className="text-[12px] text-neutral-500">{t.chatAssistantColor}</span>
          <div className="flex gap-2">
            {assistantColors.map((color, index) => <button key={color} type="button" disabled={busy}
              aria-label={`${t.chatAssistantColor} ${index + 1}`} aria-pressed={draft.color === color}
              className="flex h-7 w-7 items-center justify-center rounded-full transition-opacity hover:opacity-80 disabled:opacity-50"
              style={{ backgroundColor: color }} onClick={() => updateAssistantDraft(d => ({ ...d, color }))}>
              {draft.color === color && <Check size={14} strokeWidth={2.5} color="white" />}
            </button>)}
          </div>
        </div>
        <label className="flex flex-col gap-1.5">
          <Label>{t.chatBotRole}</Label>
          <TextArea rows={4} value={draft.system_prompt ?? draft.systemPrompt ?? ''} disabled={busy}
            placeholder={t.chatBotRoleHint} onChange={system_prompt => updateAssistantDraft(d => ({ ...d, system_prompt }))} />
        </label>
        <label className="flex flex-col gap-1.5">
          <Label>{t.chatBotDescription}</Label>
          <Input value={draft.description ?? ''} disabled={busy} onChange={description => updateAssistantDraft(d => ({ ...d, description }))} />
        </label>
        {(error || state.error) && <p className="text-xs text-red-600" role="alert">{error || state.error}</p>}
        {!created && <div className="border-t border-neutral-200/80 pt-3">
          <Button variant="ghost" disabled={busy} onClick={onAdvanced}><SlidersHorizontal size={14} />{t.chatBotAdvanced}<ChevronRight size={13} /></Button>
        </div>}
        <div className="flex items-center justify-end gap-2 border-t border-neutral-200/80 pt-4">
          <Button disabled={busy} onClick={onClose}>{t.cancel}</Button>
          <Button type="submit" variant="primary" disabled={busy || !draft.name.trim()}>{busy ? t.chatLoading : created ? t.chatBotCreate : t.save}</Button>
        </div>
      </form>
    </dialog>, document.body,
  )
}
