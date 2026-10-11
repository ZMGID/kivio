import { expect, it } from 'vitest'
import { decodeBotAvatar, encodeBotAvatar } from './botAvatarIdentity'

it('round trips a full combination within the legacy eight-character storage limit', () => {
  const identity = { body: 'bean', face: 'flat', accessory: 'headphones' } as const
  const saved = encodeBotAvatar(identity)
  expect(saved).toBe('kv1:7g2')
  expect(saved.length).toBeLessThanOrEqual(8)
  expect(decodeBotAvatar(saved)).toEqual(identity)
})

it.each(['bot', 'lucide', 'kv2:000', 'kv1:8g3', 'kv1:0h0', 'kv1:004', 'kv1:000junk'])('safely displays old or unknown icons: %s', icon => {
  expect(decodeBotAvatar(icon)).toEqual({ body: 'circle', face: 'neutral', accessory: 'none' })
})
