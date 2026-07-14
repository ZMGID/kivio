import { describe, expect, it } from 'vitest'
import {
  layoutReplaceText,
  layoutReplaceTextFlow,
  replaceTextVerticalOffset,
  tokenizeReplaceText,
  wrapReplaceText,
} from './replaceTextLayout'

const measure = (text: string, fontPx: number) => text.length * fontPx * 0.55

describe('replace text layout', () => {
  it('keeps CJK characters independently wrappable and Latin identifiers intact', () => {
    expect(tokenizeReplaceText('网络 web_search 工具')).toEqual(['网', '络', ' ', 'web_search', ' ', '工', '具'])
    const lines = wrapReplaceText('网络 web_search 工具', 60, 12, measure)
    expect(lines.join('')).toContain('web_search')
  })

  it('preserves explicit paragraph breaks', () => {
    expect(wrapReplaceText('first\nsecond', 200, 12, measure)).toEqual(['first', 'second'])
  })

  it('selects the largest fitting font down to the preferred minimum', () => {
    const layout = layoutReplaceText('complete translation', { width: 120, height: 40 }, 18, measure)
    expect(layout.fontPx).toBeGreaterThanOrEqual(7)
    expect(layout.contentHeight).toBeLessThanOrEqual(40)
    expect(layout.safeScale).toBe(1)
  })

  it('uses final safe scaling instead of truncating long text', () => {
    const text = '完整译文必须全部保留'.repeat(30)
    const layout = layoutReplaceText(text, { width: 45, height: 18 }, 16, measure)
    expect(layout.fontPx).toBe(7)
    expect(layout.safeScale).toBeLessThan(1)
    expect(layout.lines.join('').replaceAll(' ', '')).toBe(text)
  })
})

describe('replace text vertical placement', () => {
  it('top-aligns paragraphs so merged OCR does not shift the first line', () => {
    expect(replaceTextVerticalOffset('paragraph', 500, 320)).toBe(0)
  })

  it('keeps short labels and cells vertically centered', () => {
    expect(replaceTextVerticalOffset('line', 60, 40)).toBe(10)
    expect(replaceTextVerticalOffset('cell', 60, 40)).toBe(10)
  })
})

describe('replace text multi-slot flow', () => {
  it('keeps a translation group while preserving each source-line slot', () => {
    const text = '第一行译文和第二行译文必须按原来的两个位置流动'
    const layout = layoutReplaceTextFlow(
      text,
      [{ width: 110, height: 22 }, { width: 110, height: 22 }],
      14,
      measure,
    )
    expect(layout.complete).toBe(true)
    expect(layout.slots).toHaveLength(2)
    expect(layout.slots.flatMap(slot => slot.lines).join('')).toBe(text)
  })

  it('uses a shared safe scale rather than dropping the tail from the last slot', () => {
    const text = '完整译文'.repeat(80)
    const layout = layoutReplaceTextFlow(
      text,
      [{ width: 60, height: 18 }, { width: 60, height: 18 }],
      16,
      measure,
    )
    expect(layout.complete).toBe(true)
    expect(layout.safeScale).toBeLessThan(1)
    expect(layout.slots.flatMap(slot => slot.lines).join('')).toBe(text)
  })
})
