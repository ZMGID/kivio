import { describe, expect, it } from 'vitest'
import { nextMarkerPosition } from './useTrailingMarkerGlide'

describe('nextMarkerPosition', () => {
  it('首次和减少动态效果时直接落位', () => {
    expect(nextMarkerPosition(null, 300, 16)).toBe(300)
    expect(nextMarkerPosition(100, 124, 16, { reducedMotion: true })).toBe(124)
  })

  it('一行新字的位移按帧平滑逼近，不一帧跳完', () => {
    const next = nextMarkerPosition(100, 124, 16)
    expect(next).toBeGreaterThan(100)
    expect(next).toBeLessThan(110)
    let shown = 100
    for (let frame = 0; frame < 30; frame += 1) shown = nextMarkerPosition(shown, 124, 16)
    expect(shown).toBe(124)
  })

  it('往下追时最多落后 maxLag，不让新长出的正文压过来', () => {
    expect(nextMarkerPosition(100, 160, 0, { maxLagPx: 24 })).toBe(136)
    const next = nextMarkerPosition(136, 160, 16, { maxLagPx: 24 })
    expect(next).toBeGreaterThan(136)
    expect(next).toBeLessThan(160)
  })

  it('往上（内容变短）不受 maxLag 限制，照常平滑', () => {
    const next = nextMarkerPosition(160, 100, 16, { maxLagPx: 24 })
    expect(next).toBeLessThan(160)
    expect(next).toBeGreaterThan(124)
  })

  it('大位移（发送、切会话）直接落位', () => {
    expect(nextMarkerPosition(100, 400, 16)).toBe(400)
  })

  it('同一帧内（dt=0）且在上限内时不移动', () => {
    expect(nextMarkerPosition(100, 124, 0)).toBe(100)
  })
})
