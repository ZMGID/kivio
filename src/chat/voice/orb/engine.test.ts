import { describe, expect, it, vi } from 'vitest'
import { createOrbCloud, createOrbFrame, ORB_STATES, orbPoint, paintOrb, voiceEnergy, type OrbShape } from './engine'

const point = () => ({ x: 0, y: 0, depth: 0 })

function mix(a: OrbShape, b: OrbShape, amount: number): OrbShape {
  return {
    size: a.size + (b.size - a.size) * amount,
    stretch: a.stretch + (b.stretch - a.stretch) * amount,
    ripple: a.ripple + (b.ripple - a.ripple) * amount,
    twist: a.twist + (b.twist - a.twist) * amount,
    speed: a.speed + (b.speed - a.speed) * amount,
    opacity: a.opacity + (b.opacity - a.opacity) * amount,
  }
}

describe('dimensional voice particles', () => {
  it('uses a deterministic cloud with a large spherical footprint and visible depth', () => {
    const cloud = createOrbCloud(960)
    expect(cloud).toEqual(createOrbCloud(960))
    const frame = createOrbFrame(0.8, 0, ORB_STATES.listening)
    const p = point()
    let minX = 1
    let maxX = 0
    let minY = 1
    let maxY = 0
    let center = 0
    let far = 0
    let near = 0
    for (let i = 0; i < cloud.length / 3; i++) {
      orbPoint(cloud, i, frame, p)
      minX = Math.min(minX, p.x)
      maxX = Math.max(maxX, p.x)
      minY = Math.min(minY, p.y)
      maxY = Math.max(maxY, p.y)
      if (Math.hypot(p.x - 0.5, p.y - 0.5) < 0.18) center++
      if (p.depth < 0.25) far++
      if (p.depth > 0.75) near++
    }
    expect(maxX - minX).toBeGreaterThan(0.73)
    expect(maxX - minX).toBeLessThan(0.85)
    expect(maxY - minY).toBeGreaterThan(0.73)
    expect(maxY - minY).toBeLessThan(0.85)
    expect(center).toBeGreaterThan(50)
    expect(far).toBeGreaterThan(100)
    expect(near).toBeGreaterThan(100)
  })

  it('keeps every state finite, bounded and smooth across motion and state changes', () => {
    const cloud = createOrbCloud(960)
    const current = point()
    const next = point()
    const states = Object.values(ORB_STATES)
    for (const shape of states) {
      for (const time of [0, 0.8, 10, 90]) {
        for (const energy of [0, 1]) {
          const frame = createOrbFrame(time, energy, shape)
          const nextFrame = createOrbFrame(time + 1 / 30, energy, shape)
          for (let i = 0; i < cloud.length / 3; i += 7) {
            orbPoint(cloud, i, frame, current)
            orbPoint(cloud, i, nextFrame, next)
            expect(Number.isFinite(current.x) && Number.isFinite(current.y) && Number.isFinite(current.depth)).toBe(true)
            expect(current.x).toBeGreaterThan(0.02)
            expect(current.x).toBeLessThan(0.98)
            expect(current.y).toBeGreaterThan(0.02)
            expect(current.y).toBeLessThan(0.98)
            expect(current.depth).toBeGreaterThanOrEqual(0)
            expect(current.depth).toBeLessThanOrEqual(1)
            expect(Math.hypot(next.x - current.x, next.y - current.y)).toBeLessThan(0.01)
          }
        }
      }
      for (const target of states) {
        for (let step = 0; step < 10; step++) {
          const frame = createOrbFrame(2, 0.4, mix(shape, target, step / 10))
          const nextFrame = createOrbFrame(2, 0.4, mix(shape, target, (step + 1) / 10))
          for (let i = 0; i < cloud.length / 3; i += 29) {
            orbPoint(cloud, i, frame, current)
            orbPoint(cloud, i, nextFrame, next)
            expect(Math.hypot(next.x - current.x, next.y - current.y)).toBeLessThan(0.035)
          }
        }
      }
    }
  })

  it('gives connecting, working, speaking and muted states distinct continuous forms', () => {
    const cloud = createOrbCloud(960)
    const reference = point()
    const altered = point()
    const listening = createOrbFrame(1, 0, ORB_STATES.listening)
    for (const shape of [ORB_STATES.connecting, ORB_STATES.working, ORB_STATES.composing, ORB_STATES.breathing]) {
      const frame = createOrbFrame(1, 0, shape)
      let movement = 0
      for (let i = 0; i < cloud.length / 3; i++) {
        orbPoint(cloud, i, listening, reference)
        orbPoint(cloud, i, frame, altered)
        movement += Math.hypot(altered.x - reference.x, altered.y - reference.y)
      }
      expect(movement).toBeGreaterThan(0.5)
    }
  })

  it('responds to audio and paints monochrome depth-scaled dots with a cheaper inline cloud', () => {
    const cloud = createOrbCloud(960)
    const quiet = point()
    const loud = point()
    const quietFrame = createOrbFrame(1, 0, ORB_STATES.listening)
    const loudFrame = createOrbFrame(1, 1, ORB_STATES.listening)
    let movement = 0
    for (let i = 0; i < cloud.length / 3; i++) {
      orbPoint(cloud, i, quietFrame, quiet)
      orbPoint(cloud, i, loudFrame, loud)
      movement += Math.hypot(loud.x - quiet.x, loud.y - quiet.y)
    }
    expect(movement).toBeGreaterThan(1)

    const radii: number[] = []
    const alphas: number[] = []
    const ctx = {
      fillStyle: '', globalAlpha: 1,
      beginPath: vi.fn(), fill: vi.fn(),
      arc: vi.fn((_x: number, _y: number, radius: number) => { radii.push(radius) }),
    }
    ctx.fill.mockImplementation(() => { alphas.push(ctx.globalAlpha) })
    paintOrb(ctx as unknown as CanvasRenderingContext2D, 560, 1, 0, ORB_STATES.listening, 'rgb(12, 12, 12)')
    expect(ctx.arc).toHaveBeenCalledTimes(960)
    expect(ctx.fillStyle).toBe('rgb(12, 12, 12)')
    expect(ctx.globalAlpha).toBe(1)
    expect(Math.max(...radii)).toBeGreaterThan(Math.min(...radii) * 2)
    expect(Math.max(...alphas)).toBeGreaterThan(Math.min(...alphas) * 2)
    ctx.arc.mockClear()
    paintOrb(ctx as unknown as CanvasRenderingContext2D, 18, 1, 0, ORB_STATES.listening, 'white')
    expect(ctx.arc).toHaveBeenCalledTimes(64)
    expect(ctx.fillStyle).toBe('white')
  })

  it('bounds malformed and saturated meter input without dropping quiet speech', () => {
    for (const peak of [NaN, Infinity, -1, 0]) expect(voiceEnergy(peak)).toBe(0)
    expect(voiceEnergy(10)).toBe(1)
    expect(voiceEnergy(0.01)).toBeGreaterThan(0.1)
    expect(voiceEnergy(0.01)).toBeLessThan(voiceEnergy(0.1))
  })
})
