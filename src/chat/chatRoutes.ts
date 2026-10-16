import { hashPath } from './browserRoute'
import {
  chatRouteKind,
  decodeChatRouteId,
  decodeConversationRouteId,
  encodeChatRouteId,
} from './routeCodec'

export { hashPath } from './browserRoute'
export { isChatSettingsPath } from './routeCodec'

/**
 * 聊天窗口的 hash 路由判定与解析。
 *
 * 纯函数，单列一个 .ts 模块：既便于直接单测，也避免从组件文件导出非组件符号
 * （会破坏 React Fast Refresh，见 settings/memoryLayers.ts 的同类处理）。
 */

export function isChatAssistantCenterPath(path: string): boolean {
  return chatRouteKind(path) === 'assistants'
}

export function isChatOnboardingRoute(path: string): boolean {
  return chatRouteKind(path) === 'onboarding'
}

export function isChatSkillCenterPath(path: string): boolean {
  return chatRouteKind(path) === 'skill'
}

/** 插件市场：`#chat/plugins`，详情页 `#chat/plugins/{id}`。 */
export function isChatPluginCenterPath(path: string): boolean {
  return chatRouteKind(path) === 'plugins'
}

/** @deprecated 对话库已迁入设置；保留判定用于把旧 `#chat/sessions` 重定向到设置 → 对话库。 */
export function isChatSessionCenterPath(path: string): boolean {
  return chatRouteKind(path) === 'sessions'
}

export function isChatAutomationsPath(path: string): boolean {
  return chatRouteKind(path) === 'automations'
}

export function isChatSchedulesPath(path: string): boolean {
  return chatRouteKind(path) === 'schedules'
}

/** `#chat/automations/{id}` 的 id；列表页返回 null。 */
export function getRouteAutomationId(): string | null {
  const path = hashPath()
  return decodeChatRouteId('chat/automations/', path)
}

export function isChatMcpCenterPath(path: string): boolean {
  return chatRouteKind(path) === 'mcp'
}

export function isChatKnowledgeCenterPath(path: string): boolean {
  return chatRouteKind(path) === 'knowledge'
}

export function isChatNotesPath(path: string): boolean {
  return chatRouteKind(path) === 'notes'
}

export function isChatMediaPath(path: string): boolean {
  return chatRouteKind(path) === 'media'
}

export function isChatArtifactsPath(path: string): boolean {
  return chatRouteKind(path) === 'artifacts'
}

/**
 * 从当前 hash 解析会话 id；非会话路由返回 null。
 * 中心页（settings / assistants / skill / mcp / notes / sessions / plugins / automations / schedules / …）一律排除。
 */
export function getRouteConversationId(): string | null {
  return decodeConversationRouteId(hashPath())
}

export function isChatPopoutRoute(path: string): boolean {
  return chatRouteKind(path) === 'popout'
}

/** 把 hash 换成目标值；已是目标值则不写（避免多余的 hashchange）。 */
export function setHash(next: string): void {
  if (window.location.hash !== next) {
    window.location.hash = next
  }
}

/** 扩展中心页导航高亮：只跟当前 view 走，设置页不算。 */
export type ChatExtensionsNavItem = 'assistants' | 'skill' | 'mcp' | 'knowledge' | 'notes' | 'tasks' | 'artifacts' | 'plugins' | 'media'

export function extensionsNavItemForView(chatView: string): ChatExtensionsNavItem | null {
  if (chatView === 'media') return 'media'
  if (chatView === 'artifacts') return 'artifacts'
  if (chatView === 'plugins') return 'plugins'
  if (chatView === 'assistants') return 'assistants'
  if (chatView === 'skill') return 'plugins'
  if (chatView === 'mcp') return 'plugins'
  if (chatView === 'knowledge') return 'knowledge'
  if (chatView === 'notes') return 'notes'
  if (chatView === 'automations' || chatView === 'schedules') return 'tasks'
  return null
}

export function conversationHash(conversationId: string | null): string {
  return conversationId ? `#${encodeChatRouteId('chat/', conversationId)}` : '#chat'
}

export function automationHash(automationId: string): string {
  return `#${encodeChatRouteId('chat/automations/', automationId)}`
}
