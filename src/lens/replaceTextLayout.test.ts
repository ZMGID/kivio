import { describe, expect, it } from 'vitest'
import {
  layoutReplaceTextFlow,
  replaceTextVerticalOffset,
  tokenizeReplaceText,
} from './replaceTextLayout'

const measure = (text: string, fontPx: number) => text.length * fontPx * 0.55

describe('replace text tokenization', () => {
  it('keeps CJK characters independently breakable and Latin identifiers intact', () => {
    expect(tokenizeReplaceText('网络 web_search 工具')).toEqual(['网', '络', ' ', 'web_search', ' ', '工', '具'])
  })

  it('emits explicit paragraph breaks as standalone newline tokens', () => {
    expect(tokenizeReplaceText('first\nsecond')).toEqual(['first', '\n', 'second'])
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

  it('flows a single over-wide unbreakable token character by character', () => {
    // A lone long token (no spaces to break on) must still be laid out fully
    // by splitting characters across lines — not dropped or truncated.
    const url = 'httpsexamplecomverylongpathwithnobreaks'
    const layout = layoutReplaceTextFlow(url, [{ width: 40, height: 90 }], 16, measure)
    expect(layout.complete).toBe(true)
    expect(layout.slots[0].lines.join('')).toBe(url)
  })
})
