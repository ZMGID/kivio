export type TextBounds = { width: number; height: number }

export type TextMeasure = (text: string, fontPx: number) => number

export type ReplaceTextFlowSlot = TextBounds

export type ReplaceTextFlowSlotLayout = {
  lines: string[]
  contentWidth: number
  contentHeight: number
}

export type ReplaceTextFlowLayout = {
  fontPx: number
  lineHeight: number
  safeScale: number
  slots: ReplaceTextFlowSlotLayout[]
  complete: boolean
}

export type ReplaceRegionKind = 'cell' | 'line' | 'paragraph' | 'heading'

export function replaceTextVerticalOffset(
  kind: ReplaceRegionKind,
  availableHeight: number,
  contentHeight: number,
): number {
  if (kind === 'paragraph') return 0
  return Math.max(0, (availableHeight - contentHeight) / 2)
}

const CJK = /[\u2e80-\u9fff\uf900-\ufaff\u3040-\u30ff\uac00-\ud7af]/

export function tokenizeReplaceText(text: string): string[] {
  const tokens: string[] = []
  let latin = ''
  const flushLatin = () => {
    if (latin) tokens.push(latin)
    latin = ''
  }
  for (const char of text) {
    if (char === '\n') {
      flushLatin()
      tokens.push('\n')
    } else if (CJK.test(char)) {
      flushLatin()
      tokens.push(char)
    } else if (/\s/.test(char)) {
      flushLatin()
      tokens.push(char)
    } else {
      latin += char
    }
  }
  flushLatin()
  return tokens
}

function takeReplaceFlowLine(
  tokens: string[],
  maxWidth: number,
  fontPx: number,
  measure: TextMeasure,
): string {
  let current = ''
  while (tokens.length > 0) {
    const token = tokens[0]
    if (token === '\n') {
      tokens.shift()
      break
    }
    if (!current && /^\s+$/.test(token)) {
      tokens.shift()
      continue
    }
    const candidate = current + token
    if (!current || measure(candidate, fontPx) <= maxWidth) {
      if (measure(candidate, fontPx) <= maxWidth) {
        current = candidate
        tokens.shift()
        continue
      }
    }
    if (current) break

    let prefix = ''
    let consumed = 0
    for (const char of token) {
      const next = prefix + char
      if (prefix && measure(next, fontPx) > maxWidth) break
      prefix = next
      consumed += char.length
    }
    current = prefix
    const remainder = token.slice(consumed)
    if (remainder) tokens[0] = remainder
    else tokens.shift()
    break
  }
  return current.trimEnd()
}

function evaluateReplaceTextFlow(
  text: string,
  slots: ReplaceTextFlowSlot[],
  fontPx: number,
  safeScale: number,
  measure: TextMeasure,
): ReplaceTextFlowLayout {
  const tokens = tokenizeReplaceText(text)
  const lineHeight = fontPx * 1.18
  const layouts = slots.map(slot => {
    const virtualWidth = Math.max(1, slot.width / safeScale)
    const virtualHeight = Math.max(1, slot.height / safeScale)
    const lineCount = Math.max(1, Math.floor(virtualHeight / lineHeight))
    const lines: string[] = []
    for (let index = 0; index < lineCount && tokens.length > 0; index += 1) {
      lines.push(takeReplaceFlowLine(tokens, virtualWidth, fontPx, measure))
    }
    return {
      lines,
      contentWidth: Math.max(0, ...lines.map(line => measure(line, fontPx))),
      contentHeight: lines.length * lineHeight,
    }
  })
  return {
    fontPx,
    lineHeight,
    safeScale,
    slots: layouts,
    complete: tokens.length === 0,
  }
}

/**
 * Flow one complete translation through independent source slots. Translation
 * grouping therefore provides context without replacing several source lines
 * with one tall, vertically-centred render rectangle.
 */
export function layoutReplaceTextFlow(
  text: string,
  slots: ReplaceTextFlowSlot[],
  sourceFontPx: number,
  measure: TextMeasure,
  preferredMinPx = 7,
): ReplaceTextFlowLayout {
  if (slots.length === 0) {
    return { fontPx: preferredMinPx, lineHeight: preferredMinPx * 1.18, safeScale: 1, slots: [], complete: text.length === 0 }
  }
  const tallest = Math.max(...slots.map(slot => slot.height))
  const maxFont = Math.max(preferredMinPx, Math.min(sourceFontPx || 16, tallest * 0.82, 48))
  let low = preferredMinPx
  let high = maxFont
  let best: ReplaceTextFlowLayout | null = null
  for (let index = 0; index < 10; index += 1) {
    const fontPx = (low + high) / 2
    const candidate = evaluateReplaceTextFlow(text, slots, fontPx, 1, measure)
    if (candidate.complete) {
      best = candidate
      low = fontPx
    } else {
      high = fontPx
    }
  }
  if (best) return best

  let fittingScale = 1
  let scaled = evaluateReplaceTextFlow(text, slots, preferredMinPx, fittingScale, measure)
  while (!scaled.complete && fittingScale > 0.0001) {
    fittingScale /= 2
    scaled = evaluateReplaceTextFlow(text, slots, preferredMinPx, fittingScale, measure)
  }
  let scaleLow = fittingScale
  let scaleHigh = Math.min(1, fittingScale * 2)
  let scaledBest = scaled
  for (let index = 0; index < 12; index += 1) {
    const scale = (scaleLow + scaleHigh) / 2
    const candidate = evaluateReplaceTextFlow(text, slots, preferredMinPx, scale, measure)
    if (candidate.complete) {
      scaledBest = candidate
      scaleLow = scale
    } else {
      scaleHigh = scale
    }
  }
  return scaledBest
}
