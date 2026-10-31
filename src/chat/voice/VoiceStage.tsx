import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { ChevronDown, MessageSquareText, Mic, MicOff, PhoneOff } from 'lucide-react'
import { Button, IconButton } from '../../components/Button'
import { i18n, type Lang } from '../../components/i18n'
import { VoiceCaption } from './VoiceCaption'
import { VoiceOrb } from './VoiceOrb'
import { formatElapsed, useVoiceCallContext, voiceOrbState } from './useVoiceCall'

function useElapsed(since: number | null): string | null {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (since === null) return
    setNow(Date.now())
    const timer = window.setInterval(() => setNow(Date.now()), 1000)
    return () => window.clearInterval(timer)
  }, [since])
  return since === null ? null : formatElapsed((now - since) / 1000)
}

/** The call stays live when the stage is tucked away. */
export function VoiceStage({ lang, sidebarOffset, onOpenConversation }: {
  lang: Lang
  sidebarOffset: string
  onOpenConversation: (conversationId: string) => void
}) {
  const call = useVoiceCallContext()
  const t = i18n[lang]
  const rootRef = useRef<HTMLDivElement>(null)
  const open = Boolean(call?.stageOpen && call.state !== 'idle' && call.state !== 'error')
  const elapsed = useElapsed(call?.activeSince ?? null)

  useLayoutEffect(() => {
    if (!open) return
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null
    rootRef.current?.focus()
    return () => { if (previous?.isConnected) previous.focus() }
  }, [open])

  if (!call || !open) return null
  const status = call.state === 'connecting' ? t.voiceConnecting
    : call.state === 'closing' ? t.voiceClosing
      : call.muted ? t.voiceMuted
        : call.working > 0 ? t.voiceWorking.replace('{count}', String(call.working)) : t.voiceListening
  const channel = call.channel === 'codex' ? 'Codex' : call.channel === 'live' ? 'GPT-Live' : null
  const row = call.transcriptRows[call.transcriptRows.length - 1]

  return (
    <div
      ref={rootRef} role="region" aria-label={t.voiceTitle} tabIndex={-1}
      className="kv-voice-stage" style={{ left: sidebarOffset }}
      onKeyDown={(event) => {
        if (event.key === 'Escape') {
          event.preventDefault()
          event.stopPropagation()
          call.setStageOpen(false)
        } else if (call.state === 'active' && !call.muting && event.key.toLowerCase() === 'm'
          && !event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey
          && !(event.target instanceof HTMLElement && event.target.closest('input, textarea, [contenteditable="true"]'))) {
          event.preventDefault()
          call.toggleMuted()
        }
      }}
    >
      <div className="kv-voice-stage-drag" data-tauri-drag-region />
      <div className="kv-voice-stage-center">
        <div className="kv-voice-stage-art">
          <VoiceOrb state={voiceOrbState(call.state, call.muted, call.working)} levelsRef={call.levelsRef} size={600} />
        </div>
        <VoiceCaption row={row} placeholder={status} label={t.voiceCaptions} />
      </div>

      <div className="kv-voice-call-bar-position">
        <div className="kv-voice-call-bar" role="group" aria-label={t.voiceTitle}>
          <div className="kv-voice-call-clock">
            <span className="kv-voice-call-dot" data-muted={call.muted} title={status} />
            <span className="kv-voice-call-time">{elapsed ?? '–:––'}</span>
            {channel && <span className="kv-voice-call-channel">{channel}</span>}
          </div>
          <span className="kv-voice-call-divider" aria-hidden="true" />
          <IconButton size="lg" shape="circle" variant="ghost" label={t.voiceMuteHint}
            title={call.muted ? t.voiceUnmuteHint : t.voiceMuteHint} aria-pressed={call.muted}
            disabled={call.state !== 'active' || call.muting} onClick={call.toggleMuted}>
            {call.muted ? <MicOff /> : <Mic />}
          </IconButton>
          {call.conversationId && <IconButton size="lg" shape="circle" variant="ghost" label={t.voiceTranscript} onClick={() => {
            call.setStageOpen(false)
            onOpenConversation(call.conversationId!)
          }}><MessageSquareText /></IconButton>}
          <Button variant="danger" onClick={call.hangup} disabled={call.state === 'closing'}><PhoneOff />{t.voiceHangup}</Button>
          <span className="kv-voice-call-divider" aria-hidden="true" />
          <IconButton size="lg" shape="circle" variant="ghost" label={t.voiceBack} onClick={() => call.setStageOpen(false)}><ChevronDown /></IconButton>
        </div>
      </div>
    </div>
  )
}
