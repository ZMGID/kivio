import { memo, useEffect, useImperativeHandle, useRef, forwardRef, type CSSProperties } from 'react'
import {
  BLOB_BLUE,
  BLOB_EYE,
  BLOB_REST,
  KivioBlobSim,
  blobScheduleMs,
  type BlobMood,
  type BlobIdentity,
} from './kivioBlobSim'
import type { BotAccessory } from './botAvatarIdentity'
import { prefersReducedMotion } from './utils'

export type { BlobMood }

export interface KivioBlobHandle {
  pokeAt: (clientX?: number) => void
}

interface KivioBlobProps {
  identity?: BlobIdentity
  accessory?: BotAccessory
  interactive?: boolean
  size?: number
  mood?: BlobMood
  /** 切会话覆盖层盖住时停表，避免和点阵脉冲叠两套动画。 */
  paused?: boolean
  /** 变化时眨一眼 + 瞟一下（空态换文案）。连点升温不走这条。 */
  pulse?: string | number | null
  onPoke?: (streak: number) => void
}

function writeAttr(el: Element, name: string, value: string, prev: string): string {
  if (prev === value) return prev
  el.setAttribute(name, value)
  return value
}

export const KivioBlob = memo(forwardRef<KivioBlobHandle, KivioBlobProps>(function KivioBlob(
  { size = 28, mood = 'idle', paused = false, pulse, onPoke, identity, accessory = 'none', interactive = true }: KivioBlobProps,
  ref,
) {
  const hostRef = useRef<HTMLSpanElement>(null)
  const rigRef = useRef<SVGGElement>(null)
  const bodyRef = useRef<SVGPathElement>(null)
  const eye0Ref = useRef<SVGPathElement>(null)
  const eye1Ref = useRef<SVGPathElement>(null)
  const simRef = useRef<KivioBlobSim | null>(null)
  const armRef = useRef<(now?: number) => void>(() => {})
  const pausedRef = useRef(paused)
  const pulseSkipRef = useRef(true)
  const onPokeRef = useRef(onPoke)
  onPokeRef.current = onPoke
  pausedRef.current = paused
  if (!simRef.current) {
    simRef.current = new KivioBlobSim({ reducedMotion: prefersReducedMotion(), identity })
  }

  const pokeAt = (clientX?: number) => {
    const sim = simRef.current
    const host = hostRef.current
    if (!sim) return
    let lookX: number | undefined
    if (clientX != null && host) {
      const rect = host.getBoundingClientRect()
      if (rect.width > 0) lookX = ((clientX - rect.left) / rect.width) * 2 - 1
    }
    const streak = sim.poke(performance.now(), lookX)
    onPokeRef.current?.(streak)
    armRef.current()
  }
  const pokeAtRef = useRef(pokeAt)
  pokeAtRef.current = pokeAt
  useImperativeHandle(ref, () => ({ pokeAt: (clientX) => pokeAtRef.current(clientX) }), [])

  useEffect(() => {
    simRef.current?.setIdentity(identity, performance.now())
    armRef.current()
  }, [identity])

  useEffect(() => {
    simRef.current?.setMood(mood, performance.now())
    armRef.current()
  }, [mood])

  useEffect(() => {
    if (pulseSkipRef.current) {
      pulseSkipRef.current = false
      return
    }
    simRef.current?.nudge(performance.now())
    armRef.current()
  }, [pulse])

  useEffect(() => {
    armRef.current()
  }, [paused])

  useEffect(() => {
    const sim = simRef.current
    const host = hostRef.current
    const rig = rigRef.current
    const body = bodyRef.current
    const eye0 = eye0Ref.current
    const eye1 = eye1Ref.current
    if (!sim || !host || !rig || !body || !eye0 || !eye1) return

    const last = { rig: '', body: '', bodyD: '', fill: '', d0: '', t0: '', d1: '', t1: '' }
    const apply = (now: number) => {
      const p = sim.sample(now)
      last.rig = writeAttr(rig, 'transform', p.rig, last.rig)
      last.body = writeAttr(body, 'transform', p.body, last.body)
      last.bodyD = writeAttr(body, 'd', p.bodyD, last.bodyD)
      last.fill = writeAttr(body, 'fill', p.fill, last.fill)
      last.d0 = writeAttr(eye0, 'd', p.eyes[0].d, last.d0)
      last.t0 = writeAttr(eye0, 'transform', p.eyes[0].transform, last.t0)
      last.d1 = writeAttr(eye1, 'd', p.eyes[1].d, last.d1)
      last.t1 = writeAttr(eye1, 'transform', p.eyes[1].transform, last.t1)
    }

    apply(performance.now())
    if (prefersReducedMotion()) {
      armRef.current = (now = performance.now()) => apply(now)
      return () => { armRef.current = () => {} }
    }

    let raf = 0
    let timeout = 0
    let onScreen = true
    let unfocused = false
    let stopped = false

    const clearTimers = () => {
      if (raf) cancelAnimationFrame(raf)
      if (timeout) clearTimeout(timeout)
      raf = 0
      timeout = 0
    }

    const arm = (now = performance.now()) => {
      clearTimers()
      if (stopped) return
      const delay = blobScheduleMs({
        reducedMotion: false,
        hidden: document.hidden,
        onScreen,
        unfocused,
        covered: pausedRef.current,
        highFps: sim.wantsHighFps(now),
        idleWakeMs: sim.nextIdleWakeMs(now),
      })
      if (delay == null) { apply(now); return }
      const paint = (t: number) => {
        raf = 0
        apply(t)
        arm(t)
      }
      if (delay === 0) {
        raf = requestAnimationFrame(paint)
      } else {
        timeout = window.setTimeout(() => {
          timeout = 0
          raf = requestAnimationFrame(paint)
        }, delay)
      }
    }

    const onVis = () => {
      if (document.hidden) clearTimers()
      else arm()
    }
    const onBlur = () => {
      unfocused = true
      clearTimers()
    }
    const onFocus = () => {
      unfocused = false
      arm()
    }
    document.addEventListener('visibilitychange', onVis)
    window.addEventListener('blur', onBlur)
    window.addEventListener('focus', onFocus)

    const io = new IntersectionObserver((entries) => {
      onScreen = entries.some((entry) => entry.isIntersecting)
      if (onScreen) arm()
      else clearTimers()
    })
    armRef.current = arm
    io.observe(host)
    arm()

    return () => {
      stopped = true
      armRef.current = () => {}
      clearTimers()
      document.removeEventListener('visibilitychange', onVis)
      window.removeEventListener('blur', onBlur)
      window.removeEventListener('focus', onFocus)
      io.disconnect()
    }
  }, [])

  return (
    <span
      ref={hostRef}
      role={interactive ? 'button' : undefined}
      tabIndex={interactive ? 0 : undefined}
      aria-hidden={interactive ? undefined : true}
      className={`inline-flex shrink-0 select-none ${interactive ? 'cursor-pointer' : ''}`}
      style={{ '--kv-stream-logo-size': `${size}px`, width: size, height: size } as CSSProperties}
      onPointerDown={(event) => {
        if (!interactive || event.button !== 0) return
        event.preventDefault()
        pokeAt(event.clientX)
      }}
      onKeyDown={(event) => {
        if (!interactive) return
        if (event.key !== 'Enter' && event.key !== ' ') return
        event.preventDefault()
        pokeAt()
      }}
    >
      <svg
        className="kv-stream-logo"
        style={identity ? { width: size, height: size } : undefined}
        viewBox={identity ? '8 0 224 240' : '0 0 240 240'}
        width={size}
        height={size}
        overflow="visible"
      >
        <g ref={rigRef} transform={BLOB_REST.rig}>
          <path ref={bodyRef} d={BLOB_REST.bodyD} fill={BLOB_BLUE} transform={BLOB_REST.body} />
          <path ref={eye0Ref} fill={BLOB_EYE} d={BLOB_REST.eyes[0].d} transform={BLOB_REST.eyes[0].transform} />
          <path ref={eye1Ref} fill={BLOB_EYE} d={BLOB_REST.eyes[1].d} transform={BLOB_REST.eyes[1].transform} />
          {accessory === 'glasses' && <g fill="none" stroke="#292C35" strokeWidth="6" strokeLinecap="round" strokeLinejoin="round">
            <rect x="78" y="96" width="36" height="36" rx="11" /><rect x="126" y="96" width="36" height="36" rx="11" />
            <path d="M114 110 Q120 106 126 110 M69 103 L78 106 M162 106 L171 103" />
          </g>}
          {accessory === 'headphones' && <g fill="#292C35" stroke="#292C35" strokeWidth="7" strokeLinecap="round">
            <path d="M51 117 V100 C51 17 189 17 189 100 V117" fill="none" />
            <rect x="45" y="102" width="16" height="38" rx="7" /><rect x="179" y="102" width="16" height="38" rx="7" />
            <path d="M188 138 Q185 156 159 155" fill="none" />
          </g>}
          {accessory === 'sprout' && <g stroke="#315D3E" strokeWidth="4" strokeLinecap="round" strokeLinejoin="round" fill="#86B679">
            <path d="M120 48 Q120 31 134 23" fill="none" /><path d="M122 39 Q99 41 99 19 Q121 18 122 39Z" />
            <path d="M124 34 Q124 12 146 16 Q145 36 124 34Z" />
          </g>}
        </g>
      </svg>
    </span>
  )
}))
