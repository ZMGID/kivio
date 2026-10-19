import { describe, expect, it } from 'vitest'
import { bodyPoints, facePoints, BODY_R, CX, CY } from './kivioBlobShapes'
import { createBlobProjector, projectBlobVolume } from './kivioBlobVolume'

function points(d: string) {
  const values = [...d.matchAll(/-?\d+\.\d+/g)].map(m => Number(m[0]))
  return Array.from({ length: values.length / 2 }, (_, i) => [values[i * 2], values[i * 2 + 1]])
}
function area(d: string) {
  const p = points(d)
  return Math.abs(p.reduce((s, a, i) => {
    const b = p[(i + 1) % p.length]
    return s + a[0] * b[1] - a[1] * b[0]
  }, 0)) / 2
}
const project = (yaw: number, pitch = 0, squash = 1) => projectBlobVolume(
  bodyPoints('circle'), facePoints('neutral'), { yaw, pitch, squash },
)

describe('curved facial projection', () => {
  it('rotation keeps the spherical silhouette while the far eye foreshortens', () => {
    const front = project(0), side = project(0.9, 0.3)
    expect(area(side.bodyD) / area(front.bodyD)).toBeGreaterThan(0.98)
    expect(area(side.bodyD) / area(front.bodyD)).toBeLessThan(1.02)
    expect(area(side.eyes[1])).toBeLessThan(area(side.eyes[0]) * 0.65)
    for (const eye of side.eyes) {
      for (const [x, y] of points(eye)) expect(Math.hypot(x - CX, y - CY)).toBeLessThanOrEqual(BODY_R + 0.2)
    }
  })

  it('the outer eye disappears around the back instead of reappearing mirrored', () => {
    const side = project(1.7)
    expect(side.eyes[1]).toBe('')
    expect(area(side.eyes[0])).toBeGreaterThan(1)
    expect(project(Math.PI).eyes).toEqual(['', ''])
  })

  it('a right/up turn warps each eye outline rather than applying one affine transform', () => {
    const front = points(project(0).eyes[0]), turn = points(project(0.65, 0.5).eyes[0])
    // Centrally symmetric flat ovals keep the same midpoint for opposite
    // vertices under ANY affine transform. Curved-surface projection does not.
    const midpointSpread = (p: number[][]) => {
      const mids = p.slice(0, p.length / 2).map((v, i) => (v[0] + p[i + p.length / 2][0]) / 2)
      return Math.max(...mids) - Math.min(...mids)
    }
    expect(midpointSpread(front)).toBeLessThan(0.02)
    expect(midpointSpread(turn)).toBeGreaterThan(0.5)
  })

  it('squashing deforms the face together with the body and widens its volume', () => {
    const front = project(0), squashed = project(0, 0, 0.75)
    const bounds = (d: string) => {
      const p = points(d)
      return [Math.max(...p.map(p => p[0])) - Math.min(...p.map(p => p[0])), Math.max(...p.map(p => p[1])) - Math.min(...p.map(p => p[1]))]
    }
    const a = bounds(front.bodyD), b = bounds(squashed.bodyD)
    const e = bounds(front.eyes[0]), f = bounds(squashed.eyes[0])
    expect(b[0]).toBeGreaterThan(a[0])
    expect(b[1] / a[1]).toBeCloseTo(0.75, 2)
    expect(f[0] / e[0]).toBeCloseTo(b[0] / a[0], 2)
    expect(f[1] / e[1]).toBeCloseTo(b[1] / a[1], 2)
  })
})


it('cached geometry follows in-place shape, expression and pose edits without leaking to another character', () => {
  const render = createBlobProjector(), other = createBlobProjector()
  const profile = bodyPoints('circle').map(p => [...p] as [number, number])
  const eyes = facePoints('neutral').map(eye => eye.map(p => [...p] as [number, number]))
  const pose = { yaw: 0, pitch: 0, squash: 1 }
  const front = render(profile, eyes, pose)
  other(bodyPoints('squircle'), facePoints('happy'), { yaw: 1, pitch: 0.3, squash: 0.8 })
  expect(render(profile, eyes, pose)).toEqual(front)
  for (const eye of eyes) for (const p of eye) p[1] = CY + (p[1] - CY) * 0.5
  const blink = render(profile, eyes, pose)
  expect(blink.bodyD).toBe(front.bodyD)
  expect(area(blink.eyes[0]) / area(front.eyes[0])).toBeCloseTo(0.5, 2)
  for (const p of profile) p[0] = CX + (p[0] - CX) * 0.8
  const narrow = render(profile, eyes, pose)
  expect(area(narrow.bodyD) / area(front.bodyD)).toBeCloseTo(0.8, 2)
  pose.yaw = Math.PI
  expect(render(profile, eyes, pose).eyes).toEqual(['', ''])
})
