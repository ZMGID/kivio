import { updateSettingsCached } from '../api/settingsCache'
import type { ThinkingLevel, WebSearchMode } from './types'

/**
 * 顶栏 / 输入栏「记住上次选择」的前端偏好（localStorage）。与 lastAgentRuntime /
 * data/chatModelPreference 同一口径：只是新会话与空会话草稿的默认值，不是设置里的
 * 「默认模型」权威。
 */

/** 用户在顶栏最后一次选的思考等级；不再把思考等级硬回落到 high。 */
export const LAST_THINKING_KEY = 'kivio.chat.lastThinkingLevel'

const VALID_THINKING_LEVELS: ReadonlySet<string> = new Set([
  'off',
  'low',
  'medium',
  'high',
  'xhigh',
  'max',
])

/** 网络搜索模式全局默认：选一次即成为新会话 / 未显式设置会话的默认。 */
export const LAST_WEB_SEARCH_MODE_KEY = 'kivio.chat.lastWebSearchMode'
const VALID_WEB_SEARCH_MODES: ReadonlySet<string> = new Set(['off', 'builtin', 'third_party'])

export function loadLastWebSearchMode(): WebSearchMode | undefined {
  try {
    const raw = window.localStorage.getItem(LAST_WEB_SEARCH_MODE_KEY)
    return raw && VALID_WEB_SEARCH_MODES.has(raw) ? (raw as WebSearchMode) : undefined
  } catch {
    return undefined
  }
}

export function saveLastWebSearchMode(mode: WebSearchMode): void {
  try {
    window.localStorage.setItem(LAST_WEB_SEARCH_MODE_KEY, mode)
  } catch {
    /* ignore */
  }
}

export function loadLastThinkingLevel(): ThinkingLevel | null {
  try {
    const raw = window.localStorage.getItem(LAST_THINKING_KEY)
    return raw && VALID_THINKING_LEVELS.has(raw) ? (raw as ThinkingLevel) : null
  } catch {
    return null
  }
}

export function saveLastThinkingLevel(level: ThinkingLevel | null): void {
  try {
    if (level) window.localStorage.setItem(LAST_THINKING_KEY, level)
    else window.localStorage.removeItem(LAST_THINKING_KEY)
  } catch {
    /* ignore */
  }
}

/** 把聊天里刚选的模型同步进 settings，供 Mixer / 后端回落，不当作引导里的「默认模型」。 */
export async function persistLastChatModelToSettings(providerId: string, model: string): Promise<void> {
  if (!providerId.trim()) return
  try {
    await updateSettingsCached((settings) => {
      const current = settings.defaultModels?.chat
      if (current?.providerId === providerId && current?.model === model) return settings
      return {
        ...settings,
        defaultModels: {
          ...settings.defaultModels,
          chat: { providerId, model },
        },
        chatProviderId: providerId,
        chatModel: model,
      }
    })
  } catch (err) {
    console.error('Failed to persist last chat model:', err)
  }
}
