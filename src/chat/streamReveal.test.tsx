import { act, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { nextRevealLength, revealLagMs, setRevealPacingForTest, useRevealedText } from './streamReveal'

describe('nextRevealLength', () => {
  it('drains a backlog in roughly the lag window without overshooting', () => {
    let shown = 0
    const target = 300
    let elapsed = 0
    while (shown < target && elapsed < 2_000) {
      const next = nextRevealLength(shown, target, 16, 200)
      expect(next).toBeGreaterThan(shown)
      expect(next).toBeLessThanOrEqual(target)
      shown = next
      elapsed += 16
    }
    expect(shown).toBe(target)
    // Backlog-proportional speed plus a floor: a 300-char burst lands in about half a second.
    expect(elapsed).toBeLessThan(650)
  })

  it('reveals at least one character per tick and jumps over huge backlogs', () => {
    expect(nextRevealLength(10, 11, 1, 260)).toBe(11)
    expect(nextRevealLength(0, 50_000, 16, 260)).toBe(50_000 - 600)
    expect(nextRevealLength(20, 10, 16, 260)).toBe(10)
  })

  it('stretches the lag for sparse, bursty arrivals within bounds', () => {
    expect(revealLagMs(null)).toBe(200)
    expect(revealLagMs(40)).toBe(120)
    expect(revealLagMs(300)).toBe(390)
    expect(revealLagMs(5_000)).toBe(700)
  })
})

function Probe({ text, live }: { text: string; live: boolean }) {
  return <output>{useRevealedText(text, live)}</output>
}

describe('useRevealedText', () => {
  beforeEach(() => {
    setRevealPacingForTest(true)
    vi.useFakeTimers({ toFake: ['requestAnimationFrame', 'cancelAnimationFrame', 'performance', 'setTimeout', 'clearTimeout', 'Date'] })
  })
  afterEach(() => {
    vi.useRealTimers()
    setRevealPacingForTest(false)
  })

  it('shows text present at mount immediately and paces later growth monotonically', async () => {
    const { container, rerender } = render(<Probe text="Hello" live />)
    const shown = () => container.querySelector('output')!.textContent ?? ''
    expect(shown()).toBe('Hello')
    const full = `Hello${' streaming answer'.repeat(8)}`
    rerender(<Probe text={full} live />)
    expect(shown()).toBe('Hello')
    let previous = shown().length
    let sawPartial = false
    for (let frame = 0; frame < 120 && shown() !== full; frame++) {
      await act(async () => { vi.advanceTimersByTime(16) })
      expect(full.startsWith(shown())).toBe(true)
      expect(shown().length).toBeGreaterThanOrEqual(previous)
      if (shown().length > 5 && shown().length < full.length) sawPartial = true
      previous = shown().length
    }
    expect(sawPartial).toBe(true)
    expect(shown()).toBe(full)
  })

  it('shows everything at once when the stream stops or the text is replaced', async () => {
    const { container, rerender } = render(<Probe text="Start" live />)
    const shown = () => container.querySelector('output')!.textContent ?? ''
    rerender(<Probe text={`Start${'x'.repeat(200)}`} live />)
    await act(async () => { vi.advanceTimersByTime(16) })
    expect(shown().length).toBeLessThan(205)
    rerender(<Probe text="Rewritten answer" live />)
    expect(shown()).toBe('Rewritten answer')
    rerender(<Probe text={`Rewritten answer${'y'.repeat(200)}`} live={false} />)
    expect(shown()).toBe(`Rewritten answer${'y'.repeat(200)}`)
  })

  it('never cuts a surrogate pair in half', async () => {
    const { container, rerender } = render(<Probe text="" live />)
    const full = '😀'.repeat(40)
    rerender(<Probe text={full} live />)
    for (let frame = 0; frame < 60; frame++) {
      await act(async () => { vi.advanceTimersByTime(16) })
      const value = container.querySelector('output')!.textContent ?? ''
      expect(value.length % 2).toBe(0)
    }
  })
})
