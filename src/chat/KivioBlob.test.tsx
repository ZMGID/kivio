import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, expect, it, vi } from 'vitest'
import { KivioBlob } from './KivioBlob'
import { KivioBlobSim } from './kivioBlobSim'

afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.useRealTimers() })

it('updates static state with reduced motion, responds to live preference changes, and releases its frames', () => {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'performance'] })
  const events = new EventTarget()
  let reduced = true
  vi.stubGlobal('matchMedia', () => ({
    get matches() { return reduced },
    addEventListener: events.addEventListener.bind(events),
    removeEventListener: events.removeEventListener.bind(events),
  }))
  vi.stubGlobal('requestAnimationFrame', (fn: FrameRequestCallback) => window.setTimeout(() => fn(performance.now()), 16))
  vi.stubGlobal('cancelAnimationFrame', (id: number) => clearTimeout(id))
  const view = render(<KivioBlob mood="idle" />)
  const picture = () => view.container.querySelector('svg')!.innerHTML
  const idle = picture()
  view.rerender(<KivioBlob mood="work" />)
  expect(picture()).not.toBe(idle)
  const still = picture()
  act(() => vi.advanceTimersByTime(300))
  expect(picture()).toBe(still)
  act(() => { reduced = false; events.dispatchEvent(new Event('change')); vi.advanceTimersByTime(300) })
  expect(picture()).not.toBe(still)
  act(() => { reduced = true; events.dispatchEvent(new Event('change')) })
  const reducedFrame = picture()
  act(() => vi.advanceTimersByTime(300))
  expect(picture()).toBe(reducedFrame)
  fireEvent.keyDown(screen.getByRole('button', { name: '墨墨 · Momo' }), { key: 'Enter' })
  expect(picture()).not.toBe(reducedFrame) // color feedback remains available
  view.unmount()
  expect(vi.getTimerCount()).toBe(0)
})


it('ignores global pointer movement while idle but still responds to a deliberate click', () => {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'performance'] })
  vi.stubGlobal('matchMedia', () => ({ matches: false, addEventListener() {}, removeEventListener() {} }))
  vi.stubGlobal('requestAnimationFrame', (fn: FrameRequestCallback) => window.setTimeout(() => fn(performance.now()), 16))
  vi.stubGlobal('cancelAnimationFrame', (id: number) => clearTimeout(id))
  const view = render(<KivioBlob size={220} />)
  const picture = () => view.container.querySelector('svg')!.innerHTML
  const still = picture()
  fireEvent.pointerMove(window, { clientX: 140, clientY: 70, pointerType: 'mouse' })
  act(() => vi.advanceTimersByTime(300))
  expect(picture()).toBe(still)
  expect(vi.getTimerCount()).toBe(1)
  fireEvent.keyDown(screen.getByRole('button', { name: '墨墨 · Momo' }), { key: 'Enter' })
  act(() => vi.advanceTimersByTime(300))
  expect(picture()).not.toBe(still)
  view.unmount()
  expect(vi.getTimerCount()).toBe(0)
})


it.each([60, 120, 144])('paces visible animation at 60 fps on a %i Hz display and resumes without a burst', (hz) => {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'performance'] })
  vi.stubGlobal('matchMedia', () => ({ matches: false, addEventListener() {}, removeEventListener() {} }))
  const refresh = 1000 / hz
  vi.stubGlobal('requestAnimationFrame', (fn: FrameRequestCallback) => {
    const next = (Math.floor(performance.now() / refresh) + 1) * refresh
    return window.setTimeout(() => fn(next), Math.max(1, Math.ceil(next - performance.now())))
  })
  vi.stubGlobal('cancelAnimationFrame', (id: number) => clearTimeout(id))
  const view = render(<KivioBlob mood="work" />)
  const rig = view.container.querySelector('svg > g')!
  const times: number[] = []
  const sample = KivioBlobSim.prototype.sample
  vi.spyOn(KivioBlobSim.prototype, 'sample').mockImplementation(function (this: KivioBlobSim, now) {
    times.push(now)
    return sample.call(this, now)
  })
  act(() => vi.advanceTimersByTime(5000))
  expect(times.length).toBeGreaterThanOrEqual(298)
  expect(times.length).toBeLessThanOrEqual(300)
  const gaps = times.slice(1).map((time, i) => time - times[i])
  // Quantization to display refresh is unavoidable; no cumulative drift or burst.
  expect(Math.min(...gaps)).toBeGreaterThanOrEqual(16.666 - refresh - 2)
  expect(Math.max(...gaps)).toBeLessThanOrEqual(16.667 + refresh + 2)
  fireEvent.blur(window)
  const count = times.length
  act(() => vi.advanceTimersByTime(2000))
  expect(times).toHaveLength(count)
  expect(vi.getTimerCount()).toBe(0)
  fireEvent.focus(window)
  act(() => vi.advanceTimersByTime(1000))
  expect(times.length - count).toBeGreaterThanOrEqual(59)
  expect(times.length - count).toBeLessThanOrEqual(61)
  view.rerender(<KivioBlob mood="idle" />)
  act(() => vi.advanceTimersByTime(20000))
  const still = rig.outerHTML
  expect(vi.getTimerCount()).toBe(0)
  act(() => vi.advanceTimersByTime(2000))
  expect(rig.outerHTML).toBe(still)
  view.unmount()
})


it('follows only nearby local movement, returns on leave, and leaves small icons alone', () => {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'performance'] })
  vi.stubGlobal('PointerEvent', MouseEvent)
  vi.stubGlobal('matchMedia', () => ({ matches: false, addEventListener() {}, removeEventListener() {} }))
  vi.stubGlobal('requestAnimationFrame', (fn: FrameRequestCallback) => window.setTimeout(() => fn(performance.now()), 8))
  vi.stubGlobal('cancelAnimationFrame', (id: number) => clearTimeout(id))
  const view = render(<div data-momo-region><KivioBlob size={56} /><KivioBlob size={28} /></div>)
  const hosts = screen.getAllByRole('button', { name: '墨墨 · Momo' })
  vi.spyOn(hosts[0], 'getBoundingClientRect').mockReturnValue({ left: 100, top: 100, width: 56, height: 56 } as DOMRect)
  const picture = (i: number) => hosts[i].querySelector('svg')!.innerHTML
  const front = picture(0), small = picture(1)
  const region = view.container.firstElementChild!
  fireEvent.pointerMove(window, { clientX: 180, clientY: 90 })
  fireEvent.pointerMove(region, { clientX: 500, clientY: 90 })
  act(() => vi.advanceTimersByTime(1000))
  expect(picture(0)).toBe(front)
  expect(vi.getTimerCount()).toBe(1)
  fireEvent.pointerMove(region, { clientX: 180, clientY: 90 })
  act(() => vi.advanceTimersByTime(1000))
  expect(picture(0)).not.toBe(front)
  expect(picture(1)).toBe(small)
  fireEvent.pointerLeave(region)
  act(() => vi.advanceTimersByTime(4000))
  const coords = (markup: string) => [...markup.matchAll(/-?\d+\.\d+/g)].map(m => Number(m[0]))
  const restored = coords(picture(0)), original = coords(front)
  expect(restored).toHaveLength(original.length)
  expect(Math.max(...restored.map((v, i) => Math.abs(v - original[i])))).toBeLessThan(0.03)
  expect(vi.getTimerCount()).toBe(1)
  view.unmount()
})
