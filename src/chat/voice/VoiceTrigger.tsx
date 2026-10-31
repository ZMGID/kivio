import { Mic } from 'lucide-react'
import { IconButton } from '../../components/Button'
import { i18n, type Lang } from '../../components/i18n'
import { VoiceOrb } from './VoiceOrb'
import { useVoiceCallContext, voiceOrbState } from './useVoiceCall'

/** Sidebar footer trigger: a microphone while idle, the live orb during a call (toggles the stage). */
export function VoiceTrigger({ lang }: { lang: Lang }) {
  const call = useVoiceCallContext()
  if (!call) return null
  const lastError = call.lastError
  const t = i18n[lang]
  const live = call.state === 'connecting' || call.state === 'active' || call.state === 'closing'

  return (
    <span className="relative inline-flex">
      <IconButton
        size="xs"
        label={live ? (call.stageOpen ? t.voiceHideStage : t.voiceOpenStage) : t.voiceStartHint}
        title={live ? undefined : lastError ?? t.voiceStartHint}
        aria-pressed={live ? call.stageOpen : undefined}
        onClick={live ? () => call.setStageOpen(!call.stageOpen) : call.start}
      >
        {live
          ? <VoiceOrb state={voiceOrbState(call.state, call.muted, call.working)} levelsRef={call.levelsRef} size={18} />
          : <Mic strokeWidth={1.75} />}
      </IconButton>
      {!live && lastError && <span className="kv-voice-failure-dot" aria-hidden="true" />}
    </span>
  )
}

/** Above the orchestrator conversation's composer while its call is live; pressing it returns to the stage. */
export function VoiceComposerOrb({ lang, conversationId }: { lang: Lang; conversationId: string | null }) {
  const call = useVoiceCallContext()
  if (!call || !conversationId || call.conversationId !== conversationId) return null
  if (call.state !== 'active' && call.state !== 'connecting') return null
  return (
    <div className="flex shrink-0 justify-center pb-1">
      <IconButton size="lg" shape="circle" variant="ghost" label={i18n[lang].voiceOpenStage} onClick={() => call.setStageOpen(true)}>
        <VoiceOrb state={voiceOrbState(call.state, call.muted, call.working)} levelsRef={call.levelsRef} size={32} />
      </IconButton>
    </div>
  )
}
