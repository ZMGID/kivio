import { describe, expect, it } from 'vitest'
import { READY_BAR_H, clamp, computeAnchoredBar, computeMetrics, computeSelectBar } from './layout'

describe('clamp', () => {
  it('clamps values within bounds', () => {
    expect(clamp(5, 0, 10)).toBe(5)
    expect(clamp(-1, 0, 10)).toBe(0)
    expect(clamp(99, 0, 10)).toBe(10)
  })
})

describe('computeMetrics', () => {
  it('respects minimum bounds on small viewports', () => {
    const metrics = computeMetrics(800, 600)
    expect(metrics.READY_W).toBeGreaterThanOrEqual(420)
    expect(metrics.SELECT_W).toBeGreaterThanOrEqual(440)
    expect(metrics.ANSWER_H).toBeGreaterThanOrEqual(220)
    expect(metrics.SELECT_BOTTOM_OFFSET).toBeGreaterThanOrEqual(80)
  })

  it('respects maximum bounds on large viewports', () => {
    const metrics = computeMetrics(4000, 3000)
    expect(metrics.READY_W).toBeLessThanOrEqual(560)
    expect(metrics.SELECT_W).toBeLessThanOrEqual(640)
    expect(metrics.ANSWER_H).toBeLessThanOrEqual(480)
    expect(metrics.SELECT_BOTTOM_OFFSET).toBeLessThanOrEqual(160)
  })
})

describe('computeSelectBar', () => {
  it('centers the select bar horizontally near the bottom', () => {
    const metrics = computeMetrics(1920, 1080)
    const bar = computeSelectBar(1920, 1080, metrics)
    expect(bar.width).toBe(metrics.SELECT_W)
    expect(bar.x).toBe(Math.round(1920 / 2 - metrics.SELECT_W / 2))
    expect(bar.y).toBe(Math.round(1080 - metrics.SELECT_BOTTOM_OFFSET - 56))
  })
})

describe('computeAnchoredBar', () => {
  const base = {
    viewportWidth: 1920,
    viewportHeight: 1080,
    anchorX: 500,
    anchorY: 240,
    anchorWidth: 500,
    anchorHeight: 400,
    barWidth: 560,
    sideContentHeight: 480,
  }

  it('prefers the right side when the bar fits', () => {
    const bar = computeAnchoredBar(base)
    expect(bar.x).toBe(base.anchorX + base.anchorWidth + 12)
  })

  it('uses the left side when the right side does not fit', () => {
    const bar = computeAnchoredBar({
      ...base,
      anchorX: 900,
      anchorWidth: 600,
    })
    expect(bar.x).toBe(900 - 12 - base.barWidth)
  })

  it('places the bar below a large selection when neither side fits', () => {
    const anchor = { x: 48, y: 70, width: 1432, height: 912 }
    const bar = computeAnchoredBar({
      viewportWidth: 2048,
      viewportHeight: 1097,
      anchorX: anchor.x,
      anchorY: anchor.y,
      anchorWidth: anchor.width,
      anchorHeight: anchor.height,
      barWidth: 640,
      sideContentHeight: 544,
    })

    expect(bar).toEqual({ x: 444, y: 994, width: 640 })
    expect(bar.y).toBeGreaterThanOrEqual(anchor.y + anchor.height + 12)
    expect(bar.y + READY_BAR_H).toBeLessThanOrEqual(1097 - 16)
  })

  it('places the bar above when the sides and bottom do not fit', () => {
    const bar = computeAnchoredBar({
      viewportWidth: 1000,
      viewportHeight: 800,
      anchorX: 100,
      anchorY: 500,
      anchorWidth: 800,
      anchorHeight: 250,
      barWidth: 440,
      sideContentHeight: 420,
    })

    expect(bar.y + READY_BAR_H + 12).toBeLessThanOrEqual(500)
  })
})
