import { useState } from 'react'
import { Button } from '../components/Button'
import { useT } from '../components/i18n'
import { Label, Select } from '../settings/public/controls'
import { BotAvatar } from './BotAvatar'
import { BOT_ACCESSORIES, BOT_AVATAR_PRESETS, decodeBotAvatar, encodeBotAvatar, type BotAvatarIdentity } from './botAvatarIdentity'
import { BODY_SHAPES, FACE_NAMES } from './kivioBlobShapes'
import type { BlobMood } from './KivioBlob'

export function BotAvatarPicker({ icon, color, disabled, onChange }: {
  icon?: string; color?: string; disabled: boolean
  onChange: (patch: { icon: string; color?: string }) => void
}) {
  const t = useT()
  const [mood, setMood] = useState<BlobMood>('idle')
  const identity = decodeBotAvatar(icon)
  const update = (patch: Partial<BotAvatarIdentity>) => onChange({ icon: encodeBotAvatar({ ...identity, ...patch }) })
  const bodies = t.chatBotAvatarBodies.split('|')
  const faces = t.chatBotAvatarFaces.split('|')
  const accessories = t.chatBotAvatarAccessories.split('|')
  const moods: BlobMood[] = ['idle', 'think', 'search', 'work', 'speak', 'wait', 'error', 'done']
  const moodLabels = t.chatBotAvatarMoods.split('|')
  return <div className="flex flex-col gap-4">
    <div className="flex items-center gap-4">
      <BotAvatar icon={icon} color={color} size={76} mood={mood} label={moodLabels[moods.indexOf(mood)]} />
      <div className="min-w-0 flex-1 space-y-1.5">
        <Label>{t.chatBotAvatarPreview}</Label>
        <Select size="sm" ariaLabel={t.chatBotAvatarPreview} value={mood} onChange={value => setMood(value as BlobMood)} options={moods.map((value, index) => ({ value, label: moodLabels[index] }))} />
        <p className="text-[12px] leading-5 text-neutral-500">{t.chatBotAvatarHint}</p>
      </div>
    </div>
    <div className="flex flex-wrap gap-2" role="group" aria-label={t.chatBotAvatarPresets}>
      {BOT_AVATAR_PRESETS.map((preset, index) => <Button key={index} size="sm" disabled={disabled} title={`${t.chatBotAvatarPresets} ${index + 1}`}
        aria-label={`${t.chatBotAvatarPresets} ${index + 1}`} aria-pressed={icon === encodeBotAvatar(preset) && color === preset.color}
        variant={icon === encodeBotAvatar(preset) && color === preset.color ? 'default' : 'ghost'}
        onClick={() => onChange({ icon: encodeBotAvatar(preset), color: preset.color })}>
        <BotAvatar icon={encodeBotAvatar(preset)} color={preset.color} size={40} animated={false} />
      </Button>)}
    </div>
    <div className="grid grid-cols-2 gap-3">
      <div className="flex flex-col gap-1.5"><Label>{t.chatBotAvatarBody}</Label><Select ariaLabel={t.chatBotAvatarBody} disabled={disabled} value={identity.body}
        onChange={value => update({ body: value as BotAvatarIdentity['body'] })} options={BODY_SHAPES.map((value, index) => ({ value, label: bodies[index] }))} /></div>
      <div className="flex flex-col gap-1.5"><Label>{t.chatBotAvatarEyes}</Label><Select ariaLabel={t.chatBotAvatarEyes} disabled={disabled} value={identity.face}
        onChange={value => update({ face: value as BotAvatarIdentity['face'] })} options={FACE_NAMES.map((value, index) => ({ value, label: faces[index] }))} /></div>
      <div className="col-span-2 flex flex-col gap-1.5"><Label>{t.chatBotAvatarAccessory}</Label><Select ariaLabel={t.chatBotAvatarAccessory} disabled={disabled} value={identity.accessory}
        onChange={value => update({ accessory: value as BotAvatarIdentity['accessory'] })} options={BOT_ACCESSORIES.map((value, index) => ({ value, label: accessories[index] }))} /></div>
    </div>
  </div>
}
