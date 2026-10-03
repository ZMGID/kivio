import { describe, expect, it } from 'vitest'
import {
  layoutReplaceTextFlow,
  replaceSlotTextBounds,
  replaceTextVerticalOffset,
  tokenizeReplaceText,
} from './replaceTextLayout'

const measure = (text: string, fontPx: number) => text.length * fontPx * 0.55

describe('replace text tokenization', () => {
  it('keeps CJK characters independently breakable and Latin identifiers intact', () => {
    expect(tokenizeReplaceText('网络 web_search 工具')).toEqual(['网', '络', ' ', 'web_search', ' ', '工', '具'])
  })

  it('keeps nested opening punctuation and CJK nonbreaking spaces together', () => {
    expect(tokenizeReplaceText('查看（《说明》）')).toEqual(['查', '看', '（《说', '明》）'])
    expect(tokenizeReplaceText('第\u00a0一页')).toEqual(['第\u00a0一', '页'])
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
  const inkMeasure = (text: string, size: number) => ({ width: [...text].length * size, height: size * 0.72 })

  it('keeps the requested size when the visible ink fits a tight source line', () => {
    const available = replaceSlotTextBounds({
      id: 's', groupId: 'g', leafIds: ['l'], kind: 'line', flow: 'exact_line',
      bounds: { x: 0, y: 0, width: 120, height: 24 },
      anchor: { x: 2, y: 8, baselineY: 22 }, align: 'left', verticalAlign: 'top',
      sourceFontPx: 14, sourceColor: '#111111',
    }, 2.8)
    const layout = layoutReplaceTextFlow('打开设置', [available], 14, inkMeasure)
    expect(layout.complete).toBe(true)
    expect(layout.fontPx * layout.safeScale).toBe(14)
  })

  it('keeps an exact source line single even when a translation needs a smaller font', () => {
    const available = replaceSlotTextBounds({
      id: 's', groupId: 'g', leafIds: ['l'], kind: 'line', flow: 'exact_line',
      bounds: { x: 0, y: 0, width: 120, height: 24 },
      anchor: { x: 2, y: 8, baselineY: 22 }, align: 'left', verticalAlign: 'top',
      sourceFontPx: 14, sourceColor: '#111111',
    }, 2.8)
    const text = '译'.repeat(24)
    const layout = layoutReplaceTextFlow(text, [available], 14, inkMeasure)
    expect(layout.complete).toBe(true)
    expect(layout.slots[0].lines).toEqual([text])
  })

  it('does not require trailing line leading below the final line of a cell', () => {
    const layout = layoutReplaceTextFlow('甲乙\n丙丁', [{ width: 30, height: 19 }], 10, inkMeasure)
    expect(layout.fontPx).toBe(10)
    expect(layout.slots[0].lines).toEqual(['甲乙', '丙丁'])
    expect(layout.slots[0].contentHeight).toBeCloseTo(19)
  })

  it('keeps Chinese closing punctuation off the beginning of a line', () => {
    const layout = layoutReplaceTextFlow('你好，世界', [{ width: 20, height: 100 }], 10, inkMeasure)
    expect(layout.slots[0].lines).toEqual(['你', '好，', '世界'])
  })

  it('keeps a nonbreaking space with its words', () => {
    const layout = layoutReplaceTextFlow('AB\u00a0CD', [{ width: 20, height: 100 }], 10, inkMeasure)
    expect(layout.slots[0].lines).toEqual(['AB\u00a0CD'])
    expect(layout.complete).toBe(true)
  })

  it('never splits combining accents and emoji families during emergency wrapping', () => {
    const text = 'a\u0301👨‍👩‍👧‍👦b'
    const layout = layoutReplaceTextFlow(text, [{ width: 10, height: 200 }], 10,
      value => [...new Intl.Segmenter(undefined, { granularity: 'grapheme' }).segment(value)].length * 10)
    expect(layout.slots[0].lines).toEqual(['a\u0301', '👨‍👩‍👧‍👦', 'b'])
  })

  it('fits unusually tall glyph ink instead of relying only on an estimated line height', () => {
    const layout = layoutReplaceTextFlow('a\u0301\u0302', [{ width: 60, height: 16 }], 16,
      (text, size) => ({ width: text.length * size * 0.5, height: size * 2.5 }))
    expect(layout.complete).toBe(true)
    expect(layout.fontPx * layout.safeScale * 2.5).toBeLessThanOrEqual(16)
  })

  it('measures scaled glyphs at the font size that will actually be drawn', () => {
    // Font hinting need not scale linearly. Keep a fixed pixel contribution.
    const hintedMeasure = (text: string, size: number) => text.length * (size * 0.55 + 1)
    const layout = layoutReplaceTextFlow('完整译文'.repeat(20), [{ width: 40, height: 16 }], 16, hintedMeasure)
    expect(layout.complete).toBe(true)
    expect(layout.safeScale).toBeLessThan(1)
    for (const line of layout.slots[0].lines) {
      expect(hintedMeasure(line, layout.fontPx * layout.safeScale)).toBeLessThanOrEqual(40)
    }
  })

  it('fits the shortest occupied slot instead of sizing every line from the tallest', () => {
    const layout = layoutReplaceTextFlow('译\n文', [{ width: 100, height: 8 }, { width: 100, height: 40 }], 24, measure)
    expect(layout.complete).toBe(true)
    layout.slots.forEach((slot, index) => {
      expect(slot.contentHeight * layout.safeScale).toBeLessThanOrEqual([8, 40][index])
    })
  })

  it('fits even a single glyph within a narrow slot before reporting complete', () => {
    const layout = layoutReplaceTextFlow('译', [{ width: 2, height: 20 }], 16, measure)
    expect(layout.complete).toBe(true)
    expect(layout.slots[0].contentWidth * layout.safeScale).toBeLessThanOrEqual(2)
  })

  it('treats extra model line breaks as soft spacing inside a paragraph with fixed source rows', () => {
    const measure = (text: string, size: number) => ({ width: Array.from(text).length * size, height: size })
    const slots = [{ width: 100, height: 24, maxLines: 1 }, { width: 100, height: 24, maxLines: 1 }]
    for (const text of ['第一句。\n\n第二句。', '第一句。\n第二句。\n第三句。']) {
      const result = layoutReplaceTextFlow(text, slots, 16, measure)
      expect(result.complete).toBe(true)
      expect(result.fontPx * result.safeScale).toBeGreaterThanOrEqual(14)
      expect(result.slots.flatMap(slot => slot.lines).join('').replace(/\s/g, '')).toBe(text.replace(/\s/g, ''))
      expect(result.slots.every(slot => slot.lines.length <= 1)).toBe(true)
    }
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
