// Chat 工具函数
import type { ConversationListItem, ConversationGroup } from './types'

/** 是否运行在 Tauri 运行时(而非纯浏览器/SSR) */
export const isTauriRuntime = (): boolean =>
  typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

/** 用户是否偏好减少动画 */
export const prefersReducedMotion = (): boolean =>
  typeof window !== 'undefined' && window.matchMedia('(prefers-reduced-motion: reduce)').matches

/**
 * 将对话列表按时间分组
 */
export function groupConversationsByTime(
  conversations: ConversationListItem[]
): ConversationGroup[] {
  const now = Date.now() / 1000
  const oneDayAgo = now - 86400
  const sevenDaysAgo = now - 86400 * 7
  const thirtyDaysAgo = now - 86400 * 30

  const groups: ConversationGroup[] = [
    { title: '今天', conversations: [] },
    { title: '昨天', conversations: [] },
    { title: '最近 7 天', conversations: [] },
    { title: '最近 30 天', conversations: [] },
    { title: '更早', conversations: [] },
  ]

  for (const conv of conversations) {
    if (conv.updated_at >= oneDayAgo) {
      groups[0].conversations.push(conv)
    } else if (conv.updated_at >= oneDayAgo - 86400) {
      groups[1].conversations.push(conv)
    } else if (conv.updated_at >= sevenDaysAgo) {
      groups[2].conversations.push(conv)
    } else if (conv.updated_at >= thirtyDaysAgo) {
      groups[3].conversations.push(conv)
    } else {
      groups[4].conversations.push(conv)
    }
  }

  // 过滤掉空分组
  return groups.filter((g) => g.conversations.length > 0)
}

/** Empty-chat hero headline: pick one at random for each new empty conversation */
export const CHAT_EMPTY_GREETINGS = [
  'Hey — what are we doing?',
  "Let's get to it.",
  'What should we focus on?',
  'Need a hand with something?',
  "What's the goal?",
  'Where do we start?',
  'What are you trying to solve today?',
  'What should we think through together?',
  "Send it — I've got you.",
  "What's top of mind?",
] as const

export function pickRandomChatEmptyGreeting(): string {
  const index = Math.floor(Math.random() * CHAT_EMPTY_GREETINGS.length)
  return CHAT_EMPTY_GREETINGS[index] ?? CHAT_EMPTY_GREETINGS[0]
}
