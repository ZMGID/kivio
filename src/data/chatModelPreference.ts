/**
 * 记住用户在顶栏最后一次选的聊天模型，作为新会话/空会话草稿。
 * 以聊天界面的选择为准，不再单独设「Chat 默认模型」。仅前端偏好，存 localStorage。
 */

export const LAST_MODEL_KEY = 'kivio.chat.lastModel'

export type ChatModelBinding = {
  providerId: string
  model: string
}

export function loadLastModel(): ChatModelBinding | null {
  try {
    if (typeof window === 'undefined') return null
    const raw = window.localStorage.getItem(LAST_MODEL_KEY)
    if (!raw) return null
    const value = JSON.parse(raw) as Partial<ChatModelBinding>
    if (
      value
      && typeof value.providerId === 'string'
      && typeof value.model === 'string'
      && value.providerId
    ) {
      return { providerId: value.providerId, model: value.model }
    }
  } catch {
    /* ignore */
  }
  return null
}

export function saveLastModel(providerId: string, model: string): void {
  try {
    if (typeof window === 'undefined' || !providerId) return
    window.localStorage.setItem(LAST_MODEL_KEY, JSON.stringify({ providerId, model }))
  } catch {
    /* ignore */
  }
}

/**
 * 聊天草稿 / 设置页「当前模型」共用同一条回落：上次选择 → 已写入的 last-used →
 * 旧字段 chatProviderId → Lens → 翻译 → 首个启用模型。
 * 每一项都必须仍在启用列表中；没有可选模型时留空，不带入历史占位值。
 */
export function resolvePreferredChatModel(input: {
  providers: Array<{ id: string; enabled?: boolean; enabledModels: string[] }>
  last: ChatModelBinding | null
  storedChat: ChatModelBinding
  legacyChat: ChatModelBinding
  lens: ChatModelBinding
  translator: ChatModelBinding
}): ChatModelBinding {
  const providers = input.providers.filter(provider => provider.enabled !== false)
  for (const binding of [input.last, input.storedChat, input.legacyChat, input.lens, input.translator]) {
    if (binding?.model && providers.some(provider =>
      provider.id === binding.providerId && provider.enabledModels.includes(binding.model),
    )) return binding
  }
  const first = providers.find(provider => provider.enabledModels.length > 0)
  return first
    ? { providerId: first.id, model: first.enabledModels[0] }
    : { providerId: '', model: '' }
}
