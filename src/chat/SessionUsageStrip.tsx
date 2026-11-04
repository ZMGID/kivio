import { useEffect, useId, useMemo, useState, type CSSProperties } from 'react'
import { api, type ConversationCost } from '../api/tauri'
import { formatTokensK } from '../utils/tokens'
import { i18n, type Lang } from '../components/i18n'
import type { ChatMessage } from './types'
import { useSubAgents } from './useSubAgents'

interface SessionUsageStripProps {
  conversationId?: string
  generating?: boolean
  messages: ChatMessage[]
  lang: Lang
  /** provider id → apiFormat；用于判断该 provider 的 input_tokens 是否已含缓存。
   *   Anthropic 的 input_tokens 是纯非缓存部分，OpenAI 系 / Gemini 的 input 含缓存。 */
  apiFormats?: Record<string, string>
  /** 会话主 provider 的 apiFormat（消息没带 provider_id / 表里没有时的兜底）。 */
  defaultApiFormat?: string
  /**
   * 覆盖「input 是否已含缓存」。外部 CLI 的用量形状跟会话挂的 Kivio provider 无关：
   * dsh / claude / pi 已从 input 里扣掉 cache（再减一次会变成 ↑0、缓存 100%）。
   * 不传则仍按 apiFormat 判断（Anthropic 不减，其余减）。
   */
  cacheIncludedInInput?: boolean
  /** 外部 CLI 会话：多走订阅额度，按 API 价估出的美元与实际花费无关，且账本不记录其调用。 */
  hideCost?: boolean
}

/** 缓存命中率：命中缓存的 token 占输入总量（新鲜 + 缓存）的比例。 */
function formatCachePercent(cached: number, input: number): string {
  const total = cached + input
  if (total <= 0) return ''
  const pct = (cached / total) * 100
  if (pct >= 10) return `${Math.round(pct)}%`
  if (pct > 0) return `${pct.toFixed(1)}%`
  return '0%'
}

const GRID_COLS = 6
const GRID_ROWS = 4

/** 按比例把 cells 格分给各部分；非零部分至少 1 格，舍入误差由最大部分吸收。 */
function allocateCells(parts: number[], cells: number): number[] {
  const sum = parts.reduce((a, b) => a + b, 0)
  if (sum <= 0) return parts.map(() => 0)
  const out = parts.map(p => (p > 0 ? Math.max(1, Math.round((p / sum) * cells)) : 0))
  const largest = out.indexOf(Math.max(...out))
  out[largest] += cells - out.reduce((a, b) => a + b, 0)
  return out
}

/**
 * 已加载消息的 provider 实报用量，以及独立于消息窗口的完整对话和子代理估算费用。
 * 输入排除缓存命中；缓存命中率以新鲜输入与缓存命中的总和为分母。
 */
export function SessionUsageStrip({
  conversationId,
  generating = false,
  messages,
  lang,
  apiFormats = {},
  defaultApiFormat = '',
  cacheIncludedInInput,
  hideCost = false,
}: SessionUsageStripProps) {
  const cardId = useId()
  const totals = useMemo(() => {
    let input = 0
    let cached = 0
    let output = 0
    let hasAny = false
    let hasInput = false
    let hasOutput = false
    let hasCached = false
    for (const message of messages) {
      const usage = message.usage
      if (!usage) continue
      const providerId = message.provider_id ?? message.providerId
      const apiFormat = (providerId && apiFormats[providerId]) || defaultApiFormat
      const rawInput = usage.input_tokens ?? usage.inputTokens
      const rawOutput = usage.output_tokens ?? usage.outputTokens
      const rawCached = usage.cached_input_tokens ?? usage.cachedInputTokens
      const rawWrite = usage.cache_creation_input_tokens ?? usage.cacheCreationInputTokens ?? 0
      if (rawInput != null) {
        // Anthropic / 外部 CLI（dsh 等）的 input 已排除缓存；OpenAI 系 input 含缓存，减掉后同口径。
        const included = cacheIncludedInInput
          ?? (apiFormat !== 'anthropic_messages')
        input += included ? Math.max(0, rawInput - (rawCached ?? 0)) : rawInput + rawWrite
        hasAny = true
        hasInput = true
      }
      if (rawOutput != null) {
        output += rawOutput
        hasAny = true
        hasOutput = true
      }
      if (rawCached != null) {
        cached += rawCached
        hasAny = true
        hasCached = true
      }
    }
    return { input, cached, output, hasAny, hasInput, hasOutput, hasCached }
  }, [apiFormats, cacheIncludedInInput, defaultApiFormat, messages])

  const [costSnapshot, setCostSnapshot] = useState<{ id: string; value: ConversationCost | null } | null>(null)
  const { agents } = useSubAgents(conversationId ?? null)
  // Reuse the existing child subscription. Progress text must not trigger cost reads.
  const childExecutions = agents.map(child => {
    const run = child.runs.at(-1)
    return `${child.id}:${run?.id}:${run?.status}`
  }).join('|')
  useEffect(() => {
    if (!conversationId || hideCost) return
    let cancelled = false
    // Refresh on reopening and completed usage/run changes, not on text deltas.
    void api.usageGetConversationCost(conversationId).then((value) => {
      if (!cancelled) setCostSnapshot({ id: conversationId, value })
    }).catch(() => {
      if (!cancelled) setCostSnapshot({ id: conversationId, value: null })
    })
    return () => { cancelled = true }
  }, [conversationId, hideCost, generating, totals.input, totals.cached, totals.output, childExecutions])
  const cost = costSnapshot?.id === conversationId ? costSnapshot?.value : null
  const amount = cost?.costUsd
  const knownCost = amount != null && Number.isFinite(amount) && amount >= 0
  const partialCost = Boolean(cost && (cost.unpricedRequests > 0 || cost.skippedRecords > 0))
  const costText = knownCost
    ? `${amount.toFixed(2)}$`
    : '—$'
  const costHint = lang === 'zh'
    ? '根据已记录请求及模型价格估算，单位为美元，并非实际账单。'
    : 'Estimated in USD from recorded requests and model prices; not an invoice.'
  const partialHint = lang === 'zh'
    ? `部分请求缺少价格或用量。${knownCost ? '仅显示已知费用。' : ''}`
    : `Some requests lack pricing or usage.${knownCost ? ' Only known costs are shown.' : ''}`

  if (!totals.hasAny && !conversationId) return null

  const t = i18n[lang]
  const cacheKnown = totals.hasInput && totals.hasCached && totals.cached + totals.input > 0
  const cachePercent = cacheKnown ? formatCachePercent(totals.cached, totals.input) : '—'
  const inputText = totals.hasInput ? formatTokensK(totals.input) : '—'
  const outputText = totals.hasOutput ? formatTokensK(totals.output) : '—'
  const [cacheCells, inputCells, outputCells] = allocateCells([
    totals.hasCached ? totals.cached : 0,
    totals.hasInput ? totals.input : 0,
    totals.hasOutput ? totals.output : 0,
  ], GRID_COLS * GRID_ROWS)
  const cells = [
    ...Array<string>(cacheCells).fill('cache'),
    ...Array<string>(inputCells).fill('input'),
    ...Array<string>(outputCells).fill('output'),
  ]
  while (cells.length < GRID_COLS * GRID_ROWS) cells.push('empty')
  // 每亮一格费用跨一个数量级：<$0.1 / <$1 / <$10 / ≥$10。
  const costLevel = knownCost && amount > 0 ? Math.min(GRID_ROWS, Math.max(1, Math.floor(Math.log10(amount)) + 3)) : 0
  const showCost = Boolean(conversationId) && !hideCost
  const shares: Array<[string, number]> = [['cache', cacheCells], ['input', inputCells], ['output', outputCells]]

  // 条本身只有像素；悬停或键盘聚焦时上方弹出信息卡，读屏通过 aria-describedby 读到同一份内容。
  return (
    <div className="chat-session-usage" data-live={generating || undefined} data-tauri-drag-region="false"
      tabIndex={0} aria-describedby={cardId}>
      {showCost && (
        <span className="chat-session-usage-cost" data-partial={partialCost || undefined} aria-hidden="true">
          {Array.from({ length: GRID_ROWS }, (_, row) => (
            <i key={row} data-on={GRID_ROWS - row <= costLevel || undefined} />
          ))}
        </span>
      )}
      <span className="chat-session-usage-grid" aria-hidden="true">
        {cells.map((kind, index) => (
          <i key={index} data-k={kind} style={{ '--c': Math.floor(index / GRID_ROWS) } as CSSProperties} />
        ))}
      </span>
      <div id={cardId} role="tooltip" className="chat-session-usage-card">
        {showCost && (
          <div className="chat-session-usage-card-cost">
            <span>{lang === 'zh' ? '估算费用' : 'Estimated cost'}</span>
            <strong><span>{costText}</span>{partialCost ? '*' : ''}</strong>
          </div>
        )}
        <span className="chat-session-usage-card-bar" aria-hidden="true">
          {shares.map(([kind, n]) => n > 0 && <i key={kind} data-k={kind} style={{ flexGrow: n }} />)}
        </span>
        <div className="chat-session-usage-card-row" aria-label={t.chatUsageCache}>
          <i data-k="cache" />{t.chatUsageCache}<span>{cachePercent}</span>
        </div>
        <div className="chat-session-usage-card-row" aria-label={t.chatUsageInput}>
          <i data-k="input" />{t.chatUsageInput}<b>↑<span>{inputText}</span></b>
        </div>
        <div className="chat-session-usage-card-row" aria-label={t.chatUsageOutput}>
          <i data-k="output" />{t.chatUsageOutput}<b>↓<span>{outputText}</span></b>
        </div>
        {showCost && <p>{costHint}{partialCost ? ` ${partialHint}` : ''}</p>}
      </div>
    </div>
  )
}
