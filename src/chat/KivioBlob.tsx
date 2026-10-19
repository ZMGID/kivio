import { memo, useEffect, useLayoutEffect, useId, useImperativeHandle, useRef, forwardRef, type CSSProperties } from 'react'
import {
  BLOB_BLUE,
  BLOB_EYE,
  BLOB_REST,
  KivioBlobSim,
  blobScheduleMs,
  type BlobMood,
} from './kivioBlobSim'
import { prefersReducedMotion } from './utils'

export type { BlobMood }

export interface KivioBlobHandle {
  pokeAt: (clientX?: number) => void
}

interface KivioBlobProps {
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
  { size = 28, mood = 'idle', paused = false, pulse, onPoke }: KivioBlobProps,
  ref,
) {
  const shapeId = useId().replace(/:/g, '')
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
    simRef.current = new KivioBlobSim({ reducedMotion: prefersReducedMotion() })
  }

  const pokeAt = (clientX?: number, clientY?: number) => {
    const sim = simRef.current
    const host = hostRef.current
    if (!sim) return
    let lookX: number | undefined
    let lookY: number | undefined
    if (clientX != null && host) {
      const rect = host.getBoundingClientRect()
      if (rect.width > 0) lookX = ((clientX - rect.left) / rect.width) * 2 - 1
      if (clientY != null && rect.height > 0) lookY = ((clientY - rect.top) / rect.height) * 2 - 1
    }
    const streak = sim.poke(performance.now(), lookX, lookY)
    onPokeRef.current?.(streak)
    armRef.current()
  }
  const pokeAtRef = useRef(pokeAt)
  pokeAtRef.current = pokeAt
  useImperativeHandle(ref, () => ({ pokeAt: (clientX) => pokeAtRef.current(clientX) }), [])

  useLayoutEffect(() => {
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
    if (paused) simRef.current?.releaseLook()
    armRef.current()
  }, [paused])

  useLayoutEffect(() => {
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

    sim.setAmbientMotion(size >= 48, performance.now())
    apply(performance.now())
    const media = window.matchMedia('(prefers-reduced-motion: reduce)')

    // Absolute 60 Hz deadlines cap high-refresh screens without timer drift.
    const frameMs = 1000 / 60
    let nextFrame = performance.now() + frameMs
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
      if (media.matches) {
        apply(now)
        return
      }
      const delay = blobScheduleMs({
        reducedMotion: false,
        hidden: document.hidden,
        onScreen,
        unfocused,
        covered: pausedRef.current,
        highFps: sim.wantsHighFps(now),
        idleWakeMs: sim.nextIdleWakeMs(now),
      })
      if (delay == null) {
        nextFrame = 0
        return
      }
      // Wake promptly after a stopped/hidden clock; never replay missed frames.
      if (!nextFrame || now - nextFrame >= frameMs) nextFrame = now
      const deadline = delay === 0 ? nextFrame : Math.max(nextFrame, now + delay)
      const paint = (t: number) => {
        raf = 0
        if (t + 0.1 < deadline) {
          raf = requestAnimationFrame(paint)
          return
        }
        apply(t)
        nextFrame = deadline + Math.max(1, Math.floor((t - deadline) / frameMs) + 1) * frameMs
        arm(t)
      }
      // Active motion follows vsync; gate expensive sampling to 60 Hz.
      // A timer at a full 60 Hz interval can miss alternate display refreshes.
      const wait = Math.max(0, deadline - now)
      if (delay === 0 || wait < 1) raf = requestAnimationFrame(paint)
      else timeout = window.setTimeout(() => {
        timeout = 0
        raf = requestAnimationFrame(paint)
      }, wait)

    }

    const onVis = () => {
      if (document.hidden) { sim.releaseLook(); clearTimers() }
      else arm()
    }
    const onBlur = () => {
      unfocused = true
      sim.releaseLook()
      clearTimers()
    }
    const onFocus = () => {
      unfocused = false
      arm()
    }
    const onMotionPreference = () => {
      sim.setReducedMotion(media.matches)
      apply(performance.now())
      arm()
    }
    // Listen only inside the local hero/stage, never on window/document.
    // No overlay: nearby input fields and text keep their normal hit targets.
    const region = size >= 48 ? host.closest('[data-momo-region]') ?? host.parentElement : null
    const leave = () => {
      if (sim.releaseLook() && !raf) arm()
    }
    const follow = (event: Event) => {
      const pointer = event as PointerEvent
      if (pointer.pointerType === 'touch' || document.hidden || unfocused || !onScreen || pausedRef.current || media.matches || sim.mood !== 'idle') return
      const rect = host.getBoundingClientRect()
      const dx = pointer.clientX - rect.left - rect.width / 2
      const dy = pointer.clientY - rect.top - rect.height / 2
      const radius = Math.max(96, size * 0.75)
      if (Math.hypot(dx, dy) > radius) { leave(); return }
      sim.lookAt(dx / radius, dy / radius, performance.now())
      // Pointer events must not cancel a frame already waiting to paint.
      if (!raf) arm()
    }
    region?.addEventListener('pointermove', follow, { passive: true })
    region?.addEventListener('pointerleave', leave)
    media.addEventListener('change', onMotionPreference)
    document.addEventListener('visibilitychange', onVis)
    window.addEventListener('blur', onBlur)
    window.addEventListener('focus', onFocus)

    const io = new IntersectionObserver((entries) => {
      onScreen = entries.some((entry) => entry.isIntersecting)
      if (onScreen) arm()
      else { sim.releaseLook(); clearTimers() }
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
      media.removeEventListener('change', onMotionPreference)
      region?.removeEventListener('pointermove', follow)
      region?.removeEventListener('pointerleave', leave)
    }
  }, [size])

  return (
    <span
      ref={hostRef}
      role="button"
      aria-label="墨墨 · Momo"
      tabIndex={0}
      className="inline-flex shrink-0 cursor-pointer select-none"
      style={{ '--kv-stream-logo-size': `${size}px`, width: size, height: size } as CSSProperties}
      onPointerDown={(event) => {
        if (event.button !== 0) return
        event.preventDefault()
        pokeAt(event.clientX, event.clientY)
      }}
      onKeyDown={(event) => {
        if (event.key !== 'Enter' && event.key !== ' ') return
        event.preventDefault()
        pokeAt()
      }}
    >
      <svg
        className="kv-stream-logo"
        viewBox="0 0 240 240"
        width={size}
        height={size}
        overflow="visible"
        aria-hidden="true"
      >
        <defs>
          <clipPath id={`${shapeId}-clip`}><use href={`#${shapeId}-body`} /></clipPath>
        </defs>
        <g ref={rigRef} transform={BLOB_REST.rig}>
          <path id={`${shapeId}-body`} ref={bodyRef} d={BLOB_REST.bodyD} fill={BLOB_BLUE} transform={BLOB_REST.body} />
          <g clipPath={`url(#${shapeId}-clip)`}>
          <path ref={eye0Ref} fill={BLOB_EYE} d={BLOB_REST.eyes[0].d} transform={BLOB_REST.eyes[0].transform} />
          <path ref={eye1Ref} fill={BLOB_EYE} d={BLOB_REST.eyes[1].d} transform={BLOB_REST.eyes[1].transform} />
          </g>
        </g>
      </svg>
    </span>
  )
}))
