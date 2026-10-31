import { useEffect, useRef, type MutableRefObject } from 'react'
import { useDocumentDark } from '../../theme/useDocumentDark'
import { paintOrb, ORB_STATES, voiceEnergy, type OrbState } from './orb/engine'

export type { OrbState } from './orb/engine'

/** Continuous voice motion lives on the canvas, never in React's render loop. */
export function VoiceOrb({ state, levelsRef, size }: {
  state: OrbState
  levelsRef: MutableRefObject<{ mic: number; speaker: number }>
  size: number
}) {
  const dark = useDocumentDark()
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const stateRef = useRef(state)
  stateRef.current = state

  useEffect(() => {
    const canvas = canvasRef.current
    const ctx = canvas?.getContext('2d')
    if (!canvas || !ctx) return
    const dpr = Math.min(window.devicePixelRatio || 1, 2)
    canvas.width = Math.round(size * dpr)
    canvas.height = Math.round(size * dpr)
    const motion = window.matchMedia?.('(prefers-reduced-motion: reduce)')
    let reduced = motion?.matches ?? false
    const shape = { ...ORB_STATES[stateRef.current] }
    let phase = 0.8
    let energy = 0
    let last = performance.now()
    let drawn = 0
    let lastLoud = -Infinity
    let shown = stateRef.current
    let shownSpeaking = false
    let frame = 0
    let poll = 0
    let paletteAt = -Infinity
    let ink = ''

    const isSpeaking = (now: number) => {
      if (levelsRef.current.speaker > 0.018) lastLoud = now
      return now - lastLoud < 240 && stateRef.current !== 'connecting'
    }

    const refreshInk = (now: number) => {
      if (now - paletteAt <= 1000 && ink) return false
      const next = getComputedStyle(canvas).color || 'currentColor'
      const changed = next !== ink
      ink = next
      paletteAt = now
      return changed
    }

    const render = (now: number) => {
      const dt = Math.min(0.08, Math.max(0, (now - last) / 1000))
      last = now
      const levels = levelsRef.current
      const speaking = isSpeaking(now)
      const next = speaking ? 'composing' : stateRef.current
      const target = ORB_STATES[next]
      const blend = reduced ? 1 : 1 - Math.exp(-dt / 0.3)
      shape.size += (target.size - shape.size) * blend
      shape.stretch += (target.stretch - shape.stretch) * blend
      shape.ripple += (target.ripple - shape.ripple) * blend
      shape.twist += (target.twist - shape.twist) * blend
      shape.speed += (target.speed - shape.speed) * blend
      shape.opacity += (target.opacity - shape.opacity) * blend
      const peak = stateRef.current === 'connecting' || (!speaking && stateRef.current === 'breathing')
        ? 0 : voiceEnergy(speaking ? levels.speaker : levels.mic)
      energy += (peak - energy) * (1 - Math.exp(-dt / (peak > energy ? 0.06 : 0.24)))
      if (!reduced) phase += dt * shape.speed
      shown = stateRef.current
      shownSpeaking = speaking
      refreshInk(now)
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
      ctx.clearRect(0, 0, size, size)
      paintOrb(ctx, size, phase, reduced ? 0 : energy, shape, ink)
    }
    const loop = (now: number) => {
      // 30 fps is enough for this slow surface and leaves the WebView room for streamed captions.
      if (now - drawn >= 1000 / 30) { render(now); drawn = now }
      frame = requestAnimationFrame(loop)
    }
    const restart = () => {
      cancelAnimationFrame(frame)
      window.clearInterval(poll)
      if (document.hidden) return
      last = performance.now()
      if (reduced) {
        render(last)
        poll = window.setInterval(() => {
          const now = performance.now()
          if (stateRef.current !== shown || isSpeaking(now) !== shownSpeaking || refreshInk(now)) render(now)
        }, 250)
      } else {
        frame = requestAnimationFrame(loop)
      }
    }
    const onMotion = (event: MediaQueryListEvent) => { reduced = event.matches; restart() }
    restart()
    motion?.addEventListener('change', onMotion)
    document.addEventListener('visibilitychange', restart)
    return () => {
      cancelAnimationFrame(frame)
      window.clearInterval(poll)
      motion?.removeEventListener('change', onMotion)
      document.removeEventListener('visibilitychange', restart)
    }
  }, [dark, levelsRef, size])

  return <canvas ref={canvasRef} aria-hidden="true" className="kv-voice-orb" style={{ width: size, height: size }} />
}
