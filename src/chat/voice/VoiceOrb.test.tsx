import { act, render } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { VoiceOrb } from './VoiceOrb'
import * as engine from './orb/engine'

afterEach(() => {
  vi.useRealTimers()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
  document.documentElement.classList.remove('dark')
})

it('pauses while hidden, honors reduced motion and releases its work on unmount', async () => {
  vi.useFakeTimers()
  const started = Date.now()
  vi.spyOn(performance, 'now').mockImplementation(() => Date.now() - started)
  let hidden = false
  vi.spyOn(document, 'hidden', 'get').mockImplementation(() => hidden)
  const changes = new EventTarget()
  let reduced = false
  vi.stubGlobal('matchMedia', () => ({
    get matches() { return reduced },
    addEventListener: changes.addEventListener.bind(changes),
    removeEventListener: changes.removeEventListener.bind(changes),
  }))
  const frames = new Map<number, FrameRequestCallback>()
  let nextFrame = 0
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
    const id = ++nextFrame
    frames.set(id, callback)
    return id
  })
  vi.stubGlobal('cancelAnimationFrame', (id: number) => { frames.delete(id) })
  const context = {
    setTransform: vi.fn(), clearRect: vi.fn(), beginPath: vi.fn(),
    arc: vi.fn(), fill: vi.fn(),
  }
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(context as unknown as CanvasRenderingContext2D)
  const levelsRef = { current: { mic: 0, speaker: 0 } }
  const view = render(<VoiceOrb state="listening" levelsRef={levelsRef} size={128} />)
  expect(frames.size).toBe(1)
  act(() => { hidden = true; document.dispatchEvent(new Event('visibilitychange')) })
  expect(frames.size).toBe(0)
  act(() => { hidden = false; document.dispatchEvent(new Event('visibilitychange')) })
  expect(frames.size).toBe(1)

  act(() => {
    reduced = true
    const event = new Event('change')
    Object.defineProperty(event, 'matches', { value: true })
    changes.dispatchEvent(event)
  })
  expect(frames.size).toBe(0)
  expect(context.clearRect).toHaveBeenCalledTimes(1)
  view.rerender(<VoiceOrb state="working" levelsRef={levelsRef} size={128} />)
  act(() => { vi.advanceTimersByTime(1250) })
  expect(context.clearRect).toHaveBeenCalledTimes(2)
  view.container.querySelector('canvas')!.style.color = 'rgb(90, 80, 70)'
  act(() => { vi.advanceTimersByTime(1250) })
  expect(context.clearRect).toHaveBeenCalledTimes(3)
  levelsRef.current.speaker = 0.25
  act(() => { vi.advanceTimersByTime(250) })
  expect(context.clearRect).toHaveBeenCalledTimes(4)
  levelsRef.current.speaker = 0
  act(() => { vi.advanceTimersByTime(250) })
  expect(context.clearRect).toHaveBeenCalledTimes(5)
  await act(async () => { document.documentElement.classList.add('dark') })
  expect(context.clearRect).toHaveBeenCalledTimes(6)
  act(() => { hidden = true; document.dispatchEvent(new Event('visibilitychange')) })
  expect(vi.getTimerCount()).toBe(0)
  act(() => { hidden = false; document.dispatchEvent(new Event('visibilitychange')) })
  expect(context.clearRect).toHaveBeenCalledTimes(7)

  act(() => {
    reduced = false
    const event = new Event('change')
    Object.defineProperty(event, 'matches', { value: false })
    changes.dispatchEvent(event)
  })
  expect(frames.size).toBe(1)
  view.unmount()
  expect(frames.size).toBe(0)
  expect(vi.getTimerCount()).toBe(0)
})

it('smoothly follows mic and speaker levels, including speaker output while muted', () => {
  vi.spyOn(document, 'hidden', 'get').mockReturnValue(false)
  const changes = new EventTarget()
  vi.stubGlobal('matchMedia', () => ({
    matches: false,
    addEventListener: changes.addEventListener.bind(changes),
    removeEventListener: changes.removeEventListener.bind(changes),
  }))
  let now = 0
  vi.spyOn(performance, 'now').mockImplementation(() => now)
  const frames = new Map<number, FrameRequestCallback>()
  let nextFrame = 0
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
    const id = ++nextFrame
    frames.set(id, callback)
    return id
  })
  vi.stubGlobal('cancelAnimationFrame', (id: number) => { frames.delete(id) })
  const step = (time: number) => {
    now = time
    const entry = frames.entries().next()
    if (entry.done) throw new Error('Expected an animation frame')
    const [id, callback] = entry.value
    frames.delete(id)
    act(() => callback(time))
  }
  const context = { setTransform: vi.fn(), clearRect: vi.fn() }
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(context as unknown as CanvasRenderingContext2D)
  const snapshots: Array<{ time: number; energy: number; shape: engine.OrbShape; ink: string }> = []
  vi.spyOn(engine, 'paintOrb').mockImplementation((_ctx, _size, time, energy, shape, ink) => {
    snapshots.push({ time, energy, shape: { ...shape }, ink })
  })
  const levelsRef = { current: { mic: 0, speaker: 0 } }
  const view = render(<VoiceOrb state="listening" levelsRef={levelsRef} size={128} />)
  const canvas = view.container.querySelector('canvas')!
  canvas.style.color = 'rgb(12, 34, 56)'
  step(40)
  expect(snapshots[0].energy).toBe(0)
  expect(snapshots[0].ink).toBe('rgb(12, 34, 56)')
  levelsRef.current.mic = 0.04
  step(80)
  expect(snapshots[1].energy).toBeGreaterThan(0)
  expect(snapshots[1].energy).toBeLessThan(engine.voiceEnergy(0.04))
  levelsRef.current.mic = 0
  step(120)
  expect(snapshots[2].energy).toBeGreaterThan(0)
  expect(snapshots[2].energy).toBeLessThan(snapshots[1].energy)

  view.rerender(<VoiceOrb state="working" levelsRef={levelsRef} size={128} />)
  step(160)
  expect(snapshots[3].shape.twist).toBeGreaterThan(snapshots[2].shape.twist)
  expect(snapshots[3].shape.twist).toBeLessThan(engine.ORB_STATES.working.twist)
  expect(snapshots[3].time - snapshots[2].time).toBeLessThan(0.1)
  view.rerender(<VoiceOrb state="breathing" levelsRef={levelsRef} size={128} />)
  levelsRef.current.mic = 1
  step(200)
  expect(snapshots[4].energy).toBeLessThan(snapshots[3].energy)
  levelsRef.current.speaker = 0.25
  step(240)
  expect(snapshots[5].energy).toBeGreaterThan(snapshots[4].energy)
  expect(snapshots[5].shape.opacity).toBeGreaterThan(snapshots[4].shape.opacity)
  levelsRef.current.speaker = 0
  step(280)
  expect(snapshots[6].shape.opacity).toBeGreaterThan(snapshots[5].shape.opacity)
  step(520)
  expect(snapshots[7].shape.opacity).toBeLessThan(snapshots[6].shape.opacity)
  view.rerender(<VoiceOrb state="connecting" levelsRef={levelsRef} size={128} />)
  levelsRef.current.speaker = 1
  step(560)
  expect(snapshots[8].energy).toBeLessThan(snapshots[7].energy)

  canvas.style.color = 'white'
  step(1200)
  expect(snapshots[9].ink).toBe('rgb(255, 255, 255)')
  act(() => {
    const event = new Event('change')
    Object.defineProperty(event, 'matches', { value: true })
    changes.dispatchEvent(event)
  })
  expect(snapshots[10].time).toBe(snapshots[9].time)
  expect(snapshots[10].energy).toBe(0)
  act(() => {
    const event = new Event('change')
    Object.defineProperty(event, 'matches', { value: false })
    changes.dispatchEvent(event)
  })
  step(1240)
  expect(snapshots[11].time).toBeGreaterThan(snapshots[10].time)
  expect(snapshots[11].time - snapshots[10].time).toBeLessThan(0.1)
  view.unmount()
  expect(frames.size).toBe(0)
})
