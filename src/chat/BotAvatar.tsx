import { useEffect, useMemo, useRef, useState } from 'react'
import { useT } from '../components/i18n'
import { KivioBlob, type BlobMood } from './KivioBlob'
import { decodeBotAvatar } from './botAvatarIdentity'

/** Identity and presence share the existing mascot renderer and visibility scheduler. */
export function BotAvatar({ color = '#6A8FBD', icon, size = 40, mood = 'idle', label, animated = true, successKey }: {
  color?: string; icon?: string | null; size?: number; mood?: BlobMood; label?: string; animated?: boolean; successKey?: string
}) {
  const t = useT()
  const previous = useRef({ mood, successKey })
  const pendingSuccess = useRef<string[]>([])
  const [done, setDone] = useState(false)
  useEffect(() => {
    const before = previous.current
    previous.current = { mood, successKey }
    setDone(false)
    const successes = successKey?.split('|').filter(Boolean) ?? []
    const previousSuccesses = before.successKey?.split('|').filter(Boolean) ?? []
    const newSuccesses = successes.filter(key => !previousSuccesses.includes(key))
    const wasWorking = ['work', 'think', 'search', 'speak'].includes(before.mood)
    const working = ['work', 'think', 'search', 'speak'].includes(mood)
    if (newSuccesses.length && (working || wasWorking)) pendingSuccess.current = newSuccesses
    if (mood !== 'idle') {
      if (!working) pendingSuccess.current = []
      return
    }
    const celebrate = pendingSuccess.current.some(key => successes.includes(key))
    pendingSuccess.current = []
    if (!celebrate) return
    setDone(true)
    const timer = window.setTimeout(() => setDone(false), 2600)
    return () => window.clearTimeout(timer)
  }, [mood, successKey])
  const appearance = useMemo(() => decodeBotAvatar(icon), [icon])
  const identity = useMemo(() => ({ body: appearance.body, face: appearance.face, color }), [appearance, color])
  const effectiveMood = done && mood === 'idle' ? 'done' : mood
  const accessibleLabel = label && effectiveMood !== 'done' ? label : effectiveMood !== 'idle' ? t.chatBotAvatarMoods.split('|')[({ idle: 0, think: 1, search: 2, work: 3, speak: 4, wait: 5, error: 6, done: 7 })[effectiveMood]] : label
  return <span className="inline-flex shrink-0" role={accessibleLabel ? 'img' : undefined} aria-label={accessibleLabel} title={accessibleLabel} aria-hidden={accessibleLabel ? undefined : true}>
    <KivioBlob size={size} mood={effectiveMood} identity={identity} accessory={appearance.accessory} interactive={false} paused={!animated} />
  </span>
}
