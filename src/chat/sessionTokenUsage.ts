import type { ChatMessage, ProviderTokenUsage } from './types'

export function sessionTokenUsage(messages: readonly ChatMessage[]): ProviderTokenUsage[] {
  const byProvider = new Map<string | null, ProviderTokenUsage>()
  for (const message of messages) {
    const usage = message.usage
    if (!usage) continue
    const id = message.provider_id ?? message.providerId ?? null
    let row = byProvider.get(id)
    if (!row) {
      row = { provider_id: id, input_less_cached: null, input_plus_cache_write: null, cached: null, output: null }
      byProvider.set(id, row)
    }
    const input = usage.input_tokens ?? usage.inputTokens
    const cached = usage.cached_input_tokens ?? usage.cachedInputTokens
    const output = usage.output_tokens ?? usage.outputTokens
    if (input != null) {
      row.input_less_cached = (row.input_less_cached ?? 0) + Math.max(0, input - (cached ?? 0))
      row.input_plus_cache_write = (row.input_plus_cache_write ?? 0) + input
        + (usage.cache_creation_input_tokens ?? usage.cacheCreationInputTokens ?? 0)
    }
    if (cached != null) row.cached = (row.cached ?? 0) + cached
    if (output != null) row.output = (row.output ?? 0) + output
  }
  return [...byProvider.values()]
}
