import { type BodyShape, type FaceName } from './kivioBlobShapes'

export const BOT_ACCESSORIES = ['none', 'glasses', 'headphones', 'sprout'] as const
export type BotAccessory = typeof BOT_ACCESSORIES[number]
export interface BotAvatarIdentity { body: BodyShape; face: FaceName; accessory: BotAccessory }
export const DEFAULT_BOT_IDENTITY: BotAvatarIdentity = { body: 'circle', face: 'neutral', accessory: 'none' }

// Version 1 codes are permanent; changing the shape/face gallery order cannot change a saved Bot.
const BODY_CODES: Record<BodyShape, string> = { circle: '0', squircle: '1', cloud: '2', bubble: '3', puddle: '4', burst: '5', pebble: '6', bean: '7' }
const FACE_CODES: Record<FaceName, string> = { neutral: '0', dots: '1', wide: '2', tiny: '3', lines: '4', focus: '5', worry: '6', smirk: '7', happy: '8', content: '9', sleepy: 'a', dizzy: 'b', sparkle: 'c', lookUp: 'd', hmm: 'e', peek: 'f', flat: 'g' }

/** Seven characters fit the existing eight-character assistant icon storage contract. */
export function encodeBotAvatar(identity: BotAvatarIdentity): string {
  return `kv1:${BODY_CODES[identity.body]}${FACE_CODES[identity.face]}${BOT_ACCESSORIES.indexOf(identity.accessory).toString(36)}`
}

export function decodeBotAvatar(icon?: string | null): BotAvatarIdentity {
  if (!icon || !/^kv1:[0-7][0-g][0-3]$/.test(icon)) return DEFAULT_BOT_IDENTITY
  return { body: (Object.keys(BODY_CODES) as BodyShape[]).find(body => BODY_CODES[body] === icon[4])!, face: (Object.keys(FACE_CODES) as FaceName[]).find(face => FACE_CODES[face] === icon[5])!, accessory: BOT_ACCESSORIES[parseInt(icon[6], 36)] }
}

export const BOT_AVATAR_PRESETS: readonly (BotAvatarIdentity & { color: string })[] = [
  { body: 'circle', face: 'neutral', accessory: 'none', color: '#6A8FBD' },
  { body: 'squircle', face: 'focus', accessory: 'glasses', color: '#9A7CB8' },
  { body: 'cloud', face: 'dots', accessory: 'sprout', color: '#6E9E80' },
  { body: 'bean', face: 'happy', accessory: 'headphones', color: '#C8886B' },
  { body: 'pebble', face: 'hmm', accessory: 'none', color: '#C39B55' },
  { body: 'bubble', face: 'smirk', accessory: 'glasses', color: '#BA758E' },
]
