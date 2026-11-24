/** Estimate tokens: ASCII is roughly 4 chars/token; CJK and other non-ASCII chars count as 1. */
export function estimateTokens(text: string): number {
  let ascii = 0
  for (let i = 0; i < text.length; i++) {
    if (text.charCodeAt(i) < 128) ascii++
  }
  const nonAscii = text.length - ascii
  return Math.ceil(ascii / 4 + nonAscii)
}

export function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}m`
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`
  return `${n}`
}

/** Chat 用量以大写 K / M 缩写。 */
export function formatTokensK(n: number): string {
  return formatTokens(n).toUpperCase()
}

/**
 * 窄处用量缩写。满 1000 收成 k、满 1_000_000 收成 m；一位小数，整数档去掉 `.0`
 * （1000 → `1k`，不是 `1.0k`）。
 */
export function formatTokensCompact(n: number): string {
  const value = Number.isFinite(n) ? Math.max(0, n) : 0
  if (value < 1000) return `${Math.round(value)}`
  const [divisor, suffix] = value >= 1_000_000 ? [1_000_000, 'm'] : [1000, 'k']
  return `${(value / divisor).toFixed(1).replace(/\.0$/, '')}${suffix}`
}
